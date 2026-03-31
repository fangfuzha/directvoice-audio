//! AudioSession 包装层使用示例
//!
//! 这个示例展示如何在 `audio_io` 内部使用新增的会话层：
//!
//! 麦克风原始 PCM 帧 -> AEC 回声消除 -> NS 降噪 -> 播放，并可订阅处理后的帧。

use audio_io::AudioSession;
use log::info;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), String> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .format_timestamp_millis()
        .init();

    info!("=== AudioSession 会话示例启动 ===");
    info!("按 Ctrl+C 停止程序");

    let session = AudioSession::builder()
        .expose_raw_capture(false)
        .expose_processed_capture(true)
        .enable_auto_loopback(false)
        .set_aec_status(false)
        .set_ns_status(false)
        .build()?;

    let handle = session.start()?;
    info!("会话已启动: {}", handle.is_running());

    let mut processed_rx = handle
        .subscribe_processed_capture()
        .ok_or_else(|| "未启用处理后采集帧输出".to_string())?;

    let mut processed_frames: u64 = 0;
    let mut total_samples: u64 = 0;
    let start_time = Instant::now();
    let mut loopback_enabled = false;
    let mut stats_tick = tokio::time::interval(Duration::from_secs(1));

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("接收到退出信号，准备关闭会话...");
                break;
            }
            _ = stats_tick.tick() => {
                if !loopback_enabled && start_time.elapsed() >= Duration::from_millis(200) {
                    handle.set_auto_loopback_enabled(true);
                    loopback_enabled = true;
                    info!("已在运行时开启自动回环");
                }
                info!(
                    "会话统计: 帧数={}, 样本数={}",
                    processed_frames,
                    total_samples
                );
            }
            maybe_frame = processed_rx.recv() => {
                match maybe_frame {
                    Ok(frame) => {
                        processed_frames += 1;
                        total_samples += frame.len() as u64;
                    }
                    Err(error) => {
                        info!("处理后采集通道关闭: {}", error);
                        break;
                    }
                }
            }
        }
    }

    handle.stop()?;
    info!(
        "AudioSession 示例结束，帧数={}, 样本数={}",
        processed_frames, total_samples
    );
    Ok(())
}
