//! 音频播放模块
//!
//! 使用 cpal 播放音频数据

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Device, Stream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tokio::sync::broadcast;

use crate::stream::build_output_stream::AudioOutputStreamBuilder;
use crate::traits::AudioPlaybackControl;

/// 播放器状态枚举
#[allow(dead_code)]
pub(crate) enum PlaybackState {
    /// 停止状态，没有活跃的音频流
    Stopped,
    /// 播放状态，包含所有必要的音频资源
    Playing {
        sender: broadcast::Sender<Vec<f32>>,
        receiver: broadcast::Receiver<Vec<f32>>,
        stream: Stream,
    },
}

/// 音频播放设置
pub struct AudioPlaybackSettings {
    /// 音频数据源采样率
    pub(crate) source_sample_rate: u32,
    /// 通道返回的每帧样本数（默认480 = 10ms@48kHz）
    pub(crate) frame_size: usize,
    /// 播放音量 (f32 bits)
    pub(crate) volume: Arc<AtomicU32>,
    /// 是否静音
    pub(crate) muted: Arc<AtomicBool>,
}

/// 音频播放器
pub struct AudioPlayback {
    /// 音频输出设备
    pub(crate) device: Device,
    /// 播放状态
    pub(crate) state: PlaybackState,
    /// 播放设置
    pub(crate) settings: AudioPlaybackSettings,
}

// ⚠️ 安全性说明：
// cpal::Stream 和 cpal::Device 在内部可能包含裸指针 (*mut ())，导致编译器无法自动推导 Send 和 Sync。
// 根据 cpal 文档和设计，Stream 和 Device 是线程安全的句柄，可以在线程间传递和共享。
// 因此，我们手动实现 Send 和 Sync 以允许 AudioPlayback 在多线程环境（如 tokio 任务）中使用。
unsafe impl Send for AudioPlayback {}
unsafe impl Sync for AudioPlayback {}

impl AudioPlayback {
    /// 创建音频播放器构建器
    ///
    /// # 示例
    /// ```rust
    /// use audio_io::AudioPlayback;
    ///
    /// let playback = AudioPlayback::builder()
    ///     .sample_rate(16000)
    ///     .frame_size(320)
    ///     .build()?;
    /// # Ok::<(), String>(())
    /// ```
    pub fn builder() -> crate::builder::AudioPlaybackBuilder {
        crate::builder::AudioPlaybackBuilder::new()
    }

    /// 获取声道数
    ///
    /// 返回音频输出设备的声道数。接收的音频数据是单声道，
    /// 但会自动复制到所有输出声道。
    pub fn channels(&self) -> u16 {
        crate::utils::default_output_config(&self.device)
            .map(|c| c.channels())
            .unwrap_or(2)
    }

    /// 获取音频数据源采样率
    pub fn source_sample_rate(&self) -> u32 {
        self.settings.source_sample_rate
    }

    /// 获取每帧样本数
    pub fn frame_size(&self) -> usize {
        self.settings.frame_size
    }
}

impl AudioPlaybackControl for AudioPlayback {
    fn list_devices(&self) -> Result<Vec<String>, String> {
        crate::utils::list_output_devices()
    }

    fn start(&mut self) -> Result<broadcast::Sender<Vec<f32>>, String> {
        // 检查当前状态
        match self.state {
            PlaybackState::Playing { .. } => {
                return Err("音频播放已经在运行".to_string());
            }
            PlaybackState::Stopped => { // 可以启动，继续执行
            }
        }

        // 创建广播通道
        let (sender, receiver) = broadcast::channel(2);

        // 创建音频流
        let stream = AudioOutputStreamBuilder::build_output_stream(
            &self.settings,
            &self.device,
            receiver.resubscribe(),
        )?;

        // 启动音频流
        stream
            .play()
            .map_err(|e| format!("启动音频流失败: {}", e))?;

        // 保存发送端用于返回
        let sender_clone = sender.clone();

        // 更新状态
        self.state = PlaybackState::Playing {
            sender,
            receiver,
            stream,
        };
        log::debug!("音频播放已启动");
        Ok(sender_clone)
    }

    /// 停止音频播放
    fn stop(&mut self) -> bool {
        if !self.is_playing() {
            return false;
        }
        // 显式 drop 旧的 state，确保 Stream 等资源得到正确释放
        drop(std::mem::replace(&mut self.state, PlaybackState::Stopped));
        log::debug!("音频播放已停止");
        return true;
    }

    /// 检查是否正在播放
    fn is_playing(&self) -> bool {
        matches!(self.state, PlaybackState::Playing { .. })
    }

    /// 获取当前使用的设备名称
    fn current_device_name(&self) -> String {
        self.device
            .name()
            .unwrap_or_else(|_| "未知设备".to_string())
    }

    /// 切换音频输出设备
    ///
    /// 此方法不会切换音频播放状态。如果音频播放正在运行，它将继续在新设备上运行；
    /// 如果音频播放未运行，则不会启动播放。
    ///
    /// # 参数
    /// - `device_name`: 要切换到的设备名称
    ///
    /// # 返回
    /// 如果切换成功返回 Ok(()), 失败返回错误信息
    ///
    /// # 注意
    /// 切换设备时会临时停止音频流（如果正在运行），然后在新设备上重新启动。
    /// 整个过程对用户来说是无缝的，不会中断音频播放的连续性。
    fn switch_device(&mut self, device_name: &str) -> Result<(), String> {
        let target_device = crate::utils::find_output_device_by_name(device_name)?;

        // 验证设备配置，确保设备可用
        let cfg = crate::utils::default_output_config(&target_device)?;
        log::info!(
            "目标音频输出设备: {}, 采样率: {}Hz, 声道数: {}",
            device_name,
            cfg.sample_rate().0,
            cfg.channels()
        );

        match &self.state {
            PlaybackState::Playing { sender, .. } => {
                // 保留现有发送端，并在其上重建流
                let sender_clone = sender.clone();
                let receiver = sender_clone.subscribe();

                let stream = AudioOutputStreamBuilder::build_output_stream(
                    &self.settings,
                    &target_device,
                    receiver.resubscribe(),
                )?;

                stream
                    .play()
                    .map_err(|e| format!("新设备无法启动音频流: {}", e))?;

                // 成功后显式 drop 旧 state，然后更新设备和状态
                let old_state = std::mem::replace(
                    &mut self.state,
                    PlaybackState::Playing {
                        sender: sender_clone,
                        receiver,
                        stream,
                    },
                );
                drop(old_state);
                self.device = target_device;
            }
            PlaybackState::Stopped => {
                // 未播放时，仅更新设备
                self.device = target_device;
            }
        }

        Ok(())
    }

    /// 设置播放音量 (0.0 - 1.0)
    fn set_volume(&mut self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        self.settings
            .volume
            .store(volume.to_bits(), Ordering::Relaxed);
    }

    /// 获取播放音量
    fn get_volume(&self) -> f32 {
        f32::from_bits(self.settings.volume.load(Ordering::Relaxed))
    }

    /// 设置静音状态
    fn set_mute(&mut self, mute: bool) {
        self.settings.muted.store(mute, Ordering::Relaxed);
    }

    /// 获取静音状态
    fn is_muted(&self) -> bool {
        self.settings.muted.load(Ordering::Relaxed)
    }
}

impl Drop for AudioPlayback {
    fn drop(&mut self) {
        self.stop();
    }
}
