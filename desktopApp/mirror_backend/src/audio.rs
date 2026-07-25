//! Audio routing.
//!
//! The desktop app deliberately owns **no** audio output device. Sound from
//! the phone is forwarded only to sinks the user has explicitly turned on:
//!
//!   * the OBS shared-memory ring, while the OBS feed toggle is on;
//!   * the ffplay player, while a playback session is running.
//!
//! With both sinks off, audio is discarded. Nothing here ever opens ALSA,
//! WASAPI or CoreAudio — that was the old behaviour, where the first audio
//! packet silently started playback on the default output device.

/// The wire format is fixed: mono f32 little-endian at 48 kHz
/// (produced by AudioPlaybackCapture on the phone).
pub const SOURCE_SAMPLE_RATE: u32 = 48_000;

/// Route one demuxed audio packet to whichever sinks are active.
/// `data` is raw f32 LE mono PCM.
pub fn push_audio(data: &[u8]) {
    if data.len() < 4 || data.len() % 4 != 0 {
        return;
    }

    // ffplay receives the bytes untouched — it does its own conversion.
    crate::player::push_audio(data);

    // The OBS ring stores samples, so convert only when that sink is live.
    if crate::obs_feed::is_enabled() {
        let floats: Vec<f32> = data
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        crate::obs_feed::write_audio(&floats);
    }
}
