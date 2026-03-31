//! 音频播放模块测试

use audio_io::{AudioPlayback, AudioPlaybackControl};

#[test]
fn test_playback_creation() {
    let playback = AudioPlayback::builder().build();
    assert!(playback.is_ok(), "应该能成功创建播放器");
}

#[test]
fn test_current_device_name() {
    let playback = AudioPlayback::builder().build().unwrap();
    let device_name = playback.current_device_name();
    assert!(!device_name.is_empty(), "设备名称不应为空");
    println!("当前输出设备: {}", device_name);
}

#[test]
fn test_playback_state() {
    let playback = AudioPlayback::builder().build().unwrap();
    assert!(!playback.is_playing(), "初始状态应该不在播放");
}

#[test]
fn test_volume_control() {
    let mut playback = AudioPlayback::builder().build().unwrap();

    // 测试设置和获取音量
    playback.set_volume(0.5);
    assert_eq!(playback.get_volume(), 0.5, "音量应该是 0.5");

    // 测试音量范围限制
    playback.set_volume(1.5);
    assert_eq!(playback.get_volume(), 1.0, "音量应该被限制在 1.0");

    playback.set_volume(-0.5);
    assert_eq!(playback.get_volume(), 0.0, "音量应该被限制在 0.0");
}

#[test]
fn test_mute_control() {
    let mut playback = AudioPlayback::builder().build().unwrap();

    // 初始应该不静音
    assert!(!playback.is_muted(), "初始应该不静音");

    // 设置静音
    playback.set_mute(true);
    assert!(playback.is_muted(), "应该处于静音状态");

    // 取消静音
    playback.set_mute(false);
    assert!(!playback.is_muted(), "应该取消静音");
}

#[test]
fn test_list_devices() {
    let playback = AudioPlayback::builder().build().unwrap();
    let devices = playback.list_devices();
    assert!(devices.is_ok(), "应该能列出设备");

    let devices = devices.unwrap();
    assert!(!devices.is_empty(), "至少应该有一个输出设备");

    println!("可用输出设备:");
    for device in &devices {
        println!("  - {}", device);
    }
}

#[test]
fn test_switch_device_invalid() {
    let mut playback = AudioPlayback::builder().build().unwrap();
    let original_device = playback.current_device_name();

    // 尝试切换到不存在的设备
    let result = playback.switch_device("不存在的设备");
    assert!(result.is_err(), "切换到不存在的设备应该失败");
    assert_eq!(
        playback.current_device_name(),
        original_device,
        "设备不应该改变"
    );
}

#[test]
fn test_switch_device_to_same() {
    let mut playback = AudioPlayback::builder().build().unwrap();
    let original_device = playback.current_device_name();

    // 切换到相同的设备
    let result = playback.switch_device(&original_device);
    assert!(result.is_ok(), "切换到相同设备应该成功");
    assert_eq!(playback.current_device_name(), original_device);
}

#[test]
fn test_channels() {
    let playback = AudioPlayback::builder().build().unwrap();
    let channels = playback.channels();
    assert!(channels >= 1, "至少应该有一个声道");
    println!("输出设备声道数: {}", channels);
}

#[test]
fn test_sample_rate_config() {
    let playback = AudioPlayback::builder()
        .sample_rate(16000)
        .frame_size(320)
        .build()
        .unwrap();

    // 配置应该成功应用
    assert!(!playback.is_playing());
}

#[test]
fn test_stop_when_not_playing() {
    let mut playback = AudioPlayback::builder().build().unwrap();

    // 停止一个未播放的播放器应该返回 false
    let result = playback.stop();
    assert!(!result, "停止未播放的播放器应该返回 false");
}

#[test]
fn test_builder_with_volume() {
    let playback = AudioPlayback::builder().volume(0.6).build().unwrap();
    assert_eq!(playback.get_volume(), 0.6);
}

#[test]
fn test_builder_with_mute() {
    let playback = AudioPlayback::builder().mute(true).build().unwrap();
    assert!(playback.is_muted());
}

#[test]
fn test_builder_full_config() {
    let playback = AudioPlayback::builder()
        .sample_rate(16000)
        .frame_size(320)
        .volume(0.7)
        .mute(false)
        .build()
        .unwrap();

    assert_eq!(playback.get_volume(), 0.7);
    assert!(!playback.is_muted());
}

#[test]
fn test_builder_default_mixer_available() {
    let playback = AudioPlayback::builder().build().unwrap();
    let source = playback.create_mixer_source();
    assert!(source.is_ok());
}
