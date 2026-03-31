//! 交互式音频设备切换工具
//!
//! 通过命令行菜单手动选择输入/输出设备进行切换，支持实时采集->播放回环，用于功能验证与调试。
//! 适合开发阶段对单次切换逻辑的探索与验证。

use audio_io::{AudioCapture, AudioCaptureControl, AudioPlayback, AudioPlaybackControl};
use std::io::{self, Write};
use tokio::io::{AsyncBufReadExt, BufReader};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .try_init()
        .ok();

    // 启动采集
    let mut capture = AudioCapture::builder()
        .build()
        .map_err(|e| format!("创建采集器失败: {}", e))?;

    let mut capture_rx = capture.start()?;

    // 启动播放
    let mut playback = AudioPlayback::builder()
        .build()
        .map_err(|e| format!("创建播放器失败: {}", e))?;

    let playback_tx = playback.start()?;

    // 回环任务：直接将采集数据发送到播放
    let _loopback_task = tokio::spawn(async move {
        while let Some(data) = capture_rx.recv().await {
            let _ = playback_tx.send(data);
        }
    });

    println!("✓ 音频回环已启动\n");

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut choice = String::new();

    loop {
        println!("\n=== 音频设备切换工具 ===");
        println!("1. 切换采集设备");
        println!("2. 切换播放设备");
        println!("3. 退出");
        print!("请选择 (1-3): ");
        io::stdout().flush()?;

        choice.clear();
        match reader.read_line(&mut choice).await {
            Ok(0) => {
                println!("\n输入流已关闭，正在退出...");
                break;
            }
            Ok(_) => {
                let choice_trimmed = choice.trim();
                match choice_trimmed {
                    "1" => {
                        if let Err(e) = switch_input_device(&mut capture, &mut reader).await {
                            println!("✗ 切换采集设备失败: {}", e);
                        }
                    }
                    "2" => {
                        if let Err(e) = switch_output_device(&mut playback, &mut reader).await {
                            println!("✗ 切换播放设备失败: {}", e);
                        }
                    }
                    "3" => {
                        println!("正在关闭...");
                        break;
                    }
                    _ => println!("无效选择，请重试"),
                }
            }
            Err(e) => {
                println!("读取输入失败: {}", e);
                break;
            }
        }
    }

    println!("✓ 程序结束");
    Ok(())
}

async fn switch_input_device(
    capture: &mut AudioCapture,
    reader: &mut BufReader<tokio::io::Stdin>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n--- 输入设备列表 ---");

    let devices = audio_io::utils::list_input_devices()?;
    if devices.is_empty() {
        println!("没有找到输入设备");
        return Ok(());
    }

    for (idx, device) in devices.iter().enumerate() {
        println!("{}: {}", idx + 1, device);
    }

    print!("请选择设备 (1-{}): ", devices.len());
    io::stdout().flush()?;

    let mut choice = String::new();
    reader.read_line(&mut choice).await?;
    let choice = choice.trim();

    if let Ok(idx) = choice.parse::<usize>() {
        if idx > 0 && idx <= devices.len() {
            let device_name = &devices[idx - 1];
            println!("\n正在切换到: {}", device_name);

            match capture.switch_device(device_name) {
                Ok(_) => {
                    println!("✓ 成功切换到输入设备: {}", device_name);
                }
                Err(e) => {
                    println!("✗ 切换失败: {}", e);
                }
            }
        } else {
            println!("选择超出范围");
        }
    } else {
        println!("无效的数字输入");
    }

    Ok(())
}

async fn switch_output_device(
    playback: &mut AudioPlayback,
    reader: &mut BufReader<tokio::io::Stdin>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n--- 输出设备列表 ---");

    let devices = audio_io::utils::list_output_devices()?;
    if devices.is_empty() {
        println!("没有找到输出设备");
        return Ok(());
    }

    for (idx, device) in devices.iter().enumerate() {
        println!("{}: {}", idx + 1, device);
    }

    print!("请选择设备 (1-{}): ", devices.len());
    io::stdout().flush()?;

    let mut choice = String::new();
    reader.read_line(&mut choice).await?;
    let choice = choice.trim();

    if let Ok(idx) = choice.parse::<usize>() {
        if idx > 0 && idx <= devices.len() {
            let device_name = &devices[idx - 1];
            println!("\n正在切换到: {}", device_name);

            match playback.switch_device(device_name) {
                Ok(_) => {
                    println!("✓ 成功切换到输出设备: {}", device_name);
                }
                Err(e) => {
                    println!("✗ 切换失败: {}", e);
                }
            }
        } else {
            println!("选择超出范围");
        }
    } else {
        println!("无效的数字输入");
    }

    Ok(())
}
