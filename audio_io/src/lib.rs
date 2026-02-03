//! 音频I/O模块
//!
//! 提供音频采集、播放和流管理功能

// 声明子模块
pub mod builder;
pub mod capture;
pub mod playback;
pub mod stream;
pub mod traits;
pub mod utils;

// 重新导出主要结构和功能
pub use builder::{AudioCaptureBuilder, AudioPlaybackBuilder};
pub use capture::AudioCapture;
pub use playback::AudioPlayback;
pub use traits::{AudioCaptureControl, AudioPlaybackControl};
