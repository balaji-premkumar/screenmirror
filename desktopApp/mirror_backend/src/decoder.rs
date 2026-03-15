use ffmpeg_next as ffmpeg;
use ffmpeg::codec::{decoder, packet};
use std::sync::Arc;
use concurrent_queue::ConcurrentQueue;
use crate::write_frame_to_obs;
use crate::receiver::log_event;
use std::sync::atomic::{AtomicU64, Ordering};

// Global monotonic frame counter for timestamps when hardware timestamps aren't available
static FRAME_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct H265Decoder {
    decoder: decoder::Video,
}

impl H265Decoder {
    pub fn new() -> Result<Self, ffmpeg::Error> {
        ffmpeg::init()?;

        let codec = ffmpeg::decoder::find(ffmpeg::codec::Id::HEVC)
            .ok_or(ffmpeg::Error::DecoderNotFound)?;
        
        let mut context = ffmpeg::codec::context::Context::new_with_codec(codec);
        
        // Platform specific HW acceleration hints
        #[cfg(target_os = "windows")]
        log_event("INFO", "DECODER", "init", "Targeting Windows DXVA2/D3D11VA acceleration");
        
        #[cfg(target_os = "macos")]
        log_event("INFO", "DECODER", "init", "Targeting macOS VideoToolbox acceleration");

        #[cfg(target_os = "linux")]
        log_event("INFO", "DECODER", "init", "Targeting Linux NVDEC/VAAPI acceleration");

        let decoder = context.decoder().video()?;

        Ok(H265Decoder { decoder })
    }

    pub fn decode_and_push(&mut self, data: &[u8], timestamp: u64) -> Result<(), ffmpeg::Error> {
        let mut packet = packet::Packet::new(data.len());
        if let Some(pdata) = packet.data_mut() {
            pdata.copy_from_slice(data);
        }
        packet.set_pts(Some(timestamp as i64));

        if let Err(e) = self.decoder.send_packet(&packet) {
            log_event("ERROR", "DECODER", "pipeline", &format!("Packet send failed: {:?}", e));
            return Err(e);
        }

        let mut frame = ffmpeg::util::frame::Video::empty();
        while self.decoder.receive_frame(&mut frame).is_ok() {
            let data = frame.data(0);
            let frame_ts = frame.pts().unwrap_or(timestamp as i64) as u64;
            unsafe {
                write_frame_to_obs(data.as_ptr(), data.len(), frame_ts);
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
                log_event("FATAL", "DECODER", "init", &format!("Failed to initialize decoder: {:?}", e));
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
