//! 音频会话处理流水线。
//!
//! 这里只放处理阶段的配置和纯音频处理逻辑，按固定顺序执行：AEC -> NS。
//!
//! AEC 使用的是播放侧即将输出的 far-end 参考 tap，避免把采集帧本身误当成参考源。
use log::debug;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::broadcast;
/// 会话级处理流水线配置。
#[derive(Clone, Debug)]
pub struct AudioProcessingPipelineConfig {
    /// AEC 回声消除配置。
    pub aec: AecConfig,
    /// NS 降噪配置。
    pub ns: NsConfig,
}
impl Default for AudioProcessingPipelineConfig {
    fn default() -> Self {
        Self {
            aec: AecConfig::default(),
            ns: NsConfig::default(),
        }
    }
}
/// AEC 回声消除配置。
#[derive(Clone, Debug)]
pub struct AecConfig {
    /// 是否启用 AEC。
    pub enabled: bool,
    /// 回声消除尾长，单位毫秒。
    pub tail_ms: u32,
    /// 参考信号延迟，单位毫秒。
    pub delay_ms: u32,
    /// 自适应强度。
    pub adaptation_rate: f32,
    /// 双讲保护。
    pub double_talk_protection: bool,
}
impl Default for AecConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            tail_ms: 120,
            delay_ms: 0,
            adaptation_rate: 0.55,
            double_talk_protection: true,
        }
    }
}
/// NS 降噪配置。
#[derive(Clone, Debug)]
pub struct NsConfig {
    /// 是否启用 NS。
    pub enabled: bool,
    /// 降噪强度，范围通常在 0.0 - 1.0。
    pub strength: f32,
    /// 噪声底值，越大越激进。
    pub noise_floor: f32,
    /// 人声保留权重，避免过度削弱语音。
    pub speech_preserve: f32,
}
impl Default for NsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            strength: 0.5,
            noise_floor: 0.02,
            speech_preserve: 0.9,
        }
    }
}
pub(crate) async fn process_audio_bridge(
    mut capture_receiver: tokio::sync::mpsc::Receiver<Vec<f32>>,
    playback_sender: broadcast::Sender<Vec<f32>>,
    mut playback_reference_receiver: broadcast::Receiver<Vec<f32>>,
    raw_tx: Option<broadcast::Sender<Vec<f32>>>,
    processed_tx: Option<broadcast::Sender<Vec<f32>>>,
    pipeline: Arc<Mutex<AudioProcessingPipelineConfig>>,
    sample_rate: u32,
    auto_loopback_enabled: Arc<AtomicBool>,
) {
    let mut far_end_reference_history: VecDeque<f32> = VecDeque::new();
    loop {
        tokio::select! {
            maybe_frame = capture_receiver.recv() => {
                let Some(frame) = maybe_frame else {
                    break;
                };
                let pipeline = current_pipeline(&pipeline);
                if let Some(sender) = raw_tx.as_ref() {
                    let _ = sender.send(frame.clone());
                }
                let mut processed_frame = frame;
                let reference_frame = build_reference_frame(
                    &far_end_reference_history,
                    processed_frame.len(),
                    samples_from_ms(pipeline.aec.delay_ms, sample_rate),
                );
                apply_aec(
                    &mut processed_frame,
                    reference_frame.as_deref(),
                    &pipeline.aec,
                );
                apply_ns(&mut processed_frame, &pipeline.ns);
                if let Some(sender) = processed_tx.as_ref() {
                    let _ = sender.send(processed_frame.clone());
                }
                if auto_loopback_enabled.load(Ordering::Relaxed) {
                    let _ = playback_sender.send(processed_frame);
                }
            }
            maybe_reference = playback_reference_receiver.recv() => {
                match maybe_reference {
                    Ok(frame) => {
                        let pipeline = current_pipeline(&pipeline);
                        append_reference_history(
                            &mut far_end_reference_history,
                            &frame,
                            sample_rate,
                            &pipeline,
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                }
            }
        }
    }
}
fn current_pipeline(
    pipeline: &Arc<Mutex<AudioProcessingPipelineConfig>>,
) -> AudioProcessingPipelineConfig {
    pipeline
        .lock()
        .map(|config| config.clone())
        .unwrap_or_default()
}
#[cfg(test)]
fn process_frame_order_test_only(
    frame: &mut [f32],
    sample_rate: u32,
    pipeline: &AudioProcessingPipelineConfig,
    reference_history: &[f32],
) {
    let reference = build_reference_from_slice(
        reference_history,
        frame.len(),
        samples_from_ms(pipeline.aec.delay_ms, sample_rate),
    );
    apply_aec(frame, reference.as_deref(), &pipeline.aec);
    apply_ns(frame, &pipeline.ns);
}
fn append_reference_history(
    history: &mut VecDeque<f32>,
    frame: &[f32],
    sample_rate: u32,
    pipeline: &AudioProcessingPipelineConfig,
) {
    let max_history = reference_capacity(sample_rate, pipeline, frame.len());
    history.extend(frame.iter().copied());
    while history.len() > max_history {
        let _ = history.pop_front();
    }
}
fn build_reference_frame(
    history: &VecDeque<f32>,
    frame_len: usize,
    delay_samples: usize,
) -> Option<Vec<f32>> {
    let required_len = frame_len.saturating_add(delay_samples);
    if history.len() < required_len {
        debug!(
            "AEC参考帧历史数据不足: 需要 {} 样本, 当前仅有 {} 样本, 使用静默帧",
            required_len,
            history.len()
        );
        return None;
    }
    let end = history.len().saturating_sub(delay_samples);
    let start = end.saturating_sub(frame_len);
    let mut reference = Vec::with_capacity(end.saturating_sub(start));
    reference.extend(
        history
            .iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .copied(),
    );
    Some(reference)
}
#[cfg(test)]
fn build_reference_from_slice(
    history: &[f32],
    frame_len: usize,
    delay_samples: usize,
) -> Option<Vec<f32>> {
    if history.len() < frame_len.saturating_add(delay_samples) {
        return None;
    }
    let end = history.len().saturating_sub(delay_samples);
    let start = end.saturating_sub(frame_len);
    Some(history[start..end].to_vec())
}
fn apply_aec(frame: &mut [f32], reference: Option<&[f32]>, config: &AecConfig) {
    if !config.enabled {
        return;
    }
    let reference: Vec<f32> = match reference {
        Some(reference) if !reference.is_empty() => reference.to_vec(),
        _ => {
            vec![0.0f32; frame.len()]
        }
    };
    let frame_rms = rms(frame);
    let reference_rms = rms(&reference);
    let mut echo_strength = config.adaptation_rate.clamp(0.0, 1.0) * 0.7;
    if config.double_talk_protection && frame_rms > reference_rms * 1.2 {
        echo_strength *= 0.5;
    }
    let overlap = frame.len().min(reference.len());
    for index in 0..overlap {
        frame[index] -= reference[index] * echo_strength;
    }
}
fn apply_ns(frame: &mut [f32], config: &NsConfig) {
    if !config.enabled {
        return;
    }
    let noise_floor = config.noise_floor.max(0.000001);
    let strength = config.strength.clamp(0.0, 1.0);
    let preserve = config.speech_preserve.clamp(0.0, 1.0);
    for sample in frame.iter_mut() {
        let magnitude = sample.abs();
        if magnitude < noise_floor {
            *sample *= 1.0 - strength;
        } else if magnitude < noise_floor * 2.0 {
            *sample *= preserve;
        }
    }
}
fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum = samples.iter().map(|sample| sample * sample).sum::<f32>();
    (sum / samples.len() as f32).sqrt()
}
fn samples_from_ms(ms: u32, sample_rate: u32) -> usize {
    ((ms as u64 * sample_rate as u64) / 1000) as usize
}
fn reference_capacity(
    sample_rate: u32,
    pipeline: &AudioProcessingPipelineConfig,
    frame_len: usize,
) -> usize {
    let aec = &pipeline.aec;
    let delay = samples_from_ms(aec.delay_ms, sample_rate);
    let tail = samples_from_ms(aec.tail_ms, sample_rate);
    (sample_rate as usize).max(
        delay
            .saturating_add(tail)
            .saturating_add(frame_len.saturating_mul(2)),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aec_runs_before_ns() {
        let pipeline = AudioProcessingPipelineConfig {
            aec: AecConfig {
                enabled: true,
                tail_ms: 100,
                delay_ms: 0,
                adaptation_rate: 1.0,
                double_talk_protection: false,
            },
            ns: NsConfig {
                enabled: true,
                strength: 1.0,
                noise_floor: 0.1,
                speech_preserve: 0.0,
            },
        };
        let mut frame = vec![0.4_f32];
        let reference_history = vec![0.4_f32];
        process_frame_order_test_only(&mut frame, 48_000, &pipeline, &reference_history);
        assert_eq!(frame[0], 0.0);
    }
}
