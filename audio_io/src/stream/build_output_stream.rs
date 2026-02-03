//! 输出音频流构建实现
//!
//! 提供不同音频格式的输出流构建功能，支持重采样

use super::super::playback::AudioPlaybackSettings;
use crate::utils::converter::AudioOutputConverter;
use crate::utils::resampler::{create_resampler, resample_audio_data};
use cpal::SampleFormat;
use cpal::Stream;
use cpal::traits::DeviceTrait;
use ringbuf::{HeapCons, HeapRb, traits::*};
use rubato::SincFixedIn;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast::Receiver;

/// 音频输出缓冲区管理器
pub(crate) struct AudioOutputBuffer {
    consumer: HeapCons<f32>,
    handle: tokio::task::JoinHandle<()>,
}

impl AudioOutputBuffer {
    /// 创建新的音频输出缓冲区
    fn new(
        mut receiver: Receiver<Vec<f32>>,
        resampler: Option<Arc<Mutex<SincFixedIn<f32>>>>,
        buffer_capacity: usize,
    ) -> Self {
        let rb = HeapRb::<f32>::new(buffer_capacity);
        let (mut producer, consumer) = rb.split();

        // 启动异步任务接收音频数据到环形缓冲区
        let handle = tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(data) => {
                        // 重采样（如果需要）
                        let final_data = resample_audio_data(&resampler, &data);

                        // 尝试将数据写入环形缓冲区
                        let mut written = 0;
                        while written < final_data.len() {
                            let n = producer.push_slice(&final_data[written..]);
                            if n == 0 {
                                // 缓冲区已满，可能存在高延迟
                                // 策略：丢弃当前包中剩余的数据，防止阻塞异步运行时
                                log::warn!("音频输出缓冲区已满，丢弃部分数据以降低延迟");
                                break;
                            }
                            written += n;
                        }
                    }
                    Err(_) => {
                        // 接收器关闭，退出任务
                        log::debug!("输出缓冲区接受数据失败，退出任务");
                        break;
                    }
                }
            }
        });

        Self {
            consumer,
            handle,
        }
    }

    /// 获取消费者引用
    fn consumer_mut(&mut self) -> &mut HeapCons<f32> {
        &mut self.consumer
    }
}

impl Drop for AudioOutputBuffer {
    fn drop(&mut self) {
        log::debug!("音频输出缓冲区已销毁，中止关联的异步任务");
        self.handle.abort();
    }
}

/// 音频输出流构建器
pub(crate) struct AudioOutputStreamBuilder;

impl AudioOutputStreamBuilder {
    /// 构建输出音频流
    pub(crate) fn build_output_stream(
        settings: &AudioPlaybackSettings,
        device: &cpal::Device,
        receiver: Receiver<Vec<f32>>,
    ) -> Result<Stream, String> {
        let cfg = crate::utils::default_output_config(device)?;
        let actual_rate = cfg.sample_rate().0;
        let source_rate = settings.source_sample_rate;

        log::info!(
            "构建 {:?} 格式音频输出流: 输出声道数={},数据源采样率={}Hz, 实际采样率={}Hz",
            cfg.sample_format(),
            cfg.channels(),
            source_rate,
            actual_rate
        );

        // 创建重采样器（如果需要）
        let resampler = create_resampler(source_rate, actual_rate)?;
        // 创建音频输出缓冲区(容量为帧大小的2倍)
        let buffer_capacity = settings.frame_size * 2;
        let output_buffer = AudioOutputBuffer::new(receiver, resampler, buffer_capacity);

        // 根据输出设备支持格式创建流
        let stream = match cfg.sample_format() {
            SampleFormat::F32 => {
                Self::build_stream_generic::<f32>(settings, device, cfg, output_buffer)?
            }
            SampleFormat::I16 => {
                Self::build_stream_generic::<i16>(settings, device, cfg, output_buffer)?
            }
            SampleFormat::U16 => {
                Self::build_stream_generic::<u16>(settings, device, cfg, output_buffer)?
            }
            _ => return Err("不支持的音频格式".to_string()),
        };

        Ok(stream)
    }

    /// 通用音频流构建函数
    fn build_stream_generic<T>(
        settings: &AudioPlaybackSettings,
        device: &cpal::Device,
        config: cpal::SupportedStreamConfig,
        mut output_buffer: AudioOutputBuffer,
    ) -> Result<Stream, String>
    where
        T: cpal::Sample + cpal::SizedSample + Copy + Send + 'static,
        [T]: AudioOutputConverter,
    {
        let muted = settings.muted.clone();
        let volume = settings.volume.clone();
        let channels = config.channels() as usize;

        let stream = device
            .build_output_stream(
                &config.clone().into(),
                move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                    let is_muted = muted.load(Ordering::Relaxed);
                    let vol = f32::from_bits(volume.load(Ordering::Relaxed));

                    let consumer = output_buffer.consumer_mut();

                    if consumer.is_empty() {
                        // 没有数据，填充静音
                        data.fill_silence();
                    } else if is_muted {
                        // 静音，填充静音并消耗缓冲区
                        data.fill_silence();
                        let to_pop = (data.len() / channels).min(consumer.occupied_len());
                        consumer.skip(to_pop);
                    } else {
                        // 零拷贝读取：直接从环形缓冲区的内部切片转换并写入
                        let (s1, s2) = consumer.as_slices();
                        let mut offset = 0;
                        let mut total_consumed = 0;

                        // 处理第一段切片
                        let consumed1 = data[offset..].write_samples(s1, vol, channels);
                        offset += consumed1 * channels;
                        total_consumed += consumed1;

                        // 处理第二段切片（环形缓冲区可能回绕）
                        if offset < data.len() && !s2.is_empty() {
                            let consumed2 = data[offset..].write_samples(s2, vol, channels);
                            offset += consumed2 * channels;
                            total_consumed += consumed2;
                        }

                        // 标记已消耗的数据
                        consumer.skip(total_consumed);

                        // 如果数据不足以填满硬件缓冲区，填充剩余部分为静音
                        if offset < data.len() {
                            data[offset..].fill_silence();
                        }
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
