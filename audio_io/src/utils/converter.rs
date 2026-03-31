//! 音频采样转换工具
//!
//! 提供不同采样格式（f32, i16, u16）到单声道 f32 的转换功能。

use std::sync::LazyLock;

// 音频处理常量
const I16_NORMALIZATION_FACTOR: f32 = 32768.0;
const U16_OFFSET: f32 = 32768.0;

/// 全局 SIMD 架构检测实例
static SIMD_ARCH: LazyLock<pulp::Arch> = LazyLock::new(pulp::Arch::new);

/// 音频数据转换 trait
pub trait AudioSampleConverter {
    /// 将音频数据转换为 `单声道` `f32` 格式
    ///
    /// # 参数
    /// - `channels`: 源音频数据的声道数（如 1 为单声道，2 为双声道）。
    ///   如果是多声道，则会将各声道数据取平均值合并为单声道。
    ///
    /// # 返回
    /// `单声道` `f32` 数据
    fn convert_to_mono_f32(&self, channels: usize) -> Vec<f32>;
}

impl AudioSampleConverter for [f32] {
    fn convert_to_mono_f32(&self, channels: usize) -> Vec<f32> {
        if channels == 1 {
            return self.to_vec();
        }

        let frame_count = self.len() / channels;
        let mut output = Vec::with_capacity(frame_count);
        // 安全地设置长度，因为我们马上就会填充它
        unsafe { output.set_len(frame_count) };

        SIMD_ARCH.dispatch(|| match channels {
            2 => {
                for (out, frame) in output.iter_mut().zip(self.chunks_exact(2)) {
                    *out = (frame[0] + frame[1]) * 0.5;
                }
            }
            _ => {
                let inv_channels = 1.0 / channels as f32;
                for (out, frame) in output.iter_mut().zip(self.chunks_exact(channels)) {
                    *out = frame.iter().sum::<f32>() * inv_channels;
                }
            }
        });

        output
    }
}

impl AudioSampleConverter for [i16] {
    fn convert_to_mono_f32(&self, channels: usize) -> Vec<f32> {
        let frame_count = self.len() / channels;
        let mut output = Vec::with_capacity(frame_count);
        unsafe { output.set_len(frame_count) };

        SIMD_ARCH.dispatch(|| match channels {
            1 => {
                let inv_norm = 1.0 / I16_NORMALIZATION_FACTOR;
                for (out, &s) in output.iter_mut().zip(self.iter()) {
                    *out = s as f32 * inv_norm;
                }
            }
            2 => {
                let inv_norm_channels = 1.0 / (I16_NORMALIZATION_FACTOR * 2.0);
                for (out, frame) in output.iter_mut().zip(self.chunks_exact(2)) {
                    *out = (frame[0] as i32 + frame[1] as i32) as f32 * inv_norm_channels;
                }
            }
            _ => {
                let inv_norm_channels = 1.0 / (I16_NORMALIZATION_FACTOR * channels as f32);
                for (out, frame) in output.iter_mut().zip(self.chunks_exact(channels)) {
                    let sum: i32 = frame.iter().map(|&s| s as i32).sum();
                    *out = sum as f32 * inv_norm_channels;
                }
            }
        });

        output
    }
}

impl AudioSampleConverter for [u16] {
    fn convert_to_mono_f32(&self, channels: usize) -> Vec<f32> {
        let frame_count = self.len() / channels;
        let mut output = Vec::with_capacity(frame_count);
        unsafe { output.set_len(frame_count) };

        SIMD_ARCH.dispatch(|| match channels {
            1 => {
                let inv_offset = 1.0 / U16_OFFSET;
                for (out, &s) in output.iter_mut().zip(self.iter()) {
                    *out = (s as f32 - U16_OFFSET) * inv_offset;
                }
            }
            2 => {
                let inv_offset_channels = 1.0 / (U16_OFFSET * 2.0);
                for (out, frame) in output.iter_mut().zip(self.chunks_exact(2)) {
                    let sum = (frame[0] as f32 - U16_OFFSET) + (frame[1] as f32 - U16_OFFSET);
                    *out = sum * inv_offset_channels;
                }
            }
            _ => {
                let inv_channels = 1.0 / channels as f32;
                let inv_offset = 1.0 / U16_OFFSET;
                for (out, frame) in output.iter_mut().zip(self.chunks_exact(channels)) {
                    let sum: f32 = frame.iter().map(|&s| s as f32 - U16_OFFSET).sum();
                    *out = (sum * inv_channels) * inv_offset;
                }
            }
        });

        output
    }
}

// 音频输出常量
const I16_OUT_MAX: f32 = 32767.0;
const U16_OUT_OFFSET: u16 = 32768;
const U16_OUT_SCALE: f32 = 32767.0;

/// 音频输出数据转换 trait
pub trait AudioOutputConverter {
    /// f32 缓冲区的数据经过`格式转换`&&`音量控制`&&`声道复制`后写入目标缓冲区
    ///
    /// # 参数
    /// - `source`: 源 f32 缓冲区
    /// - `volume`: 音量控制因子 (0.0 - 1.0)
    /// - `channels`: 目标缓冲区的声道数
    ///
    /// 返回消耗的源样本数（单声道样本数）
    fn write_samples(&mut self, source: &[f32], volume: f32, channels: usize) -> usize;

    /// 填充静音值
    fn fill_silence(&mut self);
}

impl AudioOutputConverter for [f32] {
    fn write_samples(&mut self, source: &[f32], volume: f32, channels: usize) -> usize {
        let consumed = (self.len() / channels).min(source.len());
        if consumed == 0 {
            return 0;
        }
        let target = &mut self[..consumed * channels];
        let source = &source[..consumed];

        SIMD_ARCH.dispatch(|| match channels {
            1 => {
                for (t, s) in target.iter_mut().zip(source.iter()) {
                    *t = s * volume;
                }
            }
            2 => {
                for (t, s) in target.chunks_exact_mut(2).zip(source.iter()) {
                    let v = s * volume;
                    t[0] = v;
                    t[1] = v;
                }
            }
            _ => {
                for (t, s) in target.chunks_exact_mut(channels).zip(source.iter()) {
                    let v = s * volume;
                    t.fill(v);
                }
            }
        });
        consumed
    }

    fn fill_silence(&mut self) {
        self.fill(0.0);
    }
}

impl AudioOutputConverter for [i16] {
    fn write_samples(&mut self, source: &[f32], volume: f32, channels: usize) -> usize {
        let consumed = (self.len() / channels).min(source.len());
        if consumed == 0 {
            return 0;
        }
        let target = &mut self[..consumed * channels];
        let source = &source[..consumed];

        SIMD_ARCH.dispatch(|| match channels {
            1 => {
                for (t, s) in target.iter_mut().zip(source.iter()) {
                    *t = (s * volume * I16_OUT_MAX) as i16;
                }
            }
            2 => {
                for (t, s) in target.chunks_exact_mut(2).zip(source.iter()) {
                    let v = (s * volume * I16_OUT_MAX) as i16;
                    t[0] = v;
                    t[1] = v;
                }
            }
            _ => {
                for (t, s) in target.chunks_exact_mut(channels).zip(source.iter()) {
                    let v = (s * volume * I16_OUT_MAX) as i16;
                    t.fill(v);
                }
            }
        });
        consumed
    }

    fn fill_silence(&mut self) {
        self.fill(0);
    }
}

impl AudioOutputConverter for [u16] {
    fn write_samples(&mut self, source: &[f32], volume: f32, channels: usize) -> usize {
        let consumed = (self.len() / channels).min(source.len());
        if consumed == 0 {
            return 0;
        }
        let target = &mut self[..consumed * channels];
        let source = &source[..consumed];

        SIMD_ARCH.dispatch(|| match channels {
            1 => {
                for (t, s) in target.iter_mut().zip(source.iter()) {
                    *t = ((s * volume * U16_OUT_SCALE) + U16_OUT_OFFSET as f32) as u16;
                }
            }
            2 => {
                for (t, s) in target.chunks_exact_mut(2).zip(source.iter()) {
                    let v = ((s * volume * U16_OUT_SCALE) + U16_OUT_OFFSET as f32) as u16;
                    t[0] = v;
                    t[1] = v;
                }
            }
            _ => {
                for (t, s) in target.chunks_exact_mut(channels).zip(source.iter()) {
                    let v = ((s * volume * U16_OUT_SCALE) + U16_OUT_OFFSET as f32) as u16;
                    t.fill(v);
                }
            }
        });
        consumed
    }

    fn fill_silence(&mut self) {
        self.fill(U16_OUT_OFFSET);
    }
}
