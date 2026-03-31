use audio_io::{AudioPlayback, AudioPlaybackControl};
use log::{error, info};
use std::f32::consts::TAU;
use std::time::Duration;

async fn run() -> Result<(), String> {
    let sample_rate: u32 = 48_000;
    let frame_size: usize = 480;

    let mut playback = AudioPlayback::builder()
        .sample_rate(sample_rate)
        .frame_size(frame_size)
        .volume(0.2)
        .build()?;

    info!("播放设备: {}", playback.current_device_name());

    let source_a = playback.create_mixer_source()?;
    let source_b = playback.create_mixer_source()?;
    let _sender = playback.start()?;

    let task_a = tokio::spawn(spawn_tone_source(source_a, 440.0, sample_rate, frame_size));
    let task_b = tokio::spawn(spawn_tone_source(source_b, 660.0, sample_rate, frame_size));

    tokio::signal::ctrl_c()
        .await
        .map_err(|e| format!("等待退出信号失败: {e}"))?;

    task_a.abort();
    task_b.abort();
    playback.stop();
    Ok(())
}

async fn spawn_tone_source(
    source: audio_io::AudioMixerSource,
    frequency: f32,
    sample_rate: u32,
    frame_size: usize,
) {
    let mut phase = 0.0_f32;
    let phase_step = frequency * TAU / sample_rate as f32;
    let mut ticker = tokio::time::interval(Duration::from_millis(10));

    loop {
        ticker.tick().await;

        let mut frame = Vec::with_capacity(frame_size);
        for _ in 0..frame_size {
            frame.push(phase.sin() * 0.15);
            phase += phase_step;
            if phase > TAU {
                phase -= TAU;
            }
        }

        if let Err(e) = source.try_send(frame) {
            error!("混音源 {} 发送失败: {}", source.source_id(), e);
            break;
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .format_timestamp_millis()
        .init();

    info!("=== 多路播放示例启动 ===");
    info!("按 Ctrl+C 停止程序");
    run().await
}
