//! 音频采集模块
//!
//! 使用 cpal 从麦克风采集音频数据

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Device, Stream};
use log::debug;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tokio::sync::mpsc;

use crate::stream::build_input_stream::AudioCaptureStreamBuilder;
use crate::traits::AudioCaptureControl;
use crate::utils::find_input_device_by_name;

/// 音频采集状态
pub(crate) enum CaptureState {
    /// 空闲状态
    Idle,
    /// 正在采集
    Running {
        sender: mpsc::Sender<Vec<f32>>,
        _stream: Stream,
    },
}

/// 音频采集设置
pub struct AudioCaptureSettings {
    /// 目标采样率
    pub(crate) target_sample_rate: u32,
    /// 目标帧大小
    pub(crate) frame_size: usize,
    /// 音量(0.0 - 1.0)
    pub(crate) volume: Arc<AtomicU32>,
    /// 静音
    pub(crate) muted: Arc<AtomicBool>,
}

/// 音频采集器
pub struct AudioCapture {
    /// 音频输入设备
    pub(crate) device: Device,
    /// 采集状态
    pub(crate) state: CaptureState,
    /// 采集设置
    pub(crate) settings: AudioCaptureSettings,
}

// ⚠️ 安全性说明：
// cpal::Stream 和 cpal::Device 在内部可能包含裸指针 (*mut ())，导致编译器无法自动推导 Send 和 Sync。
// 根据 cpal 文档和设计，Stream 和 Device 是线程安全的句柄，可以在线程间传递和共享。
// 因此，我们手动实现 Send 和 Sync 以允许 AudioCapture 在多线程环境（如 tokio 任务）中使用。
unsafe impl Send for AudioCapture {}
unsafe impl Sync for AudioCapture {}

impl AudioCapture {
    /// 创建音频采集器构建器
    ///
    /// # 示例
    /// ```rust
    /// use audio_io::AudioCapture;
    ///
    /// let capture = AudioCapture::builder()
    ///     .sample_rate(16000)
    ///     .frame_size(320)
    ///     .build()?;
    /// # Ok::<(), String>(())
    /// ```
    pub fn builder() -> crate::builder::AudioCaptureBuilder {
        crate::builder::AudioCaptureBuilder::new()
    }
}

impl AudioCaptureControl for AudioCapture {
    fn list_devices(&self) -> Result<Vec<String>, String> {
        crate::utils::list_input_devices()
    }

    /// 启动音频采集
    ///
    /// 返回接收音频数据的通道
    fn start(&mut self) -> Result<mpsc::Receiver<Vec<f32>>, String> {
        if let CaptureState::Running { .. } = self.state {
            return Err("音频采集已经在运行".to_string());
        }

        // 创建有界通道(容量为2帧,减少内存占用)
        let (sender, receiver) = mpsc::channel(2);

        // 创建音频流
        let stream = {
            AudioCaptureStreamBuilder::build_input_stream(
                &self.settings,
                &self.device,
                sender.clone(),
            )?
        };

        // 启动音频流
        stream
            .play()
            .map_err(|e| format!("启动音频流失败: {e:?}"))?;

        // 更新状态
        self.state = CaptureState::Running {
            sender,
            _stream: stream,
        };
        debug!("音频采集已启动");
        Ok(receiver)
    }

    /// 停止音频采集
    fn stop(&mut self) -> bool {
        if !self.is_capturing() {
            return false;
        }
        // 显式 drop 旧的 state，确保 Stream 等资源得到正确释放
        drop(std::mem::replace(&mut self.state, CaptureState::Idle));
        debug!("音频采集已停止");
        true
    }

    /// 检查是否正在采集
    fn is_capturing(&self) -> bool {
        matches!(self.state, CaptureState::Running { .. })
    }

    /// 获取当前使用的设备名称
    fn current_device_name(&self) -> String {
        self.device
            .name()
            .unwrap_or_else(|_| "未知设备".to_string())
    }

    /// 切换音频输入设备
    ///
    /// 此方法不会切换音频采集状态。如果音频采集正在运行，它将继续在新设备上运行；
    /// 如果音频采集未运行，则不会启动采集。
    ///
    /// # 参数
    /// - `device_name`: 要切换到的设备名称
    ///
    /// # 返回
    /// 如果切换成功返回 Ok(()), 失败返回错误信息
    ///
    /// # 注意
    /// 切换设备时会临时停止音频流（如果正在运行），然后在新设备上重新启动。
    /// 整个过程对用户来说是无缝的，不会中断音频采集的连续性。
    fn switch_device(&mut self, device_name: &str) -> Result<(), String> {
        let target_device = find_input_device_by_name(device_name)?;

        // 验证设备配置，确保设备可用
        let _config = crate::utils::default_input_config(&target_device)?;

        // 如果正在运行，先尝试用新设备构建流，只有成功才替换设备和流
        if let CaptureState::Running { sender, .. } = &self.state {
            let sender_clone = sender.clone();

            // 构建新流
            let new_stream = AudioCaptureStreamBuilder::build_input_stream(
                &self.settings,
                &target_device,
                sender_clone.clone(),
            )
            .and_then(|stream| {
                stream
                    .play()
                    .map_err(|e| format!("新设备无法启动音频流: {:?}", e))
                    .map(|_| stream)
            })?;

            // 新流构建成功，显式 drop 旧 state，然后更新设备和流
            let old_state = std::mem::replace(
                &mut self.state,
                CaptureState::Running {
                    sender: sender_clone,
                    _stream: new_stream,
                },
            );
            drop(old_state);
            self.device = target_device;
        } else {
            // 未运行时，直接替换设备
            self.device = target_device;
        }

        debug!("成功切换到音频输入设备: {}", device_name);
        Ok(())
    }

    /// 设置采集音量 (0.0 - 1.0)
    fn set_volume(&mut self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        self.settings
            .volume
            .store(volume.to_bits(), Ordering::Relaxed);
    }

    /// 获取采集音量 (0.0 - 1.0)
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

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.stop();
    }
}
