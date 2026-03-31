//! 音频模块集成示例
//!
//! 用于演示 `audio_codec` 与 `audio_io` 的端到端链路：采集 -> 编码 -> 解码 -> 播放。

use audio_codec::{Application, AudioCodec};
use audio_io::{AudioCapture, AudioCaptureControl, AudioPlayback, AudioPlaybackControl};
use log::LevelFilter::Debug;
use std::time::Duration;
use tokio::time;

/// 音频集成示例。
///
/// 流程：
/// 1. 创建音频采集器、编解码器和播放器
/// 2. 启动采集，收集音频数据
/// 3. 对音频数据进行编码和解码
/// 4. 将解码后的数据发送到播放器进行播放
/// 5. 验证整个流程是否正常工作
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    log::info!("\n=== 开始音频集成示例 ===");

    let _ = env_logger::Builder::new().filter_level(Debug).try_init();

    log::info!("1. 创建音频采集器...");
    let mut capture = AudioCapture::builder()
        .sample_rate(48_000)
        .frame_size(480)
        .build()?;
    log::info!(
        "✓ 采集器创建成功，使用设备: {}",
        capture.current_device_name()
    );

    log::info!("2. 创建音频编解码器...");
    let codec = AudioCodec::builder()
        .sample_rate(48_000)
        .frame_size(480)
        .bitrate(64_000)
        .application(Application::LowDelay)
        .build()?;
    log::info!(
        "✓ 编解码器创建成功，采样率: {}Hz, 帧大小: {}",
        codec.sample_rate(),
        codec.frame_size()
    );

    log::info!("3. 创建音频播放器...");
    let mut playback = AudioPlayback::builder()
        .sample_rate(48_000)
        .frame_size(480)
        .build()?;
    log::info!(
        "✓ 播放器创建成功，使用设备: {}",
        playback.current_device_name()
    );

    log::info!("4. 启动音频采集...");
    let mut receiver = capture.start()?;
    log::info!("✓ 音频采集已启动");

    log::info!("5. 启动音频播放...");
    let sender = playback.start()?;
    log::info!("✓ 音频播放已启动");

    log::info!("6. 开始音频数据处理循环...");
    let mut processed_frames = 0;
    let mut total_samples = 0;
    let mut total_encoded_bytes: u64 = 0;
    let start_time = std::time::Instant::now();
    let test_duration = Duration::from_secs(5);

    log::info!("测试将在 5 秒后自动结束...");

    loop {
        if start_time.elapsed() >= test_duration {
            log::info!("测试 5 秒已到，准备退出...");
            break;
        }

        let audio_data = match time::timeout(Duration::from_millis(1000), receiver.recv()).await {
            Ok(Some(data)) => data,
            Ok(None) => {
                log::info!("音频采集通道已关闭");
                break;
            }
            Err(_) => {
                continue;
            }
        };

        if audio_data.is_empty() {
            continue;
        }

        total_samples += audio_data.len();
        log::info!("接收到音频数据: {} 样本", audio_data.len());

        let encoded_frames = codec.encode_stream(&audio_data).await?;
        total_encoded_bytes += encoded_frames.iter().map(|f| f.len() as u64).sum::<u64>();
        log::info!("编码完成: {} 帧", encoded_frames.len());

        let decoded_data = codec.decode_stream(&encoded_frames).await?;
        log::info!("解码完成: {} 样本", decoded_data.len());

        sender
            .send(decoded_data)
            .map_err(|_| "发送音频数据到播放器失败")?;

        processed_frames += 1;
    }

    log::info!("7. 停止音频处理...");
    capture.stop();
    playback.stop();
    log::info!("✓ 音频采集和播放已停止");

    log::info!("\n=== 测试结果统计 ===");
    let elapsed_s_f32 = start_time.elapsed().as_secs_f32();
    let elapsed_s = start_time.elapsed().as_secs_f64();
    let raw_bytes = (total_samples as u64) * 4;
    let raw_bytes_rate = raw_bytes as f64 / elapsed_s;
    let encoded_bytes_rate = total_encoded_bytes as f64 / elapsed_s;

    log::info!("处理音频帧数: {}", processed_frames);
    log::info!("总音频样本数: {}", total_samples);
    log::info!("原始PCM字节速率: {:.2} B/s", raw_bytes_rate);
    log::info!("编码后总字节速率: {:.2} B/s", encoded_bytes_rate);
    if raw_bytes > 0 {
        let compression_ratio = total_encoded_bytes as f64 / raw_bytes as f64;
        log::info!("压缩比: {:.3} (编码/原始)", compression_ratio);
    }
    log::info!("测试持续时间: {:.2}秒", elapsed_s_f32);
    if elapsed_s_f32 > 0.0 {
        log::info!("平均帧率: {:.1} FPS", processed_frames as f32 / elapsed_s_f32);
    }
    if elapsed_s > 0.0 {
        let avg_bitrate_kbps = (total_encoded_bytes as f64 * 8.0) / elapsed_s / 1000.0;
        log::info!("平均比特率: {:.2} kbps", avg_bitrate_kbps);
    }

    Ok(())
}