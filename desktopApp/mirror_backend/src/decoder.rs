use crate::receiver::log_event;
use crate::write_frame_to_obs;
use concurrent_queue::ConcurrentQueue;
use ffmpeg::codec::{decoder, packet};
use ffmpeg::software::scaling::{context::Context, flag};
use ffmpeg::util::format::pixel::Pixel;
use ffmpeg_next as ffmpeg;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

// Global metrics
static FRAME_COUNTER: AtomicU64 = AtomicU64::new(0);
pub static TOTAL_DROPPED_FRAMES: AtomicU64 = AtomicU64::new(0);
pub static MINUTE_DROPPED_FRAMES: AtomicU64 = AtomicU64::new(0);

pub struct H265Decoder {
    decoder: decoder::Video,
    scaler: Option<Context>,
    bgra_frame: Option<ffmpeg::util::frame::Video>,
    consecutive_errors: u32,
    is_software: bool,
}

impl H265Decoder {
    pub fn new() -> Result<Self, ffmpeg::Error> {
        Self::new_internal(false)
    }

    fn new_internal(force_sw: bool) -> Result<Self, ffmpeg::Error> {
        ffmpeg::init()?;

        let codec = if force_sw {
            ffmpeg::decoder::find(ffmpeg::codec::Id::HEVC)
                .ok_or(ffmpeg::Error::DecoderNotFound)?
        } else {
            // Attempt to find a hardware-accelerated decoder first
            ffmpeg::decoder::find_by_name("hevc_videotoolbox") // macOS
                .or_else(|| ffmpeg::decoder::find_by_name("hevc_qsv"))     // Intel
                .or_else(|| ffmpeg::decoder::find_by_name("hevc_cuvid"))   // NVIDIA
                .or_else(|| ffmpeg::decoder::find_by_name("hevc_d3d11va")) // Windows
                .or_else(|| ffmpeg::decoder::find_by_name("hevc_vaapi"))   // Linux
                .or_else(|| ffmpeg::decoder::find(ffmpeg::codec::Id::HEVC))
                .ok_or(ffmpeg::Error::DecoderNotFound)?
        };

        log_event(
            "INFO",
            "DECODER",
            "init",
            &format!("Initializing {} decoder: {}", if force_sw { "SOFTWARE" } else { "HARDWARE" }, codec.name()),
        );

        let context = ffmpeg::codec::context::Context::new_with_codec(codec);
        let decoder = context.decoder().video()?;

        Ok(H265Decoder {
            decoder,
            scaler: None,
            bgra_frame: None,
            consecutive_errors: 0,
            is_software: force_sw,
        })
    }

    pub fn decode_and_push(&mut self, data: &[u8], timestamp: u64) -> Result<(), ffmpeg::Error> {
        let mut packet = packet::Packet::new(data.len());
        if let Some(pdata) = packet.data_mut() {
            pdata.copy_from_slice(data);
        }
        packet.set_pts(Some(timestamp as i64));

        let send_result = self.decoder.send_packet(&packet);
        
        if let Err(e) = send_result {
            match e {
                ffmpeg::Error::Other { errno: libc::EAGAIN } => {
                    // Buffer full, must receive frames. Not an error yet.
                }
                _ => {
                    self.consecutive_errors += 1;
                    log_event("ERROR", "DECODER", "pipeline", &format!("Packet send failed: {} (Code: {})", e, e));
                    
                    // Fallback to software if HW decoder fails 5 times in a row
                    if !self.is_software && self.consecutive_errors > 5 {
                        log_event("WARN", "DECODER", "fallback", "HW Decoder failing repeatedly. Switching to Software fallback...");
                        if let Ok(new_decoder) = Self::new_internal(true) {
                            *self = new_decoder;
                        }
                    }
                    return Err(e);
                }
            }
        } else {
            self.consecutive_errors = 0;
        }

        let mut frame = ffmpeg::util::frame::Video::empty();
        while self.decoder.receive_frame(&mut frame).is_ok() {
            let width = frame.width();
            let height = frame.height();
            let format = frame.format();

            if self.scaler.is_none()
                || self.bgra_frame.as_ref().map_or(true, |f| f.width() != width || f.height() != height)
            {
                let scaler = Context::get(
                    format,
                    width,
                    height,
                    Pixel::BGRA,
                    width,
                    height,
                    flag::Flags::BILINEAR,
                ).map_err(|_| ffmpeg::Error::InvalidData)?;

                self.scaler = Some(scaler);
                self.bgra_frame = Some(ffmpeg::util::frame::Video::new(Pixel::BGRA, width, height));
            }

            if let (Some(scaler), Some(bgra_frame)) = (self.scaler.as_mut(), self.bgra_frame.as_mut()) {
                if scaler.run(&frame, bgra_frame).is_ok() {
                    let frame_ts = frame.pts().unwrap_or(timestamp as i64) as u64;
                    let width_u32 = bgra_frame.width();
                    let height_u32 = bgra_frame.height();
                    let width = width_u32 as usize;
                    let height = height_u32 as usize;
                    let stride = bgra_frame.stride(0);
                    let data = bgra_frame.data(0);

                    // Acquire buffer from pool for preview
                    let mut buffer = crate::renderer::FREE_QUEUE
                        .pop()
                        .unwrap_or_else(|_| Vec::with_capacity(width * height * 4));
                    buffer.clear();

                    if stride == width * 4 {
                        buffer.extend_from_slice(&data[..width * height * 4]);
                    } else {
                        for y in 0..height {
                            let start = y * stride;
                            let end = start + (width * 4);
                            buffer.extend_from_slice(&data[start..end]);
                        }
                    }

                    // Log metrics
                    if let Ok(mut m) = crate::metrics::METRICS.lock() {
                        // Estimate pipeline latency (simplified)
                        let latency = 10; // Mock for now
                        m.record_frame(buffer.len(), latency);
                    }

                    // Write to OBS
                    unsafe {
                        crate::write_frame_to_obs(
                            buffer.as_ptr(),
                            buffer.len(),
                            width_u32,
                            height_u32,
                            frame_ts,
                            0,
                        );
                    }

                    // Push to Preview
                    crate::renderer::update_preview_window(buffer, width, height, 0);
                }
            }
        }

        Ok(())
    }
}

pub fn start_decoder_thread(queue: Arc<ConcurrentQueue<Vec<u8>>>) {
    std::thread::spawn(move || {
        log_event("INFO", "SYSTEM", "decoder", "FFmpeg Decoder Thread Started");
        let mut decoder = match H265Decoder::new() {
            Ok(d) => d,
            Err(e) => {
                log_event(
                    "FATAL",
                    "DECODER",
                    "init",
                    &format!("Failed to initialize decoder: {:?}", e),
                );
                return;
            }
        };

        loop {
            if crate::TERMINATION_SIGNAL.load(Ordering::Relaxed) {
                log_event("INFO", "SYSTEM", "decoder", "Decoder thread receiving termination signal.");
                break;
            }

            let mut current_packet = None;

            // Jitter Buffer: Drop old packets to reduce latency if backlog builds up.
            // With bounded(20), we allow some breathing room but clear if we hit 15.
            let queue_len = queue.len();
            if queue_len > 15 {
                let mut dropped = 0;
                // Jitter Buffer: Drop old packets to reduce latency.
                // We try to find the next Keyframe (I-frame) to avoid smearing.
                // HEVC NAL units start with 00 00 01 or 00 00 00 01.
                // The NAL unit type for HEVC IDR is usually 19 or 20.
                while queue.len() > 5 {
                    if let Ok(pkt) = queue.pop() {
                        dropped += 1;
                        // Check if this packet is a keyframe. 
                        // In our framed protocol, the 9-byte transport header is stripped,
                        // so the packet starts with the raw HEVC bitstream (Annex B).
                        // For Annex B, the NAL header starts at index 4 (after 00 00 00 01).
                        if pkt.len() > 6 {
                            let nal_type = (pkt[4] >> 1) & 0x3F;
                            if nal_type == 19 || nal_type == 20 {
                                // Found a keyframe! Stop dropping and keep this one.
                                dropped -= 1; // Do not count the keyframe as dropped
                                current_packet = Some(pkt);
                                break;
                            }
                        }
                    } else {
                        break;
                    }
                }
                
                TOTAL_DROPPED_FRAMES.fetch_add(dropped, Ordering::Relaxed);
                MINUTE_DROPPED_FRAMES.fetch_add(dropped, Ordering::Relaxed);
                if dropped > 0 {
                    log_event(
                        "WARN",
                        "DECODER",
                        "jitter",
                        &format!("Dropped {} packets (GOP-aware) to reduce latency", dropped),
                    );
                }
            }

            let packet_to_decode = current_packet.or_else(|| queue.pop().ok());

            if let Some(packet_data) = packet_to_decode {
                let timestamp = FRAME_COUNTER.fetch_add(1, Ordering::Relaxed);
                if let Err(_) = decoder.decode_and_push(&packet_data, timestamp) {
                    // Errors are logged inside decode_and_push
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    });

    // Thread to reset per-minute dropped frames
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        MINUTE_DROPPED_FRAMES.store(0, Ordering::Relaxed);
    });
}
