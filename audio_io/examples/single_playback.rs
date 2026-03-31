use audio_io::{AudioPlayback, AudioPlaybackControl};
use log::{error, info};
use std::f32::consts::TAU;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), String> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .format_timestamp_millis()
        .init();

    info!("=== 单路播放示例启动 ===");
    info!("按 Ctrl+C 停止程序");

    let sample_rate: u32 = 48_000;
    let frame_size: usize = 480;
    let mut playback = AudioPlayback::builder()
        .sample_rate(sample_rate)
        .frame_size(frame_size)
        .volume(0.2)
        .build()?;

    info!("播放设备: {}", playback.current_device_name());

    let sender = playback.start()?;
    let mut phase = 0.0_f32;
    let phase_step = 440.0_f32 * TAU / sample_rate as f32;
    let mut ticker = tokio::time::interval(Duration::from_millis(10));

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("接收到退出信号，正在关闭播放...");
                break;
            }
            _ = ticker.tick() => {
                let mut frame = Vec::with_capacity(frame_size);
                for _ in 0..frame_size {
                    frame.push(phase.sin() * 0.2);
                    phase += phase_step;
                    if phase > TAU {
                        phase -= TAU;
                    }
                }

                if let Err(e) = sender.send(frame) {
                    error!("发送播放数据失败: {}", e);
                    break;
                }
            }
        }
    }

    playback.stop();
    info!("单路播放结束");
    Ok(())
}