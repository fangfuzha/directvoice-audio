//! 音频采集设备切换压力测试（仅输入）
//!
//! 自动在所有可用输入设备间循环切换，并监控进程内存使用，验证采集切换稳定性与泄漏风险。
//! 不包含输出回环播放，仅用于采集切换场景的压力测试。

use audio_io::{AudioCapture, AudioCaptureControl};
use log::{error, info, warn};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .format_timestamp_millis()
        .init();

    info!("\n=== 音频采集设备切换压力测试 ===");
    info!("提示: 本程序将高速切换采集设备以测试内存泄漏");
    info!("按 Ctrl+C 停止测试\n");

    // 获取可用输入设备
    let input_devices = audio_io::utils::list_input_devices()?;

    if input_devices.is_empty() {
        error!("没有找到输入设备，无法进行测试");
        return Ok(());
    }

    if input_devices.len() < 2 {
        warn!("只有一个输入设备，切换测试效果有限");
    }

    info!("找到 {} 个输入设备:", input_devices.len());
    for (idx, device) in input_devices.iter().enumerate() {
        info!("  {}. {}", idx + 1, device);
    }

    // 创建并启动采集器
    let mut capture = AudioCapture::builder()
        .sample_rate(48000)
        .frame_size(960)
        .build()?;

    let mut receiver = capture.start()?;
    info!(
        "✓ 采集已启动，当前设备: {}\n",
        capture.current_device_name()
    );

    // 启动数据消费任务（防止 channel 阻塞）
    let _consumer_task = tokio::spawn(async move {
        let mut count = 0u64;
        while let Some(data) = receiver.recv().await {
            count += data.len() as u64;
            // 静默消费，不输出
        }
        info!("消费任务结束，总样本数: {}", count);
    });

    // 内存监控任务
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

    info!(">>> 开始设备切换压力测试...\n");

    let mut device_idx = 0;

    // 设置 Ctrl+C 处理
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        tokio::select! {
            _ = &mut ctrl_c => {
                info!("\n接收到退出信号，停止测试...");
                break;
            }
            _ = sleep(Duration::from_millis(100)) => {
                // 切换到下一个设备
                device_idx = (device_idx + 1) % input_devices.len();
                let target_device = &input_devices[device_idx];

                match capture.switch_device(target_device) {
                    Ok(_) => {
                        info!("✓ 切换到: {}", target_device);
                    }
                    Err(e) => {
                        warn!("✗ 切换失败: {}", e);
                    }
                }
            }
        }
    }

    // 清理
    capture.stop();

    info!("\n=== 测试结束 ===");
    Ok(())
}
