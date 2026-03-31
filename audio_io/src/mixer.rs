//! 音频混音模块
//!
//! 为播放端提供可选的多路输入混合能力，默认保持关闭，避免影响单路播放路径。
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval};

#[derive(Debug, Clone)]
pub(crate) enum MixerCommand {
    Submit { source_id: usize, frame: Vec<f32> },
    SetVolume { source_id: usize, volume: f32 },
    SetMuted { source_id: usize, muted: bool },
}

/// 单个混音输入源
#[derive(Clone)]
pub struct AudioMixerSource {
    source_id: usize,
    command_tx: mpsc::Sender<MixerCommand>,
    volume: Arc<Mutex<f32>>,
    muted: Arc<Mutex<bool>>,
}

impl AudioMixerSource {
    /// 获取源编号
    pub fn source_id(&self) -> usize {
        self.source_id
    }

    /// 异步发送一帧音频到混音器
    pub async fn send(&self, frame: Vec<f32>) -> Result<(), String> {
        self.command_tx
            .send(MixerCommand::Submit {
                source_id: self.source_id,
                frame,
            })
            .await
            .map_err(|_| "混音器已停止".to_string())
    }

    /// 尝试立即发送一帧音频到混音器
    pub fn try_send(&self, frame: Vec<f32>) -> Result<(), String> {
        self.command_tx
            .try_send(MixerCommand::Submit {
                source_id: self.source_id,
                frame,
            })
            .map_err(|e| format!("发送混音数据失败: {e}"))
    }

    /// 设置该通道的音量 (0.0-1.0)
    pub async fn set_volume(&self, volume: f32) -> Result<(), String> {
        let clamped = volume.clamp(0.0, 1.0);
        *self
            .volume
            .lock()
            .map_err(|_| "获取音量锁失败".to_string())? = clamped;
        self.command_tx
            .send(MixerCommand::SetVolume {
                source_id: self.source_id,
                volume: clamped,
            })
            .await
            .map_err(|_| "混音器已停止".to_string())
    }

    /// 尝试立即设置该通道的音量 (0.0-1.0)
    pub fn try_set_volume(&self, volume: f32) -> Result<(), String> {
        let clamped = volume.clamp(0.0, 1.0);
        *self
            .volume
            .lock()
            .map_err(|_| "获取音量锁失败".to_string())? = clamped;
        self.command_tx
            .try_send(MixerCommand::SetVolume {
                source_id: self.source_id,
                volume: clamped,
            })
            .map_err(|e| format!("设置音量失败: {e}"))
    }

    /// 获取该通道当前的音量
    pub fn get_volume(&self) -> Result<f32, String> {
        let guard = self
            .volume
            .lock()
            .map_err(|_| "获取音量锁失败".to_string())?;
        Ok(*guard)
    }

    /// 设置该通道的静音状态
    pub async fn set_muted(&self, muted: bool) -> Result<(), String> {
        *self
            .muted
            .lock()
            .map_err(|_| "获取静音锁失败".to_string())? = muted;
        self.command_tx
            .send(MixerCommand::SetMuted {
                source_id: self.source_id,
                muted,
            })
            .await
            .map_err(|_| "混音器已停止".to_string())
    }

    /// 尝试立即设置该通道的静音状态
    pub fn try_set_muted(&self, muted: bool) -> Result<(), String> {
        *self
            .muted
            .lock()
            .map_err(|_| "获取静音锁失败".to_string())? = muted;
        self.command_tx
            .try_send(MixerCommand::SetMuted {
                source_id: self.source_id,
                muted,
            })
            .map_err(|e| format!("设置静音失败: {e}"))
    }

    /// 获取该通道当前的静音状态
    pub fn is_muted(&self) -> Result<bool, String> {
        let guard = self
            .muted
            .lock()
            .map_err(|_| "获取静音锁失败".to_string())?;
        Ok(*guard)
    }
}

/// 播放端内置混音控制器
pub(crate) struct PlaybackMixer {
    frame_size: usize,
    source_sample_rate: u32,
    command_tx: Mutex<mpsc::Sender<MixerCommand>>,
    command_rx: Mutex<Option<mpsc::Receiver<MixerCommand>>>,
    task: Mutex<Option<JoinHandle<()>>>,
    next_source_id: AtomicUsize,
}

impl PlaybackMixer {
    pub(crate) fn new(frame_size: usize, source_sample_rate: u32) -> Self {
        let (command_tx, command_rx) = mpsc::channel(256);
        Self {
            frame_size,
            source_sample_rate,
            command_tx: Mutex::new(command_tx),
            command_rx: Mutex::new(Some(command_rx)),
            task: Mutex::new(None),
            next_source_id: AtomicUsize::new(1),
        }
    }

    pub(crate) fn create_source(&self) -> Result<AudioMixerSource, String> {
        let source_id = self.next_source_id.fetch_add(1, Ordering::Relaxed);
        let command_tx = self
            .command_tx
            .lock()
            .map_err(|_| "获取混音器发送端失败".to_string())?
            .clone();
        Ok(AudioMixerSource {
            source_id,
            command_tx,
            volume: Arc::new(Mutex::new(1.0)),
            muted: Arc::new(Mutex::new(false)),
        })
    }

    pub(crate) fn start(&self, output_tx: broadcast::Sender<Vec<f32>>) -> Result<(), String> {
        let mut task_guard = self
            .task
            .lock()
            .map_err(|_| "获取混音任务锁失败".to_string())?;
        if task_guard.is_some() {
            return Err("音频混音器已经在运行".to_string());
        }
        let mut command_rx_guard = self
            .command_rx
            .lock()
            .map_err(|_| "获取混音通道失败".to_string())?;
        let command_rx = command_rx_guard
            .take()
            .ok_or_else(|| "混音器通道未初始化".to_string())?;
        let frame_size = self.frame_size;
        let source_sample_rate = self.source_sample_rate;
        let handle = tokio::spawn(async move {
            run_mixer(command_rx, output_tx, frame_size, source_sample_rate).await;
        });
        *task_guard = Some(handle);
        Ok(())
    }

    pub(crate) fn stop(&self) {
        if let Ok(mut task_guard) = self.task.lock() {
            if let Some(handle) = task_guard.take() {
                handle.abort();
            }
        }
        if let Ok(mut command_rx_guard) = self.command_rx.lock() {
            if command_rx_guard.is_none() {
                let (command_tx, command_rx) = mpsc::channel(256);
                if let Ok(mut command_tx_guard) = self.command_tx.lock() {
                    *command_tx_guard = command_tx;
                }
                *command_rx_guard = Some(command_rx);
            }
        }
    }
}

impl Drop for PlaybackMixer {
    fn drop(&mut self) {
        if let Ok(mut task_guard) = self.task.lock() {
            if let Some(handle) = task_guard.take() {
                handle.abort();
            }
        }
    }
}

/// 源的状态信息
#[derive(Debug, Clone)]
struct SourceState {
    volume: f32,
    muted: bool,
}

impl Default for SourceState {
    fn default() -> Self {
        Self {
            volume: 1.0,
            muted: false,
        }
    }
}

async fn run_mixer(
    mut command_rx: mpsc::Receiver<MixerCommand>,
    output_tx: broadcast::Sender<Vec<f32>>,
    frame_size: usize,
    source_sample_rate: u32,
) {
    let tick_duration = frame_duration(frame_size, source_sample_rate);
    let mut ticker = interval(tick_duration);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut pending: HashMap<usize, VecDeque<Vec<f32>>> = HashMap::new();
    let mut source_states: HashMap<usize, SourceState> = HashMap::new();
    let mut command_closed = false;

    loop {
        tokio::select! {
            command = command_rx.recv(), if !command_closed => {
                match command {
                    Some(MixerCommand::Submit { source_id, frame }) => {
                        pending.entry(source_id).or_default().push_back(normalize_frame(frame, frame_size));
                    }
                    Some(MixerCommand::SetVolume { source_id, volume }) => {
                        source_states.entry(source_id).or_default().volume = volume.clamp(0.0, 1.0);
                    }
                    Some(MixerCommand::SetMuted { source_id, muted }) => {
                        source_states.entry(source_id).or_default().muted = muted;
                    }
                    None => {
                        command_closed = true;
                    }
                }
            }
            _ = ticker.tick() => {
                if let Some(frame) = mix_next_frame(&mut pending, &source_states, frame_size) {
                    if output_tx.send(frame).is_err() {
                        break;
                    }
                } else if command_closed {
                    break;
                }
            }
        }
        if command_closed && pending.is_empty() {
            break;
        }
    }
}

fn frame_duration(frame_size: usize, source_sample_rate: u32) -> Duration {
    let sample_rate = source_sample_rate.max(1);
    let frame_size = frame_size.max(1);
    Duration::from_secs_f64(frame_size as f64 / sample_rate as f64)
}

fn mix_next_frame(
    pending: &mut HashMap<usize, VecDeque<Vec<f32>>>,
    source_states: &HashMap<usize, SourceState>,
    frame_size: usize,
) -> Option<Vec<f32>> {
    let mut frames = Vec::new();
    let mut source_ids = Vec::new();
    let mut empty_sources = Vec::new();

    for (source_id, queue) in pending.iter_mut() {
        if let Some(frame) = queue.pop_front() {
            frames.push(frame);
            source_ids.push(*source_id);
        }
        if queue.is_empty() {
            empty_sources.push(*source_id);
        }
    }

    for source_id in empty_sources {
        pending.remove(&source_id);
    }

    if frames.is_empty() {
        return None;
    }

    Some(mix_frames(&frames, &source_ids, source_states, frame_size))
}

fn normalize_frame(mut frame: Vec<f32>, frame_size: usize) -> Vec<f32> {
    match frame.len().cmp(&frame_size) {
        std::cmp::Ordering::Less => frame.resize(frame_size, 0.0),
        std::cmp::Ordering::Equal => {}
        std::cmp::Ordering::Greater => frame.truncate(frame_size),
    }
    frame
}

fn mix_frames(
    frames: &[Vec<f32>],
    source_ids: &[usize],
    source_states: &HashMap<usize, SourceState>,
    frame_size: usize,
) -> Vec<f32> {
    let mut mixed = vec![0.0; frame_size];

    for (frame_idx, frame) in frames.iter().enumerate() {
        let source_id = source_ids.get(frame_idx).copied().unwrap_or(0);
        let state = source_states.get(&source_id).cloned().unwrap_or_default();

        // 跳过静音的源
        if state.muted {
            continue;
        }

        let limit = frame.len().min(frame_size);
        for sample_idx in 0..limit {
            mixed[sample_idx] += frame[sample_idx] * state.volume;
        }
    }

    let source_count = frames.len().max(1) as f32;
    for sample in &mut mixed {
        *sample = (*sample / source_count).clamp(-1.0, 1.0);
    }

    mixed
}

#[cfg(test)]
mod tests {
    use super::{SourceState, mix_frames, normalize_frame};
    use std::collections::HashMap;

    #[test]
    fn test_normalize_frame_pads_and_truncates() {
        let padded = normalize_frame(vec![1.0, 2.0], 4);
        assert_eq!(padded, vec![1.0, 2.0, 0.0, 0.0]);
        let truncated = normalize_frame(vec![1.0, 2.0, 3.0, 4.0], 2);
        assert_eq!(truncated, vec![1.0, 2.0]);
    }

    #[test]
    fn test_mix_frames_averages_sources() {
        let mut states = HashMap::new();
        states.insert(0, SourceState::default());
        states.insert(1, SourceState::default());

        let mixed = mix_frames(&[vec![1.0, 0.5], vec![0.5, -0.5]], &[0, 1], &states, 2);
        assert_eq!(mixed, vec![0.75, 0.0]);
    }

    #[test]
    fn test_mix_frames_with_volume() {
        let mut states = HashMap::new();
        states.insert(
            0,
            SourceState {
                volume: 0.5,
                muted: false,
            },
        );
        states.insert(
            1,
            SourceState {
                volume: 0.5,
                muted: false,
            },
        );

        let mixed = mix_frames(&[vec![1.0, 1.0], vec![1.0, 1.0]], &[0, 1], &states, 2);
        assert_eq!(mixed, vec![0.5, 0.5]);
    }

    #[test]
    fn test_mix_frames_with_mute() {
        let mut states = HashMap::new();
        states.insert(
            0,
            SourceState {
                volume: 1.0,
                muted: true,
            },
        );
        states.insert(
            1,
            SourceState {
                volume: 1.0,
                muted: false,
            },
        );

        let mixed = mix_frames(&[vec![1.0, 1.0], vec![1.0, 1.0]], &[0, 1], &states, 2);
        assert_eq!(mixed, vec![0.5, 0.5]);
    }
}
