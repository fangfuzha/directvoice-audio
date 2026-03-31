use audio_io::{AudioCapture, AudioCaptureControl};
use log::{error, info};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> Result<(), String> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .format_timestamp_millis()
        .init();

    info!("=== 单路采集示例启动 ===");
    info!("按 Ctrl+C 停止程序");

    let mut capture = AudioCapture::builder()
        .sample_rate(48_000)
        .frame_size(480)
        .build()?;

    info!("采集设备: {}", capture.current_device_name());

    let mut receiver = capture.start()?;
    let mut frames_processed: u64 = 0;
    let mut total_samples: u64 = 0;
    let start_time = Instant::now();
    let mut stats_time = Instant::now();

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("接收到退出信号，正在关闭采集...");
                break;
            }
            maybe_frame = receiver.recv() => {
                match maybe_frame {
                    Some(frame) => {
                        frames_processed += 1;
                        total_samples += frame.len() as u64;

                        if stats_time.elapsed() >= Duration::from_secs(1) {
                            let elapsed = start_time.elapsed().as_secs_f32().max(0.001);
                            info!(
                                "采集统计: 帧数={}, 样本数={}, 吞吐量={:.0} samples/s",
                                frames_processed,
                                total_samples,
                                total_samples as f32 / elapsed,
                            );
                            stats_time = Instant::now();
                        }
                    }
                    None => {
                        error!("采集通道已关闭");
                        break;
                    }
                }
            }
        }
    }

    capture.stop();
    info!(
        "单路采集结束，帧数={}, 样本数={}",
        frames_processed, total_samples
    );
    Ok(())
}
