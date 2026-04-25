use crate::receiver::log_event;
use concurrent_queue::ConcurrentQueue;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

static AUDIO_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static AUDIO_MUTED: AtomicBool = AtomicBool::new(true);

pub static AUDIO_QUEUE: Lazy<Arc<ConcurrentQueue<Vec<f32>>>> =
    Lazy::new(|| Arc::new(ConcurrentQueue::bounded(50))); // Holds up to 50 audio packets

pub fn start_audio_stream() {
    if AUDIO_ACTIVE.swap(true, Ordering::Relaxed) {
        return;
    }

    log_event("INFO", "AUDIO", "cpal", "Starting audio playback stream...");

    let queue = AUDIO_QUEUE.clone();

    std::thread::spawn(move || {
        let host = cpal::default_host();
        let device = match host.default_output_device() {
            Some(d) => d,
            None => {
                log_event("ERROR", "AUDIO", "cpal", "No output audio device found.");
                AUDIO_ACTIVE.store(false, Ordering::Relaxed);
                return;
            }
        };

        let config: cpal::StreamConfig = match device.default_output_config() {
            Ok(c) => c.into(),
            Err(e) => {
                log_event("ERROR", "AUDIO", "cpal", &format!("Failed to get default output config: {}", e));
                AUDIO_ACTIVE.store(false, Ordering::Relaxed);
                return;
            }
        };

        let channels = config.channels as usize;
        let mut leftover: Vec<f32> = Vec::new();
        let mut leftover_idx = 0;

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    for frame in data.chunks_mut(channels) {
                        // Refill from queue if empty
                        if leftover_idx >= leftover.len() {
                            if let Ok(new_packet) = queue.pop() {
                                leftover = new_packet;
                                leftover_idx = 0;
                            }
                        }

                        // Get sample or 0.0 (silence)
                        let sample = if leftover_idx < leftover.len() {
                            let s = leftover[leftover_idx];
                            leftover_idx += 1;
                            s
                        } else {
                            0.0
                        };

                        let final_sample = if AUDIO_MUTED.load(Ordering::Relaxed) {
                            0.0
                        } else {
                            sample
                        };

                        // Write to all channels (mono to stereo/surround)
                        for out_sample in frame.iter_mut() {
                            *out_sample = final_sample;
                        }
                    }
                },
                |err| {
                    log_event("ERROR", "AUDIO", "cpal", &format!("Stream error: {}", err));
                },
                None,
            )
            .unwrap();

        if let Err(e) = stream.play() {
            log_event("ERROR", "AUDIO", "cpal", &format!("Failed to play: {}", e));
            AUDIO_ACTIVE.store(false, Ordering::Relaxed);
            return;
        }

        log_event("SUCCESS", "AUDIO", "cpal", "Audio playback started");

        while AUDIO_ACTIVE.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        log_event("INFO", "AUDIO", "cpal", "Audio playback stopped");
    });
}

pub fn stop_audio_stream() {
    AUDIO_ACTIVE.store(false, Ordering::Relaxed);
}

pub fn push_audio(data: &[u8]) {
    if !AUDIO_ACTIVE.load(Ordering::Relaxed) {
        start_audio_stream();
    }

    if data.len() % 4 != 0 {
        return;
    }

    let floats: Vec<f32> = data
        .chunks_exact(4)
        .map(|chunk| {
            let mut arr = [0u8; 4];
            arr.copy_from_slice(chunk);
            f32::from_le_bytes(arr)
        })
        .collect();

    // Push to OBS shared memory feed
    crate::obs_feed::write_audio(&floats);

    // Push to local playback queue, dropping the oldest packet if full to prevent unbounded latency
    if AUDIO_QUEUE.push(floats).is_err() {
        let _ = AUDIO_QUEUE.pop(); // drop oldest
    }
}


