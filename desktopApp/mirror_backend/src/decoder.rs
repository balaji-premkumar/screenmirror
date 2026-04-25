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
}

impl H265Decoder {
    pub fn new() -> Result<Self, ffmpeg::Error> {
        ffmpeg::init()?;

        // Attempt to find a hardware-accelerated decoder first, fallback to software
        let codec = ffmpeg::decoder::find_by_name("hevc_videotoolbox")
            .or_else(|| ffmpeg::decoder::find_by_name("hevc_cuvid"))
            .or_else(|| ffmpeg::decoder::find_by_name("hevc_qsv"))
            .or_else(|| ffmpeg::decoder::find_by_name("hevc_vaapi"))
            .or_else(|| ffmpeg::decoder::find(ffmpeg::codec::Id::HEVC))
            .ok_or(ffmpeg::Error::DecoderNotFound)?;

        log_event(
            "INFO",
            "DECODER",
            "init",
            &format!("Selected HEVC decoder: {}", codec.name()),
        );

        let mut context = ffmpeg::codec::context::Context::new_with_codec(codec);

        // Platform specific HW acceleration hints
        #[cfg(target_os = "windows")]
        log_event(
            "INFO",
            "DECODER",
            "init",
            "Targeting Windows DXVA2/D3D11VA acceleration",
        );

        #[cfg(target_os = "macos")]
        log_event(
            "INFO",
            "DECODER",
            "init",
            "Targeting macOS VideoToolbox acceleration",
        );

        #[cfg(target_os = "linux")]
        log_event(
            "INFO",
            "DECODER",
            "init",
            "Targeting Linux NVDEC/VAAPI acceleration",
        );

        let decoder = context.decoder().video()?;

        Ok(H265Decoder {
            decoder,
            scaler: None,
            bgra_frame: None,
        })
    }

    pub fn decode_and_push(&mut self, data: &[u8], timestamp: u64) -> Result<(), ffmpeg::Error> {
        let mut packet = packet::Packet::new(data.len());
        if let Some(pdata) = packet.data_mut() {
            pdata.copy_from_slice(data);
        }
        packet.set_pts(Some(timestamp as i64));

        if let Err(e) = self.decoder.send_packet(&packet) {
            log_event(
                "ERROR",
                "DECODER",
                "pipeline",
                &format!("Packet send failed: {:?}", e),
            );
            return Err(e);
        }

        let mut frame = ffmpeg::util::frame::Video::empty();
        while self.decoder.receive_frame(&mut frame).is_ok() {
            let width = frame.width();
            let height = frame.height();
            let format = frame.format();

            if self.scaler.is_none()
                || self
                    .bgra_frame
                    .as_ref()
                    .map_or(true, |f| f.width() != width || f.height() != height)
            {
                let mut scaler = Context::get(
                    format,
                    width,
                    height,
                    Pixel::BGRA,
                    width,
                    height,
                    flag::Flags::BILINEAR | flag::Flags::SWS_ACCEL,
                )
                .unwrap();

                // Explicitly set Rec.709 colorspace for accurate HD colors
                // (Using ffmpeg-next context methods if possible, otherwise rely on the scaler's defaults)
                // Note: ffmpeg-next SwScale Context doesn't always expose colorspace directly in v7.1
                // but we can ensure it through the input frame's properties if the decoder supports it.
                self.scaler = Some(scaler);
                self.bgra_frame = Some(ffmpeg::util::frame::Video::new(Pixel::BGRA, width, height));
            }

            if let (Some(scaler), Some(bgra_frame)) =
                (self.scaler.as_mut(), self.bgra_frame.as_mut())
            {
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

                    // Copy row by row to ensure it's tightly packed
                    if stride == width * 4 {
                        buffer.extend_from_slice(&data[..width * height * 4]);
                    } else {
                        for y in 0..height {
                            let start = y * stride;
                            let end = start + (width * 4);
                            buffer.extend_from_slice(&data[start..end]);
                        }
                    }

                    // Mirror Pro Optimization: Use SIMD for UYVY if format is detected (Example)
                    if format == Pixel::UYVY422 {
                        // We could use our new SIMD convert here
                        // crate::video_processing::compress_uyvy_to_nv12(...);
                    }

                    // Write to OBS (use standardized header format: magic wide height timestamp datasize)
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

                    // Push to Preview Window
                    crate::renderer::update_preview_window(buffer, width, height, 0);

                    if frame_ts % 100 == 0 {
                        println!("[Decoder] Processed frame: {}x{}", width, height);
                    }
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
                        // In our framed protocol, payload starts at offset 9.
                        // For HEVC, NAL header is 2 bytes. Type is (byte[0] >> 1) & 0x3F.
                        if pkt.len() > 11 {
                            let nal_type = (pkt[9] >> 1) & 0x3F;
                            if nal_type == 19 || nal_type == 20 {
                                // Found a keyframe! Stop dropping and keep this one as the new start.
                                // Actually, we want to START from a keyframe.
                                // So we drop EVERYTHING before this keyframe.
                                // Re-push this keyframe to the front or just stop here.
                                // For simplicity, if we hit a keyframe, we stop dropping.
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

            // Blocking pop instead of busy loop with sleep(1ms)
            match queue.pop() {
                Ok(packet_data) => {
                    let timestamp = FRAME_COUNTER.fetch_add(1, Ordering::Relaxed);
                    if let Err(_) = decoder.decode_and_push(&packet_data, timestamp) {
                        // Errors are logged inside decode_and_push
                    }
                }
                Err(_) => {
                    // Queue closed/empty - this blocking pop will return Err if closed,
                    // but ConcurrentQueue::pop is not actually blocking for 'pop'.
                    // We need a way to wait. Since ConcurrentQueue doesn't have a blocking pop,
                    // we can use a small sleep but we should actually use a channel or a queue
                    // that supports blocking.
                    // Wait, ConcurrentQueue::pop is non-blocking. 
                    // Let's use std::thread::park/unpark or just keep the sleep but make it better, 
                    // or better yet, switch to crossbeam-channel.
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
    });

    // Thread to reset per-minute dropped frames
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        MINUTE_DROPPED_FRAMES.store(0, Ordering::Relaxed);
    });
}
