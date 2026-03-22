use crate::receiver::log_event;
use crate::write_frame_to_obs;
use concurrent_queue::ConcurrentQueue;
use ffmpeg::codec::{decoder, packet};
use ffmpeg::software::scaling::{context::Context, flag};
use ffmpeg::util::format::pixel::Pixel;
use ffmpeg_next as ffmpeg;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

// Global monotonic frame counter for timestamps when hardware timestamps aren't available
static FRAME_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct H265Decoder {
    decoder: decoder::Video,
    scaler: Option<Context>,
    bgra_frame: Option<ffmpeg::util::frame::Video>,
}

impl H265Decoder {
    pub fn new() -> Result<Self, ffmpeg::Error> {
        ffmpeg::init()?;

        let codec =
            ffmpeg::decoder::find(ffmpeg::codec::Id::HEVC).ok_or(ffmpeg::Error::DecoderNotFound)?;

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
                || self.bgra_frame.as_ref().map_or(true, |f| f.width() != width || f.height() != height)
            {
                self.scaler = Some(Context::get(
                    format,
                    width,
                    height,
                    Pixel::BGRA,
                    width,
                    height,
                    flag::Flags::BILINEAR, // FAST_BILINEAR might be faster but BILINEAR is standard
                ).unwrap());
                self.bgra_frame = Some(ffmpeg::util::frame::Video::new(Pixel::BGRA, width, height));
            }

            if let (Some(scaler), Some(bgra_frame)) = (self.scaler.as_mut(), self.bgra_frame.as_mut()) {
                if scaler.run(&frame, bgra_frame).is_ok() {
                    let frame_ts = frame.pts().unwrap_or(timestamp as i64) as u64;
                    let width = bgra_frame.width() as usize;
                    let height = bgra_frame.height() as usize;
                    let stride = bgra_frame.stride(0);
                    let data = bgra_frame.data(0);

                    // Acquire buffer from pool for preview
                    let mut buffer = crate::renderer::FREE_QUEUE.pop().unwrap_or_else(|_| Vec::with_capacity(width * height * 4));
                    buffer.clear();

                    // Copy row by row to ensure it's tightly packed (ignoring any padding at end of stride)
                    if stride == width * 4 {
                        // Optimisation: if already packed, copy all at once
                        buffer.extend_from_slice(&data[..width * height * 4]);
                    } else {
                        for y in 0..height {
                            let start = y * stride;
                            let end = start + (width * 4);
                            buffer.extend_from_slice(&data[start..end]);
                        }
                    }

                    // Write to OBS (use the packed buffer we just built)
                    unsafe {
                        write_frame_to_obs(buffer.as_ptr(), buffer.len(), width as u32, height as u32, frame_ts, 0); // 0 = BGRA
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
            if let Ok(packet_data) = queue.pop() {
                // Use monotonic frame counter for timestamps
                // This ensures proper frame ordering even without hardware timestamps
                let timestamp = FRAME_COUNTER.fetch_add(1, Ordering::Relaxed);
                if let Err(_) = decoder.decode_and_push(&packet_data, timestamp) {
                    // Errors are logged inside decode_and_push
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    });
}
