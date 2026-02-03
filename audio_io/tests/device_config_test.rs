//! 测试设备配置获取：跨类型调用行为

use cpal::traits::{DeviceTrait, HostTrait};

#[tokio::test]
async fn test_get_input_config_from_output_device() {
    // 获取默认输出设备
    if let Some(output_device) = audio_io::utils::get_host().default_output_device() {
        let device_name = output_device.name().unwrap_or_default();
        println!("输出设备: {}", device_name);

        // 尝试获取输出设备的输入配置（应该失败）
        match audio_io::utils::default_input_config(&output_device) {
            Ok(config) => {
                println!(
                    "❌ 输出设备 '{}' 竟然有输入配置！采样率: {}Hz, 声道数: {}",
                    device_name,
                    config.sample_rate().0,
                    config.channels()
                );
            }
            Err(e) => {
                println!(
                    "✓ 输出设备 '{}' 无输入配置（预期的错误）: {}",
                    device_name, e
                );
            }
        }
    } else {
        println!("未找到默认输出设备");
    }
}

#[test]
fn test_get_output_config_from_input_device() {
    // 获取默认输入设备
    if let Some(input_device) = audio_io::utils::get_host().default_input_device() {
        let device_name = input_device.name().unwrap_or_default();
        println!("输入设备: {}", device_name);

        // 尝试获取输入设备的输出配置（应该失败）
        match audio_io::utils::default_output_config(&input_device) {
            Ok(config) => {
                println!(
                    "❌ 输入设备 '{}' 竟然有输出配置！采样率: {}Hz, 声道数: {}",
                    device_name,
                    config.sample_rate().0,
                    config.channels()
                );
            }
            Err(e) => {
                println!(
                    "✓ 输入设备 '{}' 无输出配置（预期的错误）: {}",
                    device_name, e
                );
            }
        }
    } else {
        println!("未找到默认输入设备");
    }
}
