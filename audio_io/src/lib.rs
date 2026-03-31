//! 音频I/O模块
//!
//! 提供音频采集、播放、流管理和会话编排功能。
//!
//! 会话层会把麦克风原始 PCM 帧按 AEC → NS 的顺序处理后再送往播放端，并以播放侧参考帧驱动 AEC。
// 声明子模块
pub mod builder;
pub mod capture;
pub mod mixer;
pub mod playback;
pub mod session;
pub mod stream;
pub mod traits;
pub mod utils;
// 重新导出主要结构和功能
pub use builder::{AudioCaptureBuilder, AudioPlaybackBuilder};
pub use capture::AudioCapture;
pub use mixer::AudioMixerSource;
pub use playback::AudioPlayback;
pub use session::{
    AecConfig, AudioPlaybackTrackHandle, AudioProcessingPipelineConfig, AudioSession,
    AudioSessionBuilder, AudioSessionHandle, NsConfig,
};
pub use traits::{AudioCaptureControl, AudioPlaybackControl};
