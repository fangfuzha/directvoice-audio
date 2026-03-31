//! 音频采集器构建器
use crate::AudioCapture;
use cpal::traits::{DeviceTrait, HostTrait};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32};
/// 音频采集器构建器
pub struct AudioCaptureBuilder {
    target_sample_rate: u32,
    frame_size: usize,
    volume: f32,
    muted: bool,
}
impl AudioCaptureBuilder {
    /// 创建新的构建器
    pub fn new() -> Self {
        Self {
            target_sample_rate: 48000,
            frame_size: 480,
            volume: 1.0,
            muted: false,
        }
    }
    /// 设置采样率（Hz）
    ///
    /// # 参数
    /// - `sample_rate`: 期望采样率，与设备采样率不同会自动重采样
    pub fn sample_rate(mut self, sample_rate: u32) -> Self {
        self.target_sample_rate = sample_rate;
        self
    }
    /// 设置每帧样本数
    ///
    /// # 参数
    /// - `frame_size`: 每帧样本数（如 480 = 10ms@48kHz）
    pub fn frame_size(mut self, frame_size: usize) -> Self {
        self.frame_size = frame_size;
        self
    }
    /// 设置采集音量 (0.0 - 1.0)
    ///
    /// # 参数
    /// - `volume`: 音量值，范围 0.0 到 1.0
    pub fn volume(mut self, volume: f32) -> Self {
        self.volume = volume.clamp(0.0, 1.0);
        self
    }
    /// 设置静音状态
    ///
    /// # 参数
    /// - `muted`: 是否静音
    pub fn mute(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }
    /// 构建音频采集器
    pub fn build(self) -> Result<AudioCapture, String> {
        // 获取音频采集设备
        let device = crate::utils::get_host()
            .default_input_device()
            .ok_or_else(|| "未找到默认音频输入设备".to_string())?;
        log::debug!("默认音频采集设备: {}", device.name().unwrap_or_default());
        // 获取设备支持的配置
        let config = crate::utils::default_input_config(&device)?;
        log::debug!(
            "默认音频采集配置: 通道数={},目标采样率={}Hz, 缓冲区大小={:?}, 采样格式={:?}",
            config.channels(),
            config.sample_rate().0,
            config.buffer_size(),
            config.sample_format()
        );
        let actual_sample_rate = config.sample_rate().0;
        if actual_sample_rate != self.target_sample_rate {
            log::warn!(
                "设备不支持采样率 {}Hz，使用设备默认 {}Hz（自动重采样）",
                self.target_sample_rate,
                actual_sample_rate
            );
        }
        Ok(AudioCapture {
            device,
            state: crate::capture::CaptureState::Idle,
            settings: crate::capture::AudioCaptureSettings {
                target_sample_rate: self.target_sample_rate,
                frame_size: self.frame_size,
                volume: Arc::new(AtomicU32::new(self.volume.to_bits())),
                muted: Arc::new(AtomicBool::new(self.muted)),
            },
        })
    }
}
impl Default for AudioCaptureBuilder {
    fn default() -> Self {
        Self::new()
    }
}
