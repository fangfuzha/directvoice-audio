//! 音频回环程序
//!
//! 直接将采集到的音频数据发送到播放设备，用于快速检查音频输入输出是否正常。
//! 并每秒统计处理的音频帧数和样本数。以及内存使用情况。
use audio_io::{AudioCapture, AudioCaptureControl, AudioPlayback, AudioPlaybackControl};
use log::{error, info};
use std::time::{Duration, Instant};
#[tokio::main]
async fn main() -> Result<(), String> {
    // 初始化日志为 info 级别
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .format_timestamp_millis()
        .init();
    info!("\n=== 音频回环程序启动 ===");
    info!("提示: 请确保已连接麦克风和扬声器/耳机，你将听到自己的声音。");
    info!("按 Ctrl+C 停止程序\n");
    // 监听 Ctrl+C 信号
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    // 1. 初始化采集器
    let mut capture = AudioCapture::builder()
        .sample_rate(48000)
        .frame_size(1920)
        .build()?;
    info!("✓ 采集设备: {}", capture.current_device_name());
    // 2. 初始化播放器
    let mut playback = AudioPlayback::builder()
        .sample_rate(48000)
        .frame_size(1920)
        .build()?;
    info!("✓ 播放设备: {}", playback.current_device_name());
    // 3. 启动采集
    let mut receiver = capture.start()?;
    info!("✓ 采集已启动");
    // 4. 启动播放
    let sender = playback.start()?;
    info!("✓ 播放已启动");
    // 启动内存监控任务
    let _memory_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            #[cfg(target_os = "windows")]
            {
                if let Ok(output) = std::process::Command::new("powershell")
                    .args([
                        "-Command",
                        &format!(
                            "(Get-Process -Id {}).WorkingSet64 / 1MB",
                            std::process::id()
                        ),
                    ])
                    .output()
                {
                    if let Ok(mem_str) = String::from_utf8(output.stdout) {
                        if let Ok(mem_mb) = mem_str.trim().parse::<f64>() {
                            info!("内存使用: {:.2} MB", mem_mb);
                        }
                    }
                }
            }
        }
    });
    info!(">>> 正在进行音频回环播放...\n");
    let start_time = Instant::now();
    let mut frames_processed = 0u64;
    let mut total_samples = 0u64;
    let mut stats_time = Instant::now();
    loop {
        tokio::select! {
            _ = &mut ctrl_c => {
                info!("\n接收到退出信号，正在关闭...");
                break;
            }
            Some(data) = receiver.recv() => {
                frames_processed += 1;
                total_samples += data.len() as u64;
                // log::info!("接收到 {} 个样本", data.len());
                // 发送到播放设备
                if let Err(e) = sender.send(data) {
                    error!("⚠️ 发送音频数据失败: {}", e);
                    break;
                }
                // 每秒输出统计信息
                if stats_time.elapsed() > Duration::from_secs(1) {
                    let elapsed = start_time.elapsed().as_secs_f32();
                    let throughput = total_samples as f32 / elapsed;
                    info!(
                        "处理进度 - 帧数: {}, 样本数: {}, 吞吐量: {:.0} samples/s",
                        frames_processed, total_samples, throughput
                    );
                    stats_time = Instant::now();
                }
            }
        }
    }
    // 清理资源
    capture.stop();
    playback.stop();
    let elapsed = start_time.elapsed();
    info!("\n=== 音频回环程序结束 ===");
    info!("运行时长: {:.2}s", elapsed.as_secs_f32());
    info!("总帧数: {}", frames_processed);
    info!("总样本数: {}", total_samples);
    info!(
        "平均吞吐量: {:.0} samples/s",
        total_samples as f32 / elapsed.as_secs_f32()
    );
    Ok(())
}
