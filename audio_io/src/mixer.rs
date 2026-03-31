//! 音频混音模块
//!
//! 为播放端提供可选的多路输入混合能力，默认保持关闭，避免影响单路播放路径。

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{interval, MissedTickBehavior};

#[derive(Debug)]
pub(crate) enum MixerCommand {
    Submit {
        source_id: usize,
        frame: Vec<f32>,
    },
}

/// 单个混音输入源
#[derive(Clone)]
pub struct AudioMixerSource {
    source_id: usize,
    command_tx: mpsc::Sender<MixerCommand>,
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
}

/// 播放端内置混音控制器
pub(crate) struct PlaybackMixer {
    enabled: bool,
    frame_size: usize,
    source_sample_rate: u32,
    command_tx: Mutex<mpsc::Sender<MixerCommand>>,
    command_rx: Mutex<Option<mpsc::Receiver<MixerCommand>>>,
    task: Mutex<Option<JoinHandle<()>>>,
    next_source_id: AtomicUsize,
}

impl PlaybackMixer {
    pub(crate) fn new(enabled: bool, frame_size: usize, source_sample_rate: u32) -> Self {
        let (command_tx, command_rx) = mpsc::channel(256);

        Self {
            enabled,
            frame_size,
            source_sample_rate,
            command_tx: Mutex::new(command_tx),
            command_rx: Mutex::new(Some(command_rx)),
            task: Mutex::new(None),
            next_source_id: AtomicUsize::new(1),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn create_source(&self) -> Result<AudioMixerSource, String> {
        if !self.enabled {
            return Err("混音功能未启用".to_string());
        }

        let source_id = self.next_source_id.fetch_add(1, Ordering::Relaxed);
        let command_tx = self
            .command_tx
            .lock()
            .map_err(|_| "获取混音器发送端失败".to_string())?
            .clone();

        Ok(AudioMixerSource {
            source_id,
            command_tx,
        })
    }

    pub(crate) fn start(&self, output_tx: broadcast::Sender<Vec<f32>>) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }

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
    let mut command_closed = false;

    loop {
        tokio::select! {
            command = command_rx.recv(), if !command_closed => {
                match command {
                    Some(MixerCommand::Submit { source_id, frame }) => {
                        pending.entry(source_id).or_default().push_back(normalize_frame(frame, frame_size));
                    }
                    None => {
                        command_closed = true;
                    }
                }
            }
            _ = ticker.tick() => {
                if let Some(frame) = mix_next_frame(&mut pending, frame_size) {
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
    frame_size: usize,
) -> Option<Vec<f32>> {
    let mut frames = Vec::new();
    let mut empty_sources = Vec::new();

    for (source_id, queue) in pending.iter_mut() {
        if let Some(frame) = queue.pop_front() {
            frames.push(frame);
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

    Some(mix_frames(&frames, frame_size))
}

fn normalize_frame(mut frame: Vec<f32>, frame_size: usize) -> Vec<f32> {
    match frame.len().cmp(&frame_size) {
        std::cmp::Ordering::Less => frame.resize(frame_size, 0.0),
        std::cmp::Ordering::Equal => {}
        std::cmp::Ordering::Greater => frame.truncate(frame_size),
    }

    frame
}

fn mix_frames(frames: &[Vec<f32>], frame_size: usize) -> Vec<f32> {
    let mut mixed = vec![0.0; frame_size];

    for frame in frames {
        let limit = frame.len().min(frame_size);
        for idx in 0..limit {
            mixed[idx] += frame[idx];
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
    use super::{mix_frames, normalize_frame};

    #[test]
    fn test_normalize_frame_pads_and_truncates() {
        let padded = normalize_frame(vec![1.0, 2.0], 4);
        assert_eq!(padded, vec![1.0, 2.0, 0.0, 0.0]);

        let truncated = normalize_frame(vec![1.0, 2.0, 3.0, 4.0], 2);
        assert_eq!(truncated, vec![1.0, 2.0]);
    }

    #[test]
    fn test_mix_frames_averages_sources() {
        let mixed = mix_frames(&[vec![1.0, 0.5], vec![0.5, -0.5]], 2);
        assert_eq!(mixed, vec![0.75, 0.0]);
    }
}