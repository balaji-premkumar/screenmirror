use crate::log_event;
use crate::pipeline::VideoQueue;
use ffmpeg::codec::{decoder, packet};
use ffmpeg::software::scaling::{context::Context as ScaleContext, flag};
use ffmpeg::util::format::pixel::Pixel;
use ffmpeg_next as ffmpeg;
use mirror_i18n::codes;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

// Global metrics
static FRAME_COUNTER: AtomicU64 = AtomicU64::new(0);
pub static TOTAL_DROPPED_FRAMES: AtomicU64 = AtomicU64::new(0);

/// The hardware pixel format negotiated for this session (as raw
/// AVPixelFormat value), or -1 when decoding in software.
static TARGET_HW_FMT: AtomicI32 = AtomicI32::new(-1);

/// Custom get_format callback: pick the negotiated hardware format when the
/// decoder offers it, otherwise fall back to the first (software) entry.
/// Without this callback libavcodec always picks a software format, even
/// with hw_device_ctx set.
unsafe extern "C" fn get_hw_format(
    _ctx: *mut ffmpeg::ffi::AVCodecContext,
    fmts: *const ffmpeg::ffi::AVPixelFormat,
) -> ffmpeg::ffi::AVPixelFormat {
    let target = TARGET_HW_FMT.load(Ordering::Relaxed);
    let mut p = fmts;
    while (*p) as i32 != ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE as i32 {
        if (*p) as i32 == target {
            return *p;
        }
        p = p.add(1);
    }
    // Target not offered — take the decoder's first (software) choice.
    *fmts
}

/// Try to create a hardware device context for this platform.
/// Returns (device_ctx, hw_pixel_format) on success.
unsafe fn create_hw_device() -> Option<(*mut ffmpeg::ffi::AVBufferRef, i32)> {
    use ffmpeg::ffi::*;

    let candidates: &[(AVHWDeviceType, AVPixelFormat, &str)] = &[
        #[cfg(target_os = "linux")]
        (
            AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
            AVPixelFormat::AV_PIX_FMT_VAAPI,
            "VAAPI",
        ),
        #[cfg(target_os = "linux")]
        (
            AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA,
            AVPixelFormat::AV_PIX_FMT_CUDA,
            "NVDEC/CUDA",
        ),
        #[cfg(target_os = "windows")]
        (
            AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA,
            AVPixelFormat::AV_PIX_FMT_D3D11,
            "D3D11VA",
        ),
        #[cfg(target_os = "windows")]
        (
            AVHWDeviceType::AV_HWDEVICE_TYPE_DXVA2,
            AVPixelFormat::AV_PIX_FMT_DXVA2_VLD,
            "DXVA2",
        ),
        #[cfg(target_os = "macos")]
        (
            AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
            AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX,
            "VideoToolbox",
        ),
    ];

    for (dev_type, pix_fmt, name) in candidates {
        let mut ctx: *mut AVBufferRef = std::ptr::null_mut();
        let ret = av_hwdevice_ctx_create(
            &mut ctx,
            *dev_type,
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
        );
        if ret == 0 && !ctx.is_null() {
            log_event!(codes::DECODER_INIT_HARDWARE, "device" => name);
            return Some((ctx, *pix_fmt as i32));
        }
    }
    None
}

pub struct H265Decoder {
    decoder: decoder::Video,
    scaler: Option<ScaleContext>,
    scaler_in_format: Pixel,
    bgra_frame: Option<ffmpeg::util::frame::Video>,
    hw_device: Option<*mut ffmpeg::ffi::AVBufferRef>,
    using_hw: bool,
}

// The raw hw_device pointer is only touched from the decoder thread.
unsafe impl Send for H265Decoder {}

impl Drop for H265Decoder {
    fn drop(&mut self) {
        if let Some(mut dev) = self.hw_device.take() {
            unsafe { ffmpeg::ffi::av_buffer_unref(&mut dev) };
        }
    }
}

impl H265Decoder {
    pub fn new(try_hw: bool) -> Result<Self, ffmpeg::Error> {
        ffmpeg::init()?;

        let codec =
            ffmpeg::decoder::find(ffmpeg::codec::Id::HEVC).ok_or(ffmpeg::Error::DecoderNotFound)?;

        let mut context = ffmpeg::codec::context::Context::new_with_codec(codec);

        // Multithreaded software decode as the baseline. Frame threading adds
        // a small pipeline delay but is the only way software HEVC keeps up
        // with high-fps content; 4 threads bounds that delay.
        context.set_threading(ffmpeg::codec::threading::Config {
            kind: ffmpeg::codec::threading::Type::Frame,
            count: 4,
        });

        let mut hw_device = None;
        let mut using_hw = false;

        if try_hw {
            unsafe {
                if let Some((dev, pix_fmt)) = create_hw_device() {
                    TARGET_HW_FMT.store(pix_fmt, Ordering::Relaxed);
                    let raw = context.as_mut_ptr();
                    (*raw).hw_device_ctx = ffmpeg::ffi::av_buffer_ref(dev);
                    (*raw).get_format = Some(get_hw_format);
                    hw_device = Some(dev);
                    using_hw = true;
                }
            }
        }

        if !using_hw {
            TARGET_HW_FMT.store(-1, Ordering::Relaxed);
            log_event!(codes::DECODER_INIT_SOFTWARE);
        }

        let decoder = context.decoder().video()?;

        Ok(H265Decoder {
            decoder,
            scaler: None,
            scaler_in_format: Pixel::None,
            bgra_frame: None,
            hw_device,
            using_hw,
        })
    }

    pub fn is_hw(&self) -> bool {
        self.using_hw
    }

    /// Drop any buffered reference frames. Called when decoding resumes after
    /// a gap, so stale references do not smear over the first frames.
    pub fn flush(&mut self) {
        self.decoder.flush();
    }

    fn is_hw_frame(frame: &ffmpeg::util::frame::Video) -> bool {
        let av_fmt: ffmpeg::ffi::AVPixelFormat = frame.format().into();
        unsafe {
            let desc = ffmpeg::ffi::av_pix_fmt_desc_get(av_fmt);
            if desc.is_null() {
                return false;
            }
            ((*desc).flags & ffmpeg::ffi::AV_PIX_FMT_FLAG_HWACCEL as u64) != 0
        }
    }

    pub fn decode_and_push(
        &mut self,
        data: &[u8],
        timestamp: u64,
        decode_started: std::time::Instant,
    ) -> Result<(), ffmpeg::Error> {
        let mut packet = packet::Packet::new(data.len());
        if let Some(pdata) = packet.data_mut() {
            pdata.copy_from_slice(data);
        }
        packet.set_pts(Some(timestamp as i64));

        self.decoder.send_packet(&packet)?;

        let mut frame = ffmpeg::util::frame::Video::empty();
        let mut sw_frame = ffmpeg::util::frame::Video::empty();

        while self.decoder.receive_frame(&mut frame).is_ok() {
            // Hardware frames live in GPU memory — download to system memory
            // (typically NV12) before color conversion.
            let src: &ffmpeg::util::frame::Video = if Self::is_hw_frame(&frame) {
                unsafe {
                    let ret = ffmpeg::ffi::av_hwframe_transfer_data(
                        sw_frame.as_mut_ptr(),
                        frame.as_ptr(),
                        0,
                    );
                    if ret < 0 {
                        return Err(ffmpeg::Error::from(ret));
                    }
                    let _ = ffmpeg::ffi::av_frame_copy_props(sw_frame.as_mut_ptr(), frame.as_ptr());
                }
                &sw_frame
            } else {
                &frame
            };

            let width = src.width();
            let height = src.height();
            let format = src.format();

            let needs_new_scaler = self.scaler.is_none()
                || self.scaler_in_format != format
                || self
                    .bgra_frame
                    .as_ref()
                    .is_none_or(|f| f.width() != width || f.height() != height);

            if needs_new_scaler {
                let scaler = ScaleContext::get(
                    format,
                    width,
                    height,
                    Pixel::BGRA,
                    width,
                    height,
                    flag::Flags::BILINEAR,
                )?;
                self.scaler = Some(scaler);
                self.scaler_in_format = format;
                self.bgra_frame = Some(ffmpeg::util::frame::Video::new(Pixel::BGRA, width, height));
            }

            if let (Some(scaler), Some(bgra_frame)) =
                (self.scaler.as_mut(), self.bgra_frame.as_mut())
            {
                if scaler.run(src, bgra_frame).is_ok() {
                    let width_u32 = bgra_frame.width();
                    let height_u32 = bgra_frame.height();
                    let width = width_u32 as usize;
                    let height = height_u32 as usize;
                    let stride = bgra_frame.stride(0);
                    let data = bgra_frame.data(0);

                    // Acquire buffer from pool
                    let mut buffer = crate::pipeline::framepool::acquire(width * height * 4);

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

                    // Single delivery point for decoded frames.
                    crate::deliver_frame(buffer, width_u32, height_u32, decode_started);
                }
            }
        }

        Ok(())
    }
}

pub fn start_decoder_thread(queue: Arc<VideoQueue>, my_gen: u64) {
    std::thread::spawn(move || {
        log_event!(codes::DECODER_THREAD_STARTED);

        let mut decoder = match H265Decoder::new(true) {
            Ok(d) => d,
            Err(e) => {
                log_event!(codes::DECODER_INIT_FAILED, "error" => format!("{e:?}"));
                return;
            }
        };

        let mut consecutive_errors: u32 = 0;
        let mut was_idle = true;
        let mut seen_epoch = crate::stream_epoch();

        loop {
            if !crate::session_alive(my_gen) {
                log_event!(codes::DECODER_THREAD_STOPPING);
                break;
            }

            // Blocks until a packet arrives — no sleep-poll latency.
            let Some(packet_data) = queue.pop_timeout(Duration::from_millis(100)) else {
                continue;
            };

            // Decoding only exists to serve the OBS shared-memory feed and to
            // report the frame size to the player. With neither wanting a
            // decoded frame, drop the packet instead of burning a core —
            // ffplay decodes its own copy from the passthrough stream.
            if !crate::needs_decoded_frames() {
                was_idle = true;
                continue;
            }
            // A reconnect starts a fresh bitstream; decoding it against the
            // previous session's reference frames smears until the next IRAP.
            let epoch = crate::stream_epoch();
            if epoch != seen_epoch {
                seen_epoch = epoch;
                was_idle = true;
            }
            if was_idle {
                // Resuming mid-GOP: references are stale until the next IRAP.
                decoder.flush();
                was_idle = false;
            }

            let decode_started = std::time::Instant::now();
            let timestamp = FRAME_COUNTER.fetch_add(1, Ordering::Relaxed);
            match decoder.decode_and_push(&packet_data, timestamp, decode_started) {
                Ok(_) => consecutive_errors = 0,
                Err(e) => {
                    consecutive_errors += 1;
                    if consecutive_errors <= 3 {
                        log_event!(codes::DECODER_DECODE_FAILED, "error" => format!("{e:?}"));
                    }
                    // A hardware session that persistently fails (unsupported
                    // profile, driver quirk) gets rebuilt in software once.
                    if consecutive_errors == 60 && decoder.is_hw() {
                        log_event!(codes::DECODER_HARDWARE_FALLBACK);
                        if let Ok(sw) = H265Decoder::new(false) {
                            decoder = sw;
                            consecutive_errors = 0;
                        }
                    }
                }
            }
        }
    });
}
