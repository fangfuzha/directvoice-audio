//! 音频会话编排层
//!
//! 这层包装在 `audio_io` 内部组合现有采集和播放对象，并把麦克风原始 PCM 帧交给流水线处理：
//!
//! 麦克风原始 PCM 帧 -> AEC 回声消除 -> NS 降噪 -> 播放/上层输出。
//!
//! 这里的 AEC 依赖即将送入播放端的参考帧，所以会话层必须同时持有采集和播放实例。
//! 默认情况下不会把采集帧自动回环到播放端，需要由应用层显式开启。
mod pipeline;
pub use pipeline::{AecConfig, AudioProcessingPipelineConfig, NsConfig};
use crate::{
    AudioCapture, AudioCaptureBuilder, AudioCaptureControl, AudioPlayback, AudioPlaybackBuilder,
    AudioPlaybackControl,
};
use pipeline::process_audio_bridge;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
const EXPOSED_CHANNEL_CAPACITY: usize = 8;
/// 会话构建器。
pub struct AudioSessionBuilder {
    capture_builder: AudioCaptureBuilder,
    playback_builder: AudioPlaybackBuilder,
    pipeline: AudioProcessingPipelineConfig,
    expose_raw_capture: bool,
    expose_processed_capture: bool,
    auto_loopback_enabled: bool,
}
impl AudioSessionBuilder {
    /// 创建新的会话构建器。
    pub fn new() -> Self {
        Self {
            capture_builder: AudioCapture::builder(),
            playback_builder: AudioPlayback::builder(),
            pipeline: AudioProcessingPipelineConfig::default(),
            expose_raw_capture: false,
            expose_processed_capture: true,
            auto_loopback_enabled: false,
        }
    }
    /// 替换采集构建器。
    pub fn capture_builder(mut self, builder: AudioCaptureBuilder) -> Self {
        self.capture_builder = builder;
        self
    }
    /// 替换播放构建器。
    pub fn playback_builder(mut self, builder: AudioPlaybackBuilder) -> Self {
        self.playback_builder = builder;
        self
    }
    /// 直接设置完整的处理流水线配置。
    pub fn processing(mut self, pipeline: AudioProcessingPipelineConfig) -> Self {
        self.pipeline = pipeline;
        self
    }
    /// 设置 AEC 配置。
    pub fn aec(mut self, config: AecConfig) -> Self {
        self.pipeline.aec = config;
        self
    }
    /// 设置 NS 配置。
    pub fn ns(mut self, config: NsConfig) -> Self {
        self.pipeline.ns = config;
        self
    }
    /// 设置 AEC 开关状态。
    pub fn set_aec_status(mut self, enabled: bool) -> Self {
        self.pipeline.aec.enabled = enabled;
        self
    }
    /// 设置 NS 开关状态。
    pub fn set_ns_status(mut self, enabled: bool) -> Self {
        self.pipeline.ns.enabled = enabled;
        self
    }
    /// 控制是否暴露原始采集帧。
    pub fn expose_raw_capture(mut self, enabled: bool) -> Self {
        self.expose_raw_capture = enabled;
        self
    }
    /// 控制是否暴露处理后的采集帧。
    pub fn expose_processed_capture(mut self, enabled: bool) -> Self {
        self.expose_processed_capture = enabled;
        self
    }
    /// 控制是否自动将处理后的采集帧回环到播放端。
    pub fn enable_auto_loopback(mut self, enabled: bool) -> Self {
        self.auto_loopback_enabled = enabled;
        self
    }
    /// 构建音频会话。
    pub fn build(self) -> Result<AudioSession, String> {
        let capture = self.capture_builder.build()?;
        let playback = self.playback_builder.build()?;
        Ok(AudioSession {
            inner: Arc::new(Mutex::new(AudioSessionInner {
                capture,
                playback,
                pipeline: Arc::new(Mutex::new(self.pipeline)),
                playback_sources: HashMap::new(),
                expose_raw_capture: self.expose_raw_capture,
                expose_processed_capture: self.expose_processed_capture,
                auto_loopback_enabled: Arc::new(AtomicBool::new(self.auto_loopback_enabled)),
                state: SessionState::Stopped,
                bridge_task: None,
                raw_tx: None,
                processed_tx: None,
                playback_tx: None,
            })),
        })
    }
}
impl Default for AudioSessionBuilder {
    fn default() -> Self {
        Self::new()
    }
}
/// 音频会话实例。
#[derive(Clone)]
pub struct AudioSession {
    inner: Arc<Mutex<AudioSessionInner>>,
}
/// 音频会话启动后的句柄。
#[derive(Clone)]
pub struct AudioSessionHandle {
    inner: Arc<Mutex<AudioSessionInner>>,
}

/// 会话层播放轨道句柄。
#[derive(Clone)]
pub struct AudioPlaybackTrackHandle {
    session: Arc<Mutex<AudioSessionInner>>,
    source_id: usize,
}

impl AudioPlaybackTrackHandle {
    /// 获取轨道 ID。
    pub fn source_id(&self) -> usize {
        self.source_id
    }

    fn source(&self) -> Result<crate::mixer::AudioMixerSource, String> {
        let inner = self
            .session
            .lock()
            .map_err(|_| "获取会话锁失败".to_string())?;
        inner
            .playback_sources
            .get(&self.source_id)
            .cloned()
            .ok_or_else(|| format!("未找到播放轨道: {}", self.source_id))
    }

    /// 异步发送一帧音频到该轨道。
    pub async fn send(&self, frame: Vec<f32>) -> Result<(), String> {
        self.source()?.send(frame).await
    }

    /// 尝试立即发送一帧音频到该轨道。
    pub fn try_send(&self, frame: Vec<f32>) -> Result<(), String> {
        self.source()?.try_send(frame)
    }

    /// 设置轨道音量。
    pub async fn set_volume(&self, volume: f32) -> Result<(), String> {
        self.source()?.set_volume(volume).await
    }

    /// 尝试立即设置轨道音量。
    pub fn try_set_volume(&self, volume: f32) -> Result<(), String> {
        self.source()?.try_set_volume(volume)
    }

    /// 获取轨道音量。
    pub fn get_volume(&self) -> Result<f32, String> {
        self.source()?.get_volume()
    }

    /// 设置轨道静音状态。
    pub async fn set_muted(&self, muted: bool) -> Result<(), String> {
        self.source()?.set_muted(muted).await
    }

    /// 尝试立即设置轨道静音状态。
    pub fn try_set_muted(&self, muted: bool) -> Result<(), String> {
        self.source()?.try_set_muted(muted)
    }

    /// 获取轨道静音状态。
    pub fn is_muted(&self) -> Result<bool, String> {
        self.source()?.is_muted()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionState {
    Stopped,
    Running,
}
struct AudioSessionInner {
    capture: AudioCapture,
    playback: AudioPlayback,
    pipeline: Arc<Mutex<AudioProcessingPipelineConfig>>,
    playback_sources: HashMap<usize, crate::mixer::AudioMixerSource>,
    expose_raw_capture: bool,
    expose_processed_capture: bool,
    auto_loopback_enabled: Arc<AtomicBool>,
    state: SessionState,
    bridge_task: Option<JoinHandle<()>>,
    raw_tx: Option<broadcast::Sender<Vec<f32>>>,
    processed_tx: Option<broadcast::Sender<Vec<f32>>>,
    playback_tx: Option<broadcast::Sender<Vec<f32>>>,
}
impl AudioSession {
    /// 创建新的会话构建器。
    pub fn builder() -> AudioSessionBuilder {
        AudioSessionBuilder::new()
    }
    /// 启动会话：先启动采集和播放，再用桥接任务串起 AEC → NS。
    pub fn start(&self) -> Result<AudioSessionHandle, String> {
        let shared = Arc::clone(&self.inner);
        let (
            capture_receiver,
            playback_sender,
            playback_reference_receiver,
            raw_tx,
            processed_tx,
            pipeline,
            sample_rate,
            auto_loopback_enabled,
        ) = {
            let mut inner = shared
                .lock()
                .map_err(|_| "音频会话状态锁已损坏".to_string())?;
            if inner.state == SessionState::Running {
                return Err("音频会话已经在运行".to_string());
            }
            let capture_receiver = inner.capture.start()?;
            let playback_sender = match inner.playback.start() {
                Ok(sender) => sender,
                Err(error) => {
                    let _ = inner.capture.stop();
                    return Err(error);
                }
            };
            let raw_tx = if inner.expose_raw_capture {
                Some(broadcast::channel(EXPOSED_CHANNEL_CAPACITY).0)
            } else {
                None
            };
            let processed_tx = if inner.expose_processed_capture {
                Some(broadcast::channel(EXPOSED_CHANNEL_CAPACITY).0)
            } else {
                None
            };
            let pipeline = Arc::clone(&inner.pipeline);
            let sample_rate = inner.capture.settings.target_sample_rate;
            inner.state = SessionState::Running;
            inner.raw_tx = raw_tx.clone();
            inner.processed_tx = processed_tx.clone();
            inner.playback_tx = Some(playback_sender.clone());
            let playback_reference_receiver = playback_sender.subscribe();
            (
                capture_receiver,
                playback_sender,
                playback_reference_receiver,
                raw_tx,
                processed_tx,
                pipeline,
                sample_rate,
                Arc::clone(&inner.auto_loopback_enabled),
            )
        };
        let bridge_task = tokio::spawn(process_audio_bridge(
            capture_receiver,
            playback_sender,
            playback_reference_receiver,
            raw_tx,
            processed_tx,
            pipeline,
            sample_rate,
            auto_loopback_enabled,
        ));
        let mut inner = shared
            .lock()
            .map_err(|_| "音频会话状态锁已损坏".to_string())?;
        inner.bridge_task = Some(bridge_task);
        drop(inner);
        Ok(AudioSessionHandle { inner: shared })
    }
    /// 停止会话。
    pub fn stop(&self) -> bool {
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return false,
        };
        inner.stop_locked()
    }
    /// 判断会话是否正在运行。
    pub fn is_running(&self) -> bool {
        self.inner
            .lock()
            .map(|inner| inner.state == SessionState::Running)
            .unwrap_or(false)
    }
    fn with_pipeline<R>(&self, updater: impl FnOnce(&mut AudioProcessingPipelineConfig) -> R) -> Option<R> {
        let inner = self.inner.lock().ok()?;
        let mut pipeline = inner.pipeline.lock().ok()?;
        Some(updater(&mut pipeline))
    }
    fn read_pipeline<R>(&self, reader: impl FnOnce(&AudioProcessingPipelineConfig) -> R) -> Option<R> {
        let inner = self.inner.lock().ok()?;
        let pipeline = inner.pipeline.lock().ok()?;
        Some(reader(&pipeline))
    }
    /// 设置 AEC 开关状态。
    pub fn set_aec_status(&self, enabled: bool) {
        let _ = self.with_pipeline(|pipeline| pipeline.aec.enabled = enabled);
    }
    /// 设置 NS 开关状态。
    pub fn set_ns_status(&self, enabled: bool) {
        let _ = self.with_pipeline(|pipeline| pipeline.ns.enabled = enabled);
    }
    /// 设置 NS 降噪强度。
    pub fn set_ns_strength(&self, strength: f32) {
        let _ = self.with_pipeline(|pipeline| pipeline.ns.strength = strength.clamp(0.0, 1.0));
    }
    /// 获取当前 NS 降噪强度。
    pub fn ns_strength(&self) -> Option<f32> {
        self.read_pipeline(|pipeline| pipeline.ns.strength)
    }
    /// 直接替换完整的处理配置。
    pub fn set_processing_config(&self, config: AudioProcessingPipelineConfig) {
        self.update_pipeline(|pipeline| *pipeline = config);
    }
    /// 替换 AEC 配置。
    pub fn set_aec_config(&self, config: AecConfig) {
        self.update_pipeline(|pipeline| pipeline.aec = config);
    }
    /// 替换 NS 配置。
    pub fn set_ns_config(&self, config: NsConfig) {
        self.update_pipeline(|pipeline| pipeline.ns = config);
    }
    /// 设置是否自动回环到播放端。
    pub fn set_auto_loopback_enabled(&self, enabled: bool) {
        if let Ok(inner) = self.inner.lock() {
            inner
                .auto_loopback_enabled
                .store(enabled, Ordering::Relaxed);
        }
    }
    /// 获取是否启用了自动回环。
    pub fn auto_loopback_enabled(&self) -> bool {
        self.inner
            .lock()
            .map(|inner| inner.auto_loopback_enabled.load(Ordering::Relaxed))
            .unwrap_or(false)
    }
    /// 订阅原始采集帧。
    pub fn subscribe_raw_capture(&self) -> Option<broadcast::Receiver<Vec<f32>>> {
        let inner = self.inner.lock().ok()?;
        inner.raw_tx.as_ref().map(|sender| sender.subscribe())
    }
    /// 订阅处理后的采集帧。
    pub fn subscribe_processed_capture(&self) -> Option<broadcast::Receiver<Vec<f32>>> {
        let inner = self.inner.lock().ok()?;
        inner.processed_tx.as_ref().map(|sender| sender.subscribe())
    }
    /// 获取播放侧发送端。

    /// 设置全局播放音量 (0.0 - 1.0)
    pub fn set_playback_volume(&self, volume: f32) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.playback.set_volume(volume);
        }
    }

    /// 获取全局播放音量
    pub fn get_playback_volume(&self) -> f32 {
        if let Ok(inner) = self.inner.lock() {
            inner.playback.get_volume()
        } else {
            1.0
        }
    }

    /// 设置全局静音状态
    pub fn set_playback_mute(&self, mute: bool) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.playback.set_mute(mute);
        }
    }

    /// 获取全局静音状态
    pub fn is_playback_muted(&self) -> bool {
        if let Ok(inner) = self.inner.lock() {
            inner.playback.is_muted()
        } else {
            false
        }
    }

    /// 创建一个独立的播放轨道句柄，可单独控制音量和静音。
    pub fn create_playback_track(&self) -> Result<AudioPlaybackTrackHandle, String> {
        let mut inner = self.inner.lock().map_err(|_| "获取会话锁失败".to_string())?;
        let source = inner.playback.create_mixer_source()?;
        let source_id = source.source_id();
        inner.playback_sources.insert(source_id, source);
        Ok(AudioPlaybackTrackHandle {
            session: Arc::clone(&self.inner),
            source_id,
        })
    }

    /// 根据轨道 ID 获取已注册的播放轨道句柄。
    pub fn playback_track(&self, source_id: usize) -> Option<AudioPlaybackTrackHandle> {
        let inner = self.inner.lock().ok()?;
        inner.playback_sources.get(&source_id)?;
        Some(AudioPlaybackTrackHandle {
            session: Arc::clone(&self.inner),
            source_id,
        })
    }

    /// 列出所有已注册的播放轨道 ID。
    pub fn playback_track_ids(&self) -> Vec<usize> {
        self.inner
            .lock()
            .map(|inner| inner.playback_sources.keys().copied().collect())
            .unwrap_or_default()
    }

    /// 设置指定播放轨道音量。
    pub fn set_playback_track_volume(&self, source_id: usize, volume: f32) -> Result<(), String> {
        let track = self
            .playback_track(source_id)
            .ok_or_else(|| format!("未找到播放轨道: {source_id}"))?;
        track.try_set_volume(volume)
    }

    /// 设置指定播放轨道静音状态。
    pub fn set_playback_track_mute(&self, source_id: usize, mute: bool) -> Result<(), String> {
        let track = self
            .playback_track(source_id)
            .ok_or_else(|| format!("未找到播放轨道: {source_id}"))?;
        track.try_set_muted(mute)
    }

    /// 批量设置所有播放轨道音量。
    pub fn set_all_playback_tracks_volume(&self, volume: f32) -> Result<(), String> {
        let tracks = self.collect_playback_tracks();
        for track in tracks {
            track.try_set_volume(volume)?;
        }
        Ok(())
    }

    /// 批量设置所有播放轨道静音状态。
    pub fn set_all_playback_tracks_mute(&self, mute: bool) -> Result<(), String> {
        let tracks = self.collect_playback_tracks();
        for track in tracks {
            track.try_set_muted(mute)?;
        }
        Ok(())
    }

    fn collect_playback_tracks(&self) -> Vec<AudioPlaybackTrackHandle> {
        let source_ids = self.playback_track_ids();
        source_ids
            .into_iter()
            .map(|source_id| AudioPlaybackTrackHandle {
                session: Arc::clone(&self.inner),
                source_id,
            })
            .collect()
    }

    /// 根据轨道 ID 获取已注册的播放轨道句柄。
    pub fn playback_source(&self, source_id: usize) -> Option<AudioPlaybackTrackHandle> {
        self.playback_track(source_id)
    }

    /// 列出所有已注册的播放轨道 ID。
    pub fn playback_source_ids(&self) -> Vec<usize> {
        self.playback_track_ids()
    }

    /// 设置指定播放轨道音量。
    pub fn set_playback_source_volume(&self, source_id: usize, volume: f32) -> Result<(), String> {
        self.set_playback_track_volume(source_id, volume)
    }

    /// 设置指定播放轨道静音状态。
    pub fn set_playback_source_mute(&self, source_id: usize, mute: bool) -> Result<(), String> {
        self.set_playback_track_mute(source_id, mute)
    }

    /// 批量设置所有播放轨道音量。
    pub fn set_all_playback_sources_volume(&self, volume: f32) -> Result<(), String> {
        self.set_all_playback_tracks_volume(volume)
    }

    /// 批量设置所有播放轨道静音状态。
    pub fn set_all_playback_sources_mute(&self, mute: bool) -> Result<(), String> {
        self.set_all_playback_tracks_mute(mute)
    }

    pub fn playback_sender(&self) -> Option<broadcast::Sender<Vec<f32>>> {
        let inner = self.inner.lock().ok()?;
        inner.playback_tx.as_ref().cloned()
    }
    fn update_pipeline<F>(&self, update: F)
    where
        F: FnOnce(&mut AudioProcessingPipelineConfig),
    {
        if let Ok(inner) = self.inner.lock() {
            if let Ok(mut pipeline) = inner.pipeline.lock() {
                update(&mut pipeline);
            }
        }
    }
}
impl AudioSessionHandle {
    /// 停止会话。
    pub fn stop(self) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "音频会话状态锁已损坏".to_string())?;
        inner.stop_locked();
        Ok(())
    }
    /// 判断会话是否正在运行。
    pub fn is_running(&self) -> bool {
        self.inner
            .lock()
            .map(|inner| inner.state == SessionState::Running)
            .unwrap_or(false)
    }
    fn with_pipeline<R>(&self, updater: impl FnOnce(&mut AudioProcessingPipelineConfig) -> R) -> Option<R> {
        let inner = self.inner.lock().ok()?;
        let mut pipeline = inner.pipeline.lock().ok()?;
        Some(updater(&mut pipeline))
    }
    fn read_pipeline<R>(&self, reader: impl FnOnce(&AudioProcessingPipelineConfig) -> R) -> Option<R> {
        let inner = self.inner.lock().ok()?;
        let pipeline = inner.pipeline.lock().ok()?;
        Some(reader(&pipeline))
    }
    /// 设置 AEC 开关状态。
    pub fn set_aec_status(&self, enabled: bool) {
        let _ = self.with_pipeline(|pipeline| pipeline.aec.enabled = enabled);
    }
    /// 设置 NS 开关状态。
    pub fn set_ns_status(&self, enabled: bool) {
        let _ = self.with_pipeline(|pipeline| pipeline.ns.enabled = enabled);
    }
    /// 设置 NS 降噪强度。
    pub fn set_ns_strength(&self, strength: f32) {
        let _ = self.with_pipeline(|pipeline| pipeline.ns.strength = strength.clamp(0.0, 1.0));
    }
    /// 获取当前 NS 降噪强度。
    pub fn ns_strength(&self) -> Option<f32> {
        self.read_pipeline(|pipeline| pipeline.ns.strength)
    }
    /// 订阅原始采集帧。
    pub fn subscribe_raw_capture(&self) -> Option<broadcast::Receiver<Vec<f32>>> {
        let inner = self.inner.lock().ok()?;
        inner.raw_tx.as_ref().map(|sender| sender.subscribe())
    }
    /// 订阅处理后的采集帧。
    pub fn subscribe_processed_capture(&self) -> Option<broadcast::Receiver<Vec<f32>>> {
        let inner = self.inner.lock().ok()?;
        inner.processed_tx.as_ref().map(|sender| sender.subscribe())
    }
    /// 获取播放侧发送端。

    /// 设置全局播放音量 (0.0 - 1.0)
    pub fn set_playback_volume(&self, volume: f32) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.playback.set_volume(volume);
        }
    }

    /// 获取全局播放音量
    pub fn get_playback_volume(&self) -> f32 {
        if let Ok(inner) = self.inner.lock() {
            inner.playback.get_volume()
        } else {
            1.0
        }
    }

    /// 设置全局静音状态
    pub fn set_playback_mute(&self, mute: bool) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.playback.set_mute(mute);
        }
    }

    /// 获取全局静音状态
    pub fn is_playback_muted(&self) -> bool {
        if let Ok(inner) = self.inner.lock() {
            inner.playback.is_muted()
        } else {
            false
        }
    }

    /// 创建一个独立的播放轨道句柄，可单独控制音量和静音。

    /// 创建一个独立的播放轨道句柄，可单独控制音量和静音。
    pub fn create_playback_track(&self) -> Result<AudioPlaybackTrackHandle, String> {
        let mut inner = self.inner.lock().map_err(|_| "获取会话锁失败".to_string())?;
        let source = inner.playback.create_mixer_source()?;
        let source_id = source.source_id();
        inner.playback_sources.insert(source_id, source);
        Ok(AudioPlaybackTrackHandle {
            session: Arc::clone(&self.inner),
            source_id,
        })
    }

    /// 根据轨道 ID 获取已注册的播放轨道句柄。

    /// 根据轨道 ID 获取已注册的播放轨道句柄。
    pub fn playback_track(&self, source_id: usize) -> Option<AudioPlaybackTrackHandle> {
        let inner = self.inner.lock().ok()?;
        inner.playback_sources.get(&source_id)?;
        Some(AudioPlaybackTrackHandle {
            session: Arc::clone(&self.inner),
            source_id,
        })
    }

    /// 列出所有已注册的播放轨道 ID。

    /// 列出所有已注册的播放轨道 ID。
    pub fn playback_track_ids(&self) -> Vec<usize> {
        self.inner
            .lock()
            .map(|inner| inner.playback_sources.keys().copied().collect())
            .unwrap_or_default()
    }

    /// 设置指定播放轨道音量。

    /// 设置指定播放轨道音量。
    pub fn set_playback_track_volume(&self, source_id: usize, volume: f32) -> Result<(), String> {
        let track = self
            .playback_track(source_id)
            .ok_or_else(|| format!("未找到播放轨道: {source_id}"))?;
        track.try_set_volume(volume)
    }

    /// 设置指定播放轨道静音状态。

    /// 设置指定播放轨道静音状态。
    pub fn set_playback_track_mute(&self, source_id: usize, mute: bool) -> Result<(), String> {
        let track = self
            .playback_track(source_id)
            .ok_or_else(|| format!("未找到播放轨道: {source_id}"))?;
        track.try_set_muted(mute)
    }

    /// 批量设置所有播放轨道音量。

    /// 批量设置所有播放轨道音量。
    pub fn set_all_playback_tracks_volume(&self, volume: f32) -> Result<(), String> {
        for track in self.collect_playback_tracks() {
            track.try_set_volume(volume)?;
        }
        Ok(())
    }

    /// 批量设置所有播放轨道静音状态。

    /// 批量设置所有播放轨道静音状态。
    pub fn set_all_playback_tracks_mute(&self, mute: bool) -> Result<(), String> {
        for track in self.collect_playback_tracks() {
            track.try_set_muted(mute)?;
        }
        Ok(())
    }

    fn collect_playback_tracks(&self) -> Vec<AudioPlaybackTrackHandle> {
        self.playback_track_ids()
            .into_iter()
            .map(|source_id| AudioPlaybackTrackHandle {
                session: Arc::clone(&self.inner),
                source_id,
            })
            .collect()
    }

    pub fn playback_sender(&self) -> Option<broadcast::Sender<Vec<f32>>> {
        let inner = self.inner.lock().ok()?;
        inner.playback_tx.as_ref().cloned()
    }
    /// 设置是否自动回环到播放端。
    pub fn set_auto_loopback_enabled(&self, enabled: bool) {
        if let Ok(inner) = self.inner.lock() {
            inner
                .auto_loopback_enabled
                .store(enabled, Ordering::Relaxed);
        }
    }
    /// 获取是否启用了自动回环。
    pub fn auto_loopback_enabled(&self) -> bool {
        self.inner
            .lock()
            .map(|inner| inner.auto_loopback_enabled.load(Ordering::Relaxed))
            .unwrap_or(false)
    }
}
impl AudioSessionInner {
    fn stop_locked(&mut self) -> bool {
        if self.state != SessionState::Running {
            return false;
        }
        if let Some(task) = self.bridge_task.take() {
            task.abort();
        }
        let _ = self.capture.stop();
        let _ = self.playback.stop();
        self.raw_tx = None;
        self.processed_tx = None;
        self.playback_tx = None;
        self.playback_sources.clear();
        self.state = SessionState::Stopped;
        true
    }
}
