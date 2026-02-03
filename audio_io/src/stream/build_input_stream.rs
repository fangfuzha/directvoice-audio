use crate::capture::AudioCaptureSettings;
use crate::utils::converter::AudioSampleConverter;
use crate::utils::resampler::{create_resampler, resample_audio_data};
use cpal::Device;
use cpal::SampleFormat;
use cpal::Stream;
use cpal::traits::DeviceTrait;
use ringbuf::{HeapProd, HeapRb, traits::*};
use rubato::SincFixedIn;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{Notify, mpsc};

/// 音频采集环形缓冲区容量 (约 20 ms 48kHz 单声道音频)
const BUFFER_CAPACITY: usize = 960;

/// 音频采集缓冲区管理器
struct AudioInputBuffer {
    producer: HeapProd<f32>,
    notifier: Arc<Notify>,
    handle: tokio::task::JoinHandle<()>,
}

impl AudioInputBuffer {
    /// 创建新的音频采集缓冲区
    fn new(
        sender: mpsc::Sender<Vec<f32>>,
        resampler: Option<Arc<Mutex<SincFixedIn<f32>>>>,
        volume: Arc<AtomicU32>,
        muted: Arc<AtomicBool>,
        frame_size: usize,
    ) -> Self {
        let rb = HeapRb::<f32>::new(BUFFER_CAPACITY);
        let (producer, mut consumer) = rb.split();

        // 创建通知器用于信号传递
        let notifier = Arc::new(Notify::new());
        let notifier_for_task = notifier.clone();

        // 启动异步任务从环形缓冲区读取数据并处理
        let handle = tokio::spawn(async move {
            // 可重用的缓冲区，并预分配容量
            let mut buffer = Vec::with_capacity(BUFFER_CAPACITY);
            // 用于聚合帧的输出缓冲区
            let mut output_buffer = Vec::with_capacity(frame_size);

            loop {
                // 等待唤醒信号
                notifier_for_task.notified().await;

                // 获取缓冲区的可用数据长度
                let available = consumer.occupied_len();
                if available == 0 {
                    continue;
                }

                // 读取可用数据
                buffer.resize(available, 0.0f32);
                let read_count = consumer.pop_slice(&mut buffer);
                buffer.truncate(read_count);

                // 重采样（如果需要）
                let mut final_data = resample_audio_data(&resampler, &buffer);

                // 应用音量和静音
                let is_muted = muted.load(Ordering::Relaxed);
                if is_muted {
                    for sample in final_data.iter_mut() {
                        *sample = 0.0;
                    }
                } else {
                    let vol = f32::from_bits(volume.load(Ordering::Relaxed));
                    if (vol - 1.0).abs() > f32::EPSILON {
                        for sample in final_data.iter_mut() {
                            *sample *= vol;
                        }
                    }
                }

                // 将处理后的数据聚合到输出缓冲区
                output_buffer.extend_from_slice(&final_data);

                // 当聚合数据达到目标帧大小时，发送一帧
                while output_buffer.len() >= frame_size {
                    let frame: Vec<f32> = output_buffer.drain(..frame_size).collect();
                    match sender.try_send(frame) {
                        Ok(_) => {}
                        Err(mpsc::error::TrySendError::Full(_frame)) => {
                            log::warn!("音频采集通道已满，丢弃 {} 个样本", _frame.len());
                            break;
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            log::warn!("音频采集接受通道已关闭，停止任务");
                            return;
                        }
                    }
                }
            }
        });

        Self {
            producer,
            notifier,
            handle,
        }
    }

    /// 获取生产者引用
    fn producer_mut(&mut self) -> &mut HeapProd<f32> {
        &mut self.producer
    }
}

impl Drop for AudioInputBuffer {
    fn drop(&mut self) {
        log::debug!("音频采集缓冲区已销毁，中止关联的异步任务");
        self.handle.abort();
    }
}

/// 音频流构建器
pub(crate) struct AudioCaptureStreamBuilder;

impl AudioCaptureStreamBuilder {
    /// 构建输入音频流
    pub(crate) fn build_input_stream(
        settings: &AudioCaptureSettings,
        device: &Device,
        sender: mpsc::Sender<Vec<f32>>,
    ) -> Result<Stream, String> {
        let config = crate::utils::default_input_config(device)?;
        let sample_format = config.sample_format();
        let channels = config.channels();
        let actual_rate = config.sample_rate().0;
        let target_rate = settings.target_sample_rate;
        let frame_size = settings.frame_size;

        log::info!(
            "构建 {:?} 格式音频输入流: 采样声道数={},实际采样率={}Hz,目标采样率={}Hz,帧大小={}样本",
            sample_format,
            channels,
            actual_rate,
            target_rate,
            frame_size
        );

        // 创建重采样器（如果需要）
        let resampler = create_resampler(actual_rate, target_rate)?;

        // 创建音频采集缓冲区
        let input_buffer = AudioInputBuffer::new(
            sender,
            resampler,
            settings.volume.clone(),
            settings.muted.clone(),
            frame_size,
        );

        // 根据支持的采样格式创建流
        let stream = match sample_format {
            SampleFormat::F32 => Self::build_stream_internal::<f32>(device, config, input_buffer)?,
            SampleFormat::I16 => Self::build_stream_internal::<i16>(device, config, input_buffer)?,
            SampleFormat::U16 => Self::build_stream_internal::<u16>(device, config, input_buffer)?,
            _ => return Err("不支持的音频格式".to_string()),
        };

        Ok(stream)
    }

    /// 内部泛型函数，用于构建不同采样格式的音频流
    fn build_stream_internal<T>(
        device: &Device,
        config: cpal::SupportedStreamConfig,
        mut input_buffer: AudioInputBuffer,
    ) -> Result<Stream, String>
    where
        T: cpal::SizedSample + 'static,
        [T]: AudioSampleConverter,
    {
        let config: cpal::StreamConfig = config.into();
        let channels = config.channels as usize;

        let stream = device
            .build_input_stream(
                &config,
                move |data: &[T], _: &cpal::InputCallbackInfo| {
                    // 将音频数据转换为单声道 f32 格式
                    let mono_data = data.convert_to_mono_f32(channels);

                    // 写入环形缓冲区
                    let producer = input_buffer.producer_mut();
                    let mut written = 0;
                    while written < mono_data.len() {
                        let n = producer.push_slice(&mono_data[written..]);
                        if n == 0 {
                            log::warn!(
                                "音频采集缓冲区已满，丢弃 {} 个样本数据",
                                mono_data.len() - written
                            );
                            break;
                        }
                        written += n;
                    }

                    // 有数据写入时立即唤醒异步任务
                    if written > 0 {
                        input_buffer.notifier.notify_one();
                    }
                },
                |e| {
                    log::error!("音频流错误: {e}");
                },
                None,
            )
            .map_err(|e| format!("创建音频流失败: {e}"))?;
        Ok(stream)
    }
}
