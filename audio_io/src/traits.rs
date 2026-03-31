use tokio::sync::{broadcast, mpsc};
/// 音频采集控制接口
pub trait AudioCaptureControl: Send + Sync {
    /// 列出所有可用的音频输入设备名称
    fn list_devices(&self) -> Result<Vec<String>, String>;
    /// 启动音频采集,返回音频数据接收通道
    fn start(&mut self) -> Result<mpsc::Receiver<Vec<f32>>, String>;
    /// 停止音频采集
    fn stop(&mut self) -> bool;
    /// 检查是否正在采集
    fn is_capturing(&self) -> bool;
    /// 获取当前音频采集设备名称
    fn current_device_name(&self) -> String;
    /// 切换采集设备
    fn switch_device(&mut self, device_name: &str) -> Result<(), String>;
    /// 设置采集音量 (0.0 - 1.0)
    fn set_volume(&mut self, volume: f32);
    /// 获取采集音量 (0.0 - 1.0)
    fn get_volume(&self) -> f32;
    /// 设置静音状态
    fn set_mute(&mut self, mute: bool);
    /// 获取静音状态
    fn is_muted(&self) -> bool;
}
/// 音频播放控制接口
pub trait AudioPlaybackControl: Send + Sync {
    /// 列出所有可用的音频输出设备名称
    fn list_devices(&self) -> Result<Vec<String>, String>;
    /// 启动音频播放,返回音频数据发送通道
    fn start(&mut self) -> Result<broadcast::Sender<Vec<f32>>, String>;
    /// 停止音频播放
    fn stop(&mut self) -> bool;
    /// 检查是否正在播放
    fn is_playing(&self) -> bool;
    /// 获取当前播放设备名称
    fn current_device_name(&self) -> String;
    /// 切换播放设备
    fn switch_device(&mut self, device_name: &str) -> Result<(), String>;
    /// 设置播放音量 (0.0 - 1.0)
    fn set_volume(&mut self, volume: f32);
    /// 获取播放音量 (0.0 - 1.0)
    fn get_volume(&self) -> f32;
    /// 设置静音状态
    fn set_mute(&mut self, mute: bool);
    /// 获取静音状态
    fn is_muted(&self) -> bool;
}
