//! 音频播放器构建器

use crate::AudioPlayback;
use cpal::traits::{DeviceTrait, HostTrait};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32};

/// 音频播放器构建器
pub struct AudioPlaybackBuilder {
    source_sample_rate: u32,
    frame_size: usize,
    volume: f32,
    muted: bool,
}

impl AudioPlaybackBuilder {
    /// 创建新的构建器
    pub fn new() -> Self {
        Self {
            source_sample_rate: 48000,
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
        self.source_sample_rate = sample_rate;
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

    /// 设置播放音量 (0.0 - 1.0)
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

    /// 构建音频播放器
    pub fn build(self) -> Result<AudioPlayback, String> {
        // 获取默认输出设备
        let device = crate::utils::get_host()
            .default_output_device()
            .ok_or_else(|| "未找到默认音频输出设备".to_string())?;

        log::debug!("使用音频输出设备: {}", device.name().unwrap_or_default());

        // 获取默认配置
        let config = crate::utils::default_output_config(&device)?;

        log::info!(
            "音频播放配置: 采样率={}Hz, 声道数={}, 缓冲区大小={:?}, 格式={:?}",
            config.sample_rate().0,
            config.channels(),
            config.buffer_size(),
            config.sample_format()
        );

        let actual_sample_rate = config.sample_rate().0;
        if actual_sample_rate != self.source_sample_rate {
            log::warn!(
                "设备不支持采样率 {}Hz，使用设备默认 {}Hz（自动重采样）",
                self.source_sample_rate,
                actual_sample_rate
            );
        }

        Ok(AudioPlayback {
            device,
            state: crate::playback::PlaybackState::Stopped,
            settings: crate::playback::AudioPlaybackSettings {
                source_sample_rate: self.source_sample_rate,
                frame_size: self.frame_size,
                volume: Arc::new(AtomicU32::new(self.volume.to_bits())),
                muted: Arc::new(AtomicBool::new(self.muted)),
            },
        })
    }
}

impl Default for AudioPlaybackBuilder {
    fn default() -> Self {
        Self::new()
    }
}
