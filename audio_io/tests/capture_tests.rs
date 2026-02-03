//! 音频采集模块测试

use audio_io::{AudioCapture, AudioCaptureControl};

#[test]
fn test_current_device_name() {
    let capture = AudioCapture::builder().build().unwrap();
    let device_name = capture.current_device_name();
    assert!(!device_name.is_empty());
    println!("当前输入设备: {}", device_name);
}

#[test]
fn test_switch_device_invalid() {
    let mut capture = AudioCapture::builder().build().unwrap();
    let original_device = capture.current_device_name();

    // 尝试切换到不存在的设备
    let result = capture.switch_device("不存在的设备");
    assert!(result.is_err());
    assert_eq!(capture.current_device_name(), original_device);
}

#[test]
fn test_switch_device_to_same() {
    let mut capture = AudioCapture::builder().build().unwrap();
    let original_device = capture.current_device_name();

    // 切换到相同的设备
    let result = capture.switch_device(&original_device);
    assert!(result.is_ok());
    assert_eq!(capture.current_device_name(), original_device);
}

#[test]
fn test_builder_with_config() {
    let capture = AudioCapture::builder()
        .sample_rate(16000)
        .frame_size(320)
        .build();
    assert!(capture.is_ok());
}

#[test]
fn test_builder_with_volume() {
    let capture = AudioCapture::builder()
        .volume(0.5)
        .build()
        .unwrap();
    assert_eq!(capture.get_volume(), 0.5);
}

#[test]
fn test_builder_with_mute() {
    let capture = AudioCapture::builder()
        .mute(true)
        .build()
        .unwrap();
    assert!(capture.is_muted());
}

#[test]
fn test_builder_full_config() {
    let capture = AudioCapture::builder()
        .sample_rate(16000)
        .frame_size(320)
        .volume(0.8)
        .mute(false)
        .build()
        .unwrap();
    
    assert_eq!(capture.get_volume(), 0.8);
    assert!(!capture.is_muted());
}
