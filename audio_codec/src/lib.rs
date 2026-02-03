//! 音频编解码模块
//!
//! 统一封装 Opus 编码器和解码器，提供编解码功能

use audiopus::{Channels, SampleRate, coder::Decoder, coder::Encoder, coder::GenericCtl};
use std::sync::Arc;
use tokio::sync::Mutex;

pub use audiopus::Application; // 重新导出 Application 枚举

/// 将 u32 采样率转换为 Opus SampleRate
fn to_opus_sample_rate(sample_rate: u32) -> Result<SampleRate, String> {
    match sample_rate {
        8000 => Ok(SampleRate::Hz8000),
        12000 => Ok(SampleRate::Hz12000),
        16000 => Ok(SampleRate::Hz16000),
        24000 => Ok(SampleRate::Hz24000),
        48000 => Ok(SampleRate::Hz48000),
        _ => Err(format!(
            "不支持的采样率: {sample_rate}，支持的采样率: 8000, 12000, 16000, 24000, 48000"
        )),
    }
}

/// 音频编解码器构建器
pub struct AudioCodecBuilder {
    sample_rate: u32,
    frame_size: usize,
    bitrate: i32,
    application: Application,
}

impl AudioCodecBuilder {
    /// 创建新的构建器
    pub fn new() -> Self {
        Self {
            sample_rate: 48000,
            frame_size: 480,
            bitrate: 64000,
            application: Application::LowDelay,
        }
    }

    /// 设置采样率（Hz）
    ///
    /// 支持: 8000, 12000, 16000, 24000, 48000
    pub fn sample_rate(mut self, sample_rate: u32) -> Self {
        self.sample_rate = sample_rate;
        self
    }

    /// 设置帧大小（样本数）
    ///
    /// 通常为 480 (10ms @ 48kHz) 或 960 (20ms @ 48kHz)
    pub fn frame_size(mut self, frame_size: usize) -> Self {
        self.frame_size = frame_size;
        self
    }

    /// 设置比特率（bps）
    ///
    /// 影响音质和带宽，默认 64000
    pub fn bitrate(mut self, bitrate: i32) -> Self {
        self.bitrate = bitrate;
        self
    }

    /// 设置应用模式
    ///
    /// 可选: Voip, Audio, LowDelay
    pub fn application(mut self, application: Application) -> Self {
        self.application = application;
        self
    }

    /// 构建音频编解码器
    pub fn build(self) -> Result<AudioCodec, String> {
        AudioCodec::new(
            self.sample_rate,
            self.frame_size,
            self.bitrate,
            self.application,
        )
    }
}

impl Default for AudioCodecBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// 音频编解码器
pub struct AudioCodec {
    /// Opus 编码器
    encoder: Arc<Mutex<Encoder>>,
    /// Opus 解码器
    decoder: Arc<Mutex<Decoder>>,
    /// 采样率
    sample_rate: u32,
    /// 帧大小（每帧样本数）
    frame_size: usize,
}

impl AudioCodec {
    /// 创建音频编解码器构建器
    ///
    /// # 示例
    /// ```rust
    /// use audio_codec::{AudioCodec, Application};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let codec = AudioCodec::builder()
    ///     .sample_rate(48000)
    ///     .frame_size(480)
    ///     .bitrate(64000)
    ///     .application(Application::LowDelay)
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn builder() -> AudioCodecBuilder {
        AudioCodecBuilder::new()
    }

    /// 创建新的音频编解码器
    ///
    /// # 参数
    /// - `sample_rate`: 采样率（Hz），支持 8000, 12000, 16000, 24000, 48000
    /// - `frame_size`: 帧大小（样本数），通常是 960 (20ms @ 48kHz)
    /// - `bitrate`: 比特率（bps），默认 64000,影响音质和带宽
    /// - `application`: 应用模式，默认 LowDelay
    fn new(
        sample_rate: u32,
        frame_size: usize,
        bitrate: i32,
        application: Application,
    ) -> Result<Self, String> {
        // 匹配采样率
        let opus_sample_rate = to_opus_sample_rate(sample_rate)?;

        // 创建 Opus 编码器（单声道，指定编码器应用模式）
        let mut encoder = Encoder::new(opus_sample_rate, Channels::Mono, application)
            .map_err(|e| format!("创建 Opus 编码器失败: {e:?}"))?;
        // 设置编码器比特率
        encoder
            .set_bitrate(audiopus::Bitrate::BitsPerSecond(bitrate))
            .map_err(|e| format!("设置编码器比特率失败: {e:?}"))?;

        // 创建 Opus 解码器（单声道）
        let decoder = Decoder::new(opus_sample_rate, Channels::Mono)
            .map_err(|e| format!("创建 Opus 解码器失败: {:?}", e))?;

        log::info!(
            "音频编解码器已创建: 采样率={sample_rate}Hz, 帧大小={frame_size}, 比特率={bitrate}bps, 应用模式={application:?}"
        );

        Ok(Self {
            encoder: Arc::new(Mutex::new(encoder)),
            decoder: Arc::new(Mutex::new(decoder)),
            sample_rate,
            frame_size,
        })
    }

    /// 编码音频帧
    ///
    /// # 参数
    /// - `pcm_data`: PCM 音频数据（f32 格式，-1.0 到 1.0）
    ///
    /// # 返回
    /// 编码后的 Opus 数据
    pub async fn encode(&self, pcm_data: &[f32]) -> Result<Vec<u8>, String> {
        // 检查帧大小
        if pcm_data.len() != self.frame_size {
            return Err(format!(
                "音频帧大小不匹配: 期望 {}, 实际 {}",
                self.frame_size,
                pcm_data.len()
            ));
        }

        // 编码（直接使用 f32 数据）
        let encoder = self.encoder.lock().await;
        let mut output = vec![0u8; 4000]; // Opus 最大帧大小

        let len = encoder
            .encode_float(pcm_data, &mut output)
            .map_err(|e| format!("编码失败: {:?}", e))?;

        output.truncate(len);

        Ok(output)
    }

    /// 解码音频帧
    ///
    /// # 参数
    /// - `opus_data`: Opus 编码的音频数据
    ///
    /// # 返回
    /// 解码后的 PCM 数据（f32 格式，-1.0 到 1.0）
    pub async fn decode(&self, opus_data: &[u8]) -> Result<Vec<f32>, String> {
        let mut decoder = self.decoder.lock().await;

        // 创建输出缓冲区（f32 格式）
        let mut output = vec![0.0f32; self.frame_size];

        // 解码（直接输出 f32）
        let len = decoder
            .decode_float(Some(opus_data), &mut output, false)
            .map_err(|e| format!("解码失败: {:?}", e))?;

        // 截取实际解码的长度
        output.truncate(len);

        Ok(output)
    }

    /// 解码丢包帧（使用 PLC - Packet Loss Concealment）
    ///
    /// 当检测到丢包时，应该使用此方法生成补偿音频
    pub async fn decode_plc(&self) -> Result<Vec<f32>, String> {
        let mut decoder = self.decoder.lock().await;

        // 创建输出缓冲区（f32 格式）
        let mut output = vec![0.0f32; self.frame_size];

        // 使用 PLC 解码（传入 None::<&[u8]> 表示丢包）
        let len = decoder
            .decode_float(None::<&[u8]>, &mut output, false)
            .map_err(|e| format!("PLC 解码失败: {:?}", e))?;

        // 截取实际解码的长度
        output.truncate(len);

        Ok(output)
    }

    /// 批量编码音频数据
    ///
    /// 将长音频数据分割成多个帧并编码
    pub async fn encode_stream(&self, pcm_data: &[f32]) -> Result<Vec<Vec<u8>>, String> {
        let mut encoded_frames = Vec::new();

        for chunk in pcm_data.chunks(self.frame_size) {
            // 如果最后一帧不足，填充零
            if chunk.len() < self.frame_size {
                let mut padded = chunk.to_vec();
                padded.resize(self.frame_size, 0.0);
                encoded_frames.push(self.encode(&padded).await?);
            } else {
                encoded_frames.push(self.encode(chunk).await?);
            }
        }

        Ok(encoded_frames)
    }

    /// 批量解码音频数据
    pub async fn decode_stream(&self, opus_frames: &[Vec<u8>]) -> Result<Vec<f32>, String> {
        let mut pcm_data = Vec::new();

        for frame in opus_frames {
            let decoded = self.decode(frame).await?;
            pcm_data.extend(decoded);
        }

        Ok(pcm_data)
    }

    /// 获取采样率
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 获取帧大小
    pub fn frame_size(&self) -> usize {
        self.frame_size
    }

    /// 获取编码比特率
    pub async fn bitrate(&self) -> Result<i32, String> {
        let encoder = self.encoder.lock().await;
        match encoder.bitrate() {
            Ok(audiopus::Bitrate::BitsPerSecond(bps)) => Ok(bps), // 表示每秒比特数
            Ok(audiopus::Bitrate::Auto) => Ok(0),                 // 表示编码器自动选择比特率
            Ok(audiopus::Bitrate::Max) => Ok(i32::MAX), // 表示允许的最大比特率 = 数据包的最大字节数 * 8
            Err(e) => Err(format!("获取编码器比特率失败: {e:?}")),
        }
    }

    /// 设置编码比特率
    pub async fn set_bitrate(&mut self, bitrate: i32) -> Result<(), String> {
        let mut encoder = self.encoder.lock().await;
        encoder
            .set_bitrate(audiopus::Bitrate::BitsPerSecond(bitrate))
            .map_err(|e| format!("设置比特率失败: {:?}", e))?;
        log::info!("编码器比特率已更新: {bitrate}bps");
        Ok(())
    }

    /// 获取编码器应用模式
    pub async fn application(&self) -> Result<Application, String> {
        let encoder = self.encoder.lock().await;
        encoder
            .application()
            .map_err(|e| format!("获取编码器应用模式失败: {:?}", e))
    }

    /// 设置编码器应用模式
    pub async fn set_application(&mut self, application: Application) -> Result<(), String> {
        let mut encoder = self.encoder.lock().await;
        encoder
            .set_application(application)
            .map_err(|e| format!("设置编码器应用模式失败: {:?}", e))?;
        log::info!("编码器应用模式已更新: {:?}", application);
        Ok(())
    }

    /// 重置编码器状态
    ///
    /// 清除编码器的内部状态，包括预测历史和缓冲区。
    ///
    /// 用于处理音频流的中断、重新开始或状态重置，确保编码器处于干净状态。
    pub async fn reset_encoder(&self) -> Result<(), String> {
        let mut encoder = self.encoder.lock().await;
        encoder
            .reset_state()
            .map_err(|e| format!("重置编码器失败: {:?}", e))?;
        log::debug!("编码器状态已重置");
        Ok(())
    }

    /// 重置解码器状态
    ///
    /// 清除解码器的内部状态，包括预测历史和缓冲区。
    /// 用于处理音频流的中断、重新开始或状态重置，确保解码器处于干净状态。
    pub async fn reset_decoder(&self) -> Result<(), String> {
        let mut decoder = self.decoder.lock().await;
        decoder
            .reset_state()
            .map_err(|e| format!("重置解码器失败: {:?}", e))?;

        log::debug!("解码器状态已重置");
        Ok(())
    }
}
