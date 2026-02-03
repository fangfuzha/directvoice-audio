# audio_io

音频采集与播放库，基于 `cpal` + `tokio`，内置重采样与环形缓冲，适用于实时语音场景。

## 特性

- 音频采集：默认 48kHz，支持自定义采样率与帧长，带音量/静音控制。
- 音频播放：单声道输入，自动复制到多声道输出，支持设备热切换。
- 重采样：使用 `rubato`，自动在设备采样率与目标采样率间转换。
- 环形缓冲：`ringbuf` 抗抖动设计，采集端有界通道(容量=2)防止内存膨胀。
- 并发模型：`tokio` 异步，采集/播放各自独立任务。

## 依赖环境

- Rust 1.78+ (2024 edition)
- 平台音频后端: Windows (WASAPI)、macOS (CoreAudio)、Linux (ALSA/PulseAudio)

## 安装

在同一工作区的其他 crate 中引用本地路径:

```toml
[dependencies]
audio_io = { path = "../audio_io" }
```

## 快速开始：采集到播放(loopback)

```rust
use audio_io::{AudioCapture, AudioPlayback, AudioCaptureControl, AudioPlaybackControl};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // 1) 创建采集器与播放器
    let mut capture = AudioCapture::new()?    // 默认 48kHz, frame_size=480 (10ms)
        .frame_size(480);
    let mut playback = AudioPlayback::new(Some(48_000))?;

    // 2) 启动采集(有界 mpsc 通道, 容量=2)
    let mut rx = capture.start()?;

    // 3) 启动播放( broadcast 发送端，可供多个订阅者消费 )
    let tx = playback.start()?;

    // 4) 简单转发: 采集到的帧直接送往播放
    while let Some(frame) = rx.recv().await {
        // 当消费端跟不上时，内部会丢弃最旧的数据，保证实时性
        if tx.send(frame).is_err() {
            break; // 播放端已关闭
        }
    }

    Ok(())
}
```

## API 速览

- 设备枚举: `list_input_devices()`, `list_output_devices()`
- 采集控制: `AudioCapture::new()`, `.sample_rate(u32)`, `.frame_size(usize)`, `.start()`, `.stop()`, `.switch_device()`, `.set_volume()`, `.set_mute()`
- 播放控制: `AudioPlayback::new(Some(sample_rate))`, `.start()`, `.stop()`, `.switch_device()`, `.set_volume()`, `.set_mute()`
- 通道语义:
  - 采集: `mpsc::channel(2)`，消费者若变慢则最新帧会被丢弃以保护实时性。
  - 播放: `broadcast::channel(2)`，支持多订阅者同时播放/录制/监控。

## 设计备注

- 采集在 `cpal` 回调线程中写环形缓冲；异步任务批量聚合为帧后推送通道。
- 播放端在收到单声道帧后，会自动复制到所有输出声道。
- Release 默认启用 `opt-level=3`, `lto=fat`, `codegen-units=1`, `panic=abort`, `strip=true`。

## 示例与调试

- 可执行示例: `cargo run --release --bin audio_loopback`
- 建议在桌面/耳机连接良好时测试，Ctrl+C 可优雅退出。
