//! 设备频繁切换压力测试
//!
//! 自动在可用音频设备之间频繁切换，测试是否存在内存泄漏或其他资源问题

use audio_io::{AudioCapture, AudioCaptureControl, AudioPlayback, AudioPlaybackControl};
use log::{error, info, warn};
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .format_timestamp_millis()
        .init();

    info!("\n=== 音频设备切换压力测试 ===");
    info!("提示: 本程序将自动频繁切换音频设备以测试内存泄漏");
    info!("按 Ctrl+C 停止测试\n");

    // 获取可用设备列表
    let input_devices = audio_io::utils::list_input_devices()?;
    let output_devices = audio_io::utils::list_output_devices()?;

    if input_devices.is_empty() {
        error!("没有找到输入设备，无法进行测试");
        return Ok(());
    }
    if output_devices.is_empty() {
        error!("没有找到输出设备，无法进行测试");
        return Ok(());
    }

    info!("找到 {} 个输入设备:", input_devices.len());
    for (idx, device) in input_devices.iter().enumerate() {
        info!("  {}. {}", idx + 1, device);
    }
    info!("找到 {} 个输出设备:", output_devices.len());
    for (idx, device) in output_devices.iter().enumerate() {
        info!("  {}. {}", idx + 1, device);
    }

    // 启动采集
    let mut capture = AudioCapture::builder().build()?;
    let mut capture_rx = capture.start()?;

    // 启动播放
    let mut playback = AudioPlayback::builder().build()?;
    let playback_tx = playback.start()?;

    // 回环任务
    let _loopback_task = tokio::spawn(async move {
        while let Some(data) = capture_rx.recv().await {
            let _ = playback_tx.send(data);
        }
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

    info!("\n>>> 开始设备切换压力测试...\n");

    let mut input_device_idx = 0;
    let mut output_device_idx = 0;

    // 设置 Ctrl+C 处理
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        tokio::select! {
            _ = &mut ctrl_c => {
                info!("\n接收到退出信号，停止测试...");
                break;
            }
            _ = sleep(Duration::from_millis(50)) => {
                // 切换输入设备
                input_device_idx = (input_device_idx + 1) % input_devices.len();
                let input_device = &input_devices[input_device_idx];

                match capture.switch_device(input_device) {
                    Ok(_) => {
                        info!("✓ 切换输入设备到: {}", input_device);
                    }
                    Err(e) => {
                        warn!("✗ 切换输入设备失败: {}", e);
                    }
                }

                // 切换输出设备
                output_device_idx = (output_device_idx + 1) % output_devices.len();
                let output_device = &output_devices[output_device_idx];

                match playback.switch_device(output_device) {
                    Ok(_) => {
                        info!("✓ 切换输出设备到: {}", output_device);
                    }
                    Err(e) => {
                        warn!("✗ 切换输出设备失败: {}", e);
                    }
                }
            }
        }
    }

    // 清理
    capture.stop();
    playback.stop();

    info!("\n=== 测试结束 ===");
    Ok(())
}
