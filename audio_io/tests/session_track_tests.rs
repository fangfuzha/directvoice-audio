use audio_io::AudioSession;

#[test]
fn test_create_playback_track_registers_source() {
    let session = AudioSession::builder().build().unwrap();

    let track = session.create_playback_track().unwrap();
    let source_id = track.source_id();

    assert!(source_id > 0);
    assert_eq!(session.playback_track_ids(), vec![source_id]);

    let fetched = session.playback_track(source_id).expect("轨道应可按 ID 找到");
    assert_eq!(fetched.source_id(), source_id);
}

#[tokio::test]
async fn test_playback_track_individual_controls() {
    let session = AudioSession::builder().build().unwrap();
    let track = session.create_playback_track().unwrap();
    let source_id = track.source_id();

    assert_eq!(track.get_volume().unwrap(), 1.0);
    assert!(!track.is_muted().unwrap());

    track.set_volume(0.35).await.unwrap();
    assert_eq!(session.playback_track(source_id).unwrap().get_volume().unwrap(), 0.35);

    track.set_muted(true).await.unwrap();
    assert!(session.playback_track(source_id).unwrap().is_muted().unwrap());

    session.set_playback_track_volume(source_id, 0.6).unwrap();
    assert_eq!(track.get_volume().unwrap(), 0.6);

    session.set_playback_track_mute(source_id, false).unwrap();
    assert!(!track.is_muted().unwrap());
}

#[test]
fn test_playback_track_batch_controls() {
    let session = AudioSession::builder().build().unwrap();
    let first = session.create_playback_track().unwrap();
    let second = session.create_playback_track().unwrap();

    session.set_all_playback_tracks_volume(0.2).unwrap();
    assert_eq!(first.get_volume().unwrap(), 0.2);
    assert_eq!(second.get_volume().unwrap(), 0.2);

    session.set_all_playback_tracks_mute(true).unwrap();
    assert!(first.is_muted().unwrap());
    assert!(second.is_muted().unwrap());

    assert_eq!(session.playback_track_ids().len(), 2);
}

#[test]
fn test_playback_track_lookup_missing_id() {
    let session = AudioSession::builder().build().unwrap();

    assert!(session.playback_track(9_999).is_none());
    assert!(session.set_playback_track_volume(9_999, 0.5).is_err());
    assert!(session.set_playback_track_mute(9_999, true).is_err());
}