//! 音频重采样工具
//!
//! 提供采集端和播放端共享的音频重采样功能

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::sync::{Arc, Mutex};

/// 创建音频重采样器
///
/// 当实际采样率与目标采样率不同时，创建重采样器进行音频重采样。
/// 使用 Sinc 插值算法提供高质量的重采样。
///
/// # 参数
/// - `actual_sample_rate`: 输入音频的实际采样率（Hz）
/// - `target_sample_rate`: 目标输出采样率（Hz）
///
/// # 返回值
/// - `Ok(Some(resampler))`: 成功创建重采样器（采样率不同时）
/// - `Ok(None)`: 不需要重采样（采样率相同）
/// - `Err(msg)`: 创建失败
///
/// # 重采样参数说明
/// - `sinc_len: 256`: Sinc 函数长度，越大质量越高但计算量越大
/// - `f_cutoff: 0.95`: 截止频率（相对于奈奎斯特频率），0.95 保留 95% 带宽
/// - `interpolation: Linear`: 线性插值，性能和质量的平衡
/// - `oversampling_factor: 256`: 过采样因子，提高插值精度
/// - `window: BlackmanHarris2`: 窗函数，减少频谱泄露
/// - `chunk_size: 1024`: 每次处理的采样点数
/// - `channels: 1`: 单声道处理
///
/// # 示例
/// ```rust,ignore
/// let resampler = create_resampler(16000, 48000)?;
/// if let Some(r) = resampler {
///     // 使用重采样器处理音频
/// }
/// ```
pub fn create_resampler(
    actual_sample_rate: u32,
    target_sample_rate: u32,
) -> Result<Option<Arc<Mutex<SincFixedIn<f32>>>>, String> {
    if actual_sample_rate == target_sample_rate {
        // 采样率相同，不需要重采样
        return Ok(None);
    }

    // 配置 Sinc 插值参数
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };

    // 创建重采样器
    // 参数：比率、最大比率变化、插值参数、块大小、声道数
    let resampler = SincFixedIn::<f32>::new(
        target_sample_rate as f64 / actual_sample_rate as f64,
        2.0, // 最大比率变化（允许动态调整）
        params,
        1024, // 块大小
        1,    // 单声道
    )
    .map_err(|e| format!("创建重采样器失败: {:?}", e))?;

    Ok(Some(Arc::new(Mutex::new(resampler))))
}

/// 对音频数据进行重采样
///
/// 使用提供的重采样器对音频数据进行重采样处理。
/// 采用非阻塞方式（try_lock），失败时回退到原始数据以保证实时性。
///
/// # 参数
/// - `resampler`: 可选的重采样器实例
/// - `data`: 待重采样的音频数据（单声道 f32 格式）
///
/// # 返回值
/// - 重采样后的音频数据，如果不需要重采样或失败则返回原始数据的克隆
///
/// # 行为说明
/// - 如果 `resampler` 为 `None`，返回数据的克隆
/// - 如果重采样器锁定失败（正在被其他线程使用），返回数据克隆并记录调试日志
/// - 如果重采样过程失败，返回数据克隆并记录警告日志
/// - 成功时返回重采样后的数据
///
/// # 示例
/// ```rust,ignore
/// let resampler = create_resampler(16000, 48000)?;
/// let audio_data = vec![0.1, 0.2, 0.3];
/// let resampled = resample_audio_data(&resampler, &audio_data);
/// ```
pub fn resample_audio_data(
    resampler: &Option<Arc<Mutex<SincFixedIn<f32>>>>,
    data: &[f32],
) -> Vec<f32> {
    if let Some(resampler) = resampler {
        match resampler.try_lock() {
            Ok(mut r) => {
                match r.process(&vec![data], None) {
                    Ok(mut waves_out) => {
                        // 直接取走向量，避免克隆
                        waves_out.pop().unwrap_or_else(|| data.to_vec())
                    }
                    Err(e) => {
                        log::warn!("重采样失败: {e:?}");
                        data.to_vec()
                    }
                }
            }
            Err(_) => {
                // 锁定失败，使用原始数据（避免阻塞）
                log::debug!("重采样器忙碌，使用原始数据");
                data.to_vec()
            }
        }
    } else {
        data.to_vec()
    }
}
