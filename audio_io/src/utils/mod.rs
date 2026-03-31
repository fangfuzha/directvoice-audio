//! 音频工具模块
//!
//! 提供音频相关的工具函数
pub mod converter;
pub mod resampler;
use cpal::Device;
use cpal::SupportedStreamConfig;
use cpal::traits::{DeviceTrait, HostTrait};
use std::sync::LazyLock;
/// 全局音频主机实例
static AUDIO_HOST: LazyLock<cpal::Host> = LazyLock::new(cpal::default_host);
/// 获取全局音频主机引用
pub fn get_host() -> &'static cpal::Host {
    &AUDIO_HOST
}
/// 列出所有可用的音频输入设备
pub fn list_input_devices() -> Result<Vec<String>, String> {
    let devices = get_host()
        .input_devices()
        .map_err(|e| format!("枚举输入设备失败: {e}"))?;
    Ok(devices.filter_map(|d| d.name().ok()).collect())
}
/// 列出所有可用的音频输出设备
pub fn list_output_devices() -> Result<Vec<String>, String> {
    let devices = get_host()
        .output_devices()
        .map_err(|e| format!("枚举输出设备失败: {e}"))?;
    Ok(devices.filter_map(|d| d.name().ok()).collect())
}
/// 根据名称查找音频输入设备
pub fn find_input_device_by_name(device_name: &str) -> Result<Device, String> {
    let devices: Vec<_> = get_host()
        .input_devices()
        .map_err(|e| format!("枚举输入设备失败: {e}"))?
        .collect();
    devices
        .into_iter()
        .find(|device| device.name().map(|n| n == device_name).unwrap_or(false))
        .ok_or_else(|| format!("未找到设备: {device_name}"))
}
/// 根据名称查找音频输出设备
pub fn find_output_device_by_name(device_name: &str) -> Result<Device, String> {
    let devices: Vec<_> = get_host()
        .output_devices()
        .map_err(|e| format!("枚举输出设备失败: {e}"))?
        .collect();
    devices
        .into_iter()
        .find(|device| device.name().map(|n| n == device_name).unwrap_or(false))
        .ok_or_else(|| format!("未找到设备: {device_name}"))
}
/// 获取输入设备的默认配置
pub fn default_input_config(device: &Device) -> Result<SupportedStreamConfig, String> {
    device
        .default_input_config()
        .map_err(|e| format!("获取默认输入配置失败: {e}"))
}
/// 获取输出设备的默认配置
pub fn default_output_config(device: &Device) -> Result<SupportedStreamConfig, String> {
    device
        .default_output_config()
        .map_err(|e| format!("获取默认输出配置失败: {e}"))
}
