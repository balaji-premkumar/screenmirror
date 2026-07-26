//! ffplay playback sink.
//!
//! This is the *only* path on which the desktop produces sound. The app never
//! opens an audio device itself: incoming HEVC video and f32 PCM audio are
//! remuxed into a Matroska stream and piped straight into a child `ffplay`
//! process, which owns both the window and the audio output.
//!
//! ```text
//! USB -> Demuxer -+- HEVC -+
//!                 +- PCM  -+-> Matroska (avformat, custom AVIO)
//!                                     | stdin pipe
//!                                     v
//!                               ffplay -i -
//! ```
//!
//! Video is passed through untouched — the desktop decoder is not involved,
//! so this path costs one remux, not a decode plus a colour conversion.
//!
//! Start-up is deferred until two things are known:
//!   1. a packet carrying parameter sets *and* an IRAP picture has arrived,
//!      so ffplay begins at a point it can actually decode;
//!   2. the frame size, reported by the decoder through `note_dimensions()`
//!      (Matroska needs PixelWidth/PixelHeight in the track header).

use crate::log_event;
use crate::pipeline::scan_packet;
use ffmpeg_next as ffmpeg;
use mirror_i18n::codes;
use once_cell::sync::Lazy;
use std::collections::VecDeque;
use std::ffi::CString;
use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

pub const STATE_STOPPED: i32 = 0;
pub const STATE_STARTING: i32 = 1;
pub const STATE_PLAYING: i32 = 2;

static STATE: AtomicI32 = AtomicI32::new(STATE_STOPPED);

/// Bumped on every stop so a worker thread that is mid-write exits instead of
/// writing into the next session's pipe.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Frame size discovered by the decoder. 0 = not known yet.
static DIM_W: AtomicU32 = AtomicU32::new(0);
static DIM_H: AtomicU32 = AtomicU32::new(0);

/// Set once a CSD+keyframe packet has been seen for this session. Until then
/// video packets are discarded — feeding ffplay a mid-GOP start produces a
/// few seconds of macroblock garbage before the next IRAP.
static ARMED: AtomicBool = AtomicBool::new(false);

/// Parameter sets (VPS/SPS/PPS) taken from the arming packet. The Matroska
/// muxer builds the track's `hvcC` CodecPrivate block out of these — without
/// them `avformat_write_header` fails with AVERROR_INVALIDDATA.
static CSD: Lazy<Mutex<Vec<u8>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// Pulls the VPS/SPS/PPS NAL units out of an Annex-B packet, re-emitted with
/// 4-byte start codes. FFmpeg's `hvcC` writer accepts Annex-B extradata.
///
/// The walker itself lives in `mirror-protocol` alongside the frame format,
/// because the mobile sender needs the same one to decide which packets it may
/// drop under queue pressure.
use mirror_protocol::hevc::extract_parameter_sets;

/// True while a session wants packets (starting or playing). Checked on the
/// USB thread, so it is a plain atomic rather than a lock.
static ACCEPTING: AtomicBool = AtomicBool::new(false);

pub fn state() -> i32 {
    STATE.load(Ordering::Acquire)
}

pub fn is_active() -> bool {
    ACCEPTING.load(Ordering::Relaxed)
}

/// The decoder still has to run while the player is waiting to learn the
/// frame size. Once it is known, a player-only session needs no decoding.
pub fn needs_dimensions() -> bool {
    ACCEPTING.load(Ordering::Relaxed) && DIM_W.load(Ordering::Relaxed) == 0
}

/// Called from the decoder's frame sink so the muxer can fill in the
/// Matroska track header.
pub fn note_dimensions(width: u32, height: u32) {
    if width == 0 || height == 0 {
        return;
    }
    if DIM_W.load(Ordering::Relaxed) != width || DIM_H.load(Ordering::Relaxed) != height {
        DIM_W.store(width, Ordering::Relaxed);
        DIM_H.store(height, Ordering::Release);
        QUEUE.wake();
    }
}

// ── Ingress queue ───────────────────────────────────────────

enum Media {
    Video { data: Vec<u8>, keyframe: bool },
    Audio(Vec<u8>),
}

/// ~1 s of video at 120 fps plus a comfortable audio margin. On overflow the
/// oldest *video* packet goes first so audio (small, and the clock ffplay
/// syncs to) survives a stalled pipe.
const QUEUE_CAP: usize = 256;

struct Queue {
    inner: Mutex<VecDeque<Media>>,
    cond: Condvar,
}

impl Queue {
    fn push(&self, item: Media) {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() >= QUEUE_CAP {
            let victim = q
                .iter()
                .position(|m| matches!(m, Media::Video { .. }))
                .unwrap_or(0);
            q.remove(victim);
        }
        q.push_back(item);
        drop(q);
        self.cond.notify_one();
    }

    fn pop(&self, timeout: Duration) -> Option<Media> {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if q.is_empty() {
            let (guard, _) = self
                .cond
                .wait_timeout(q, timeout)
                .unwrap_or_else(|e| e.into_inner());
            q = guard;
        }
        q.pop_front()
    }

    fn clear(&self) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    fn wake(&self) {
        self.cond.notify_all();
    }
}

static QUEUE: Lazy<Queue> = Lazy::new(|| Queue {
    inner: Mutex::new(VecDeque::with_capacity(QUEUE_CAP)),
    cond: Condvar::new(),
});

/// Feed an encoded video packet. Cheap no-op when the player is not running.
pub fn push_video(data: &[u8]) {
    if !ACCEPTING.load(Ordering::Relaxed) {
        return;
    }
    let info = scan_packet(data);
    if !ARMED.load(Ordering::Relaxed) {
        // Wait for a self-contained entry point.
        if !(info.has_parameter_sets && info.has_keyframe) {
            return;
        }
        let params = extract_parameter_sets(data);
        if params.is_empty() {
            return;
        }
        *CSD.lock().unwrap_or_else(|e| e.into_inner()) = params;
        ARMED.store(true, Ordering::Release);
        QUEUE.wake();
    }
    QUEUE.push(Media::Video {
        data: data.to_vec(),
        keyframe: info.has_keyframe,
    });
}

/// Feed raw f32 LE mono 48 kHz audio. Dropped until the video stream is armed
/// so the muxer never emits audio ahead of the first video packet.
pub fn push_audio(data: &[u8]) {
    if !ACCEPTING.load(Ordering::Relaxed) || !ARMED.load(Ordering::Relaxed) {
        return;
    }
    if data.len() < 4 {
        return;
    }
    QUEUE.push(Media::Audio(data.to_vec()));
}

// ── Pipe sink (AVIO -> child stdin) ─────────────────────────

struct PipeSink {
    stdin: Option<ChildStdin>,
    broken: bool,
}

unsafe extern "C" fn write_packet(
    opaque: *mut libc::c_void,
    buf: *const u8,
    buf_size: libc::c_int,
) -> libc::c_int {
    if opaque.is_null() || buf.is_null() || buf_size <= 0 {
        return 0;
    }
    let sink = &mut *(opaque as *mut PipeSink);
    let Some(stdin) = sink.stdin.as_mut() else {
        return -libc::EIO;
    };
    let slice = std::slice::from_raw_parts(buf, buf_size as usize);
    match stdin.write_all(slice) {
        Ok(()) => buf_size,
        Err(_) => {
            // ffplay window closed, or the process died.
            sink.broken = true;
            -libc::EPIPE
        }
    }
}

// ── Matroska muxer ──────────────────────────────────────────

struct Muxer {
    ctx: *mut ffmpeg::ffi::AVFormatContext,
    sink: *mut PipeSink,
    video_idx: i32,
    audio_idx: i32,
    header_written: bool,
}

impl Muxer {
    // NB: `pipe`, not `stdin` — `use ffmpeg::ffi::*` below pulls in a C
    // binding called `stdin` that would shadow the parameter.
    unsafe fn new(
        width: u32,
        height: u32,
        extradata: &[u8],
        pipe: ChildStdin,
    ) -> Result<Muxer, String> {
        use ffmpeg::ffi::*;

        let mut ctx: *mut AVFormatContext = ptr::null_mut();
        let fmt = CString::new("matroska").unwrap();
        if avformat_alloc_output_context2(&mut ctx, ptr::null_mut(), fmt.as_ptr(), ptr::null()) < 0
            || ctx.is_null()
        {
            return Err("avformat_alloc_output_context2 failed".into());
        }

        let mut mux = Muxer {
            ctx,
            sink: ptr::null_mut(),
            video_idx: -1,
            audio_idx: -1,
            header_written: false,
        };

        // Custom AVIO writing straight into the child's stdin.
        const IO_BUF: usize = 64 * 1024;
        let io_buf = av_malloc(IO_BUF) as *mut libc::c_uchar;
        if io_buf.is_null() {
            return Err("av_malloc for AVIO buffer failed".into());
        }
        let sink = Box::into_raw(Box::new(PipeSink {
            stdin: Some(pipe),
            broken: false,
        }));
        mux.sink = sink;

        let pb = avio_alloc_context(
            io_buf,
            IO_BUF as libc::c_int,
            1, // write_flag
            sink as *mut libc::c_void,
            None,
            Some(write_packet),
            None, // not seekable — a pipe
        );
        if pb.is_null() {
            av_free(io_buf as *mut libc::c_void);
            return Err("avio_alloc_context failed".into());
        }
        (*pb).seekable = 0;
        (*ctx).pb = pb;
        (*ctx).flags |= AVFMT_FLAG_CUSTOM_IO;
        // Never let the interleaver hold video hostage waiting for audio that
        // may never come (phone with the mic permission denied).
        (*ctx).max_interleave_delta = 100_000; // µs

        // ── Video: HEVC passthrough, millisecond timebase ──
        let vs = avformat_new_stream(ctx, ptr::null());
        if vs.is_null() {
            return Err("avformat_new_stream(video) failed".into());
        }
        mux.video_idx = (*vs).index;
        (*vs).time_base = AVRational { num: 1, den: 1000 };
        let vpar = (*vs).codecpar;
        (*vpar).codec_type = AVMediaType::AVMEDIA_TYPE_VIDEO;
        (*vpar).codec_id = AVCodecID::AV_CODEC_ID_HEVC;
        (*vpar).width = width as libc::c_int;
        (*vpar).height = height as libc::c_int;
        (*vpar).format = AVPixelFormat::AV_PIX_FMT_YUV420P as libc::c_int;

        // CodecPrivate (hvcC) is mandatory for HEVC in Matroska.
        if extradata.is_empty() {
            return Err("no HEVC parameter sets available for the track header".into());
        }
        let pad = AV_INPUT_BUFFER_PADDING_SIZE as usize;
        let ed = av_mallocz(extradata.len() + pad) as *mut u8;
        if ed.is_null() {
            return Err("av_mallocz for extradata failed".into());
        }
        ptr::copy_nonoverlapping(extradata.as_ptr(), ed, extradata.len());
        (*vpar).extradata = ed;
        (*vpar).extradata_size = extradata.len() as libc::c_int;

        // ── Audio: raw f32 LE mono at the wire rate ──
        let a = avformat_new_stream(ctx, ptr::null());
        if a.is_null() {
            return Err("avformat_new_stream(audio) failed".into());
        }
        mux.audio_idx = (*a).index;
        (*a).time_base = AVRational {
            num: 1,
            den: crate::sinks::SOURCE_SAMPLE_RATE as libc::c_int,
        };
        let apar = (*a).codecpar;
        (*apar).codec_type = AVMediaType::AVMEDIA_TYPE_AUDIO;
        (*apar).codec_id = AVCodecID::AV_CODEC_ID_PCM_F32LE;
        (*apar).format = AVSampleFormat::AV_SAMPLE_FMT_FLT as libc::c_int;
        (*apar).sample_rate = crate::sinks::SOURCE_SAMPLE_RATE as libc::c_int;
        (*apar).bits_per_coded_sample = 32;
        (*apar).block_align = 4;
        av_channel_layout_default(&mut (*apar).ch_layout, 1);

        let ret = avformat_write_header(ctx, ptr::null_mut());
        if ret < 0 {
            return Err(format!("avformat_write_header failed ({ret})"));
        }
        mux.header_written = true;
        Ok(mux)
    }

    fn pipe_broken(&self) -> bool {
        !self.sink.is_null() && unsafe { (*self.sink).broken }
    }

    unsafe fn write(
        &mut self,
        stream_idx: i32,
        data: &[u8],
        pts: i64,
        duration: i64,
        keyframe: bool,
    ) -> Result<(), String> {
        use ffmpeg::ffi::*;

        let pkt = av_packet_alloc();
        if pkt.is_null() {
            return Err("av_packet_alloc failed".into());
        }
        if av_new_packet(pkt, data.len() as libc::c_int) < 0 {
            av_packet_free(&mut { pkt });
            return Err("av_new_packet failed".into());
        }
        ptr::copy_nonoverlapping(data.as_ptr(), (*pkt).data, data.len());
        (*pkt).stream_index = stream_idx;
        (*pkt).pts = pts;
        (*pkt).dts = pts;
        (*pkt).duration = duration;
        if keyframe {
            (*pkt).flags |= AV_PKT_FLAG_KEY as libc::c_int;
        }

        let ret = av_interleaved_write_frame(self.ctx, pkt);
        av_packet_free(&mut { pkt });
        if ret < 0 {
            return Err(format!("av_interleaved_write_frame failed ({ret})"));
        }
        Ok(())
    }
}

impl Drop for Muxer {
    fn drop(&mut self) {
        unsafe {
            use ffmpeg::ffi::*;
            if self.header_written && !self.pipe_broken() {
                let _ = av_write_trailer(self.ctx);
            }
            if !self.ctx.is_null() {
                let pb = (*self.ctx).pb;
                if !pb.is_null() {
                    av_free((*pb).buffer as *mut libc::c_void);
                    avio_context_free(&mut { pb });
                    (*self.ctx).pb = ptr::null_mut();
                }
                avformat_free_context(self.ctx);
                self.ctx = ptr::null_mut();
            }
            if !self.sink.is_null() {
                // Dropping the ChildStdin closes the pipe, so ffplay sees EOF.
                drop(Box::from_raw(self.sink));
                self.sink = ptr::null_mut();
            }
        }
    }
}

// ── ffplay discovery & launch ───────────────────────────────

fn ffplay_command(project_root: &str) -> Option<Command> {
    #[cfg(target_os = "windows")]
    let bundled = format!(r"{project_root}\bin\ffplay.exe");
    #[cfg(not(target_os = "windows"))]
    let bundled = format!("{project_root}/bin/ffplay");

    if !project_root.is_empty() && std::path::Path::new(&bundled).exists() {
        return Some(Command::new(bundled));
    }

    #[cfg(target_os = "windows")]
    let finder = ("where", "ffplay");
    #[cfg(not(target_os = "windows"))]
    let finder = ("which", "ffplay");

    let found = Command::new(finder.0)
        .arg(finder.1)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    found.then(|| Command::new("ffplay"))
}

fn spawn_ffplay(project_root: &str) -> Result<Child, String> {
    let mut cmd = ffplay_command(project_root)
        .ok_or_else(|| "ffplay not found in bin/ or on PATH".to_string())?;

    cmd.args([
        "-hide_banner",
        "-loglevel",
        "warning",
        // Live-source tuning: don't buffer ahead, don't wait to probe.
        "-fflags",
        "nobuffer",
        "-flags",
        "low_delay",
        "-probesize",
        "32768",
        "-analyzeduration",
        "0",
        "-framedrop",
        "-window_title",
        "Mirror Stream (USB)",
        "-autoexit",
        "-f",
        "matroska",
        "-i",
        "-",
    ])
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::null());

    cmd.spawn()
        .map_err(|e| format!("failed to launch ffplay: {e}"))
}

// ── Lifecycle ───────────────────────────────────────────────

/// Start playback. Returns 0 when a session was started (or one was already
/// running), -1 when ffplay is unavailable.
pub fn start(project_root: &str) -> i32 {
    if STATE
        .compare_exchange(
            STATE_STOPPED,
            STATE_STARTING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return 0; // already starting or playing
    }

    // Writing to a pipe whose reader has gone away must not kill the host
    // process — the user closing the ffplay window is a normal event.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let my_gen = GENERATION.load(Ordering::Acquire);
    ARMED.store(false, Ordering::Release);
    DIM_W.store(0, Ordering::Relaxed);
    DIM_H.store(0, Ordering::Relaxed);
    QUEUE.clear();
    ACCEPTING.store(true, Ordering::Release);

    let root = project_root.to_string();
    std::thread::spawn(move || run_session(root, my_gen));

    log_event!(codes::PLAYER_WAITING_FOR_KEYFRAME);
    0
}

pub fn stop() {
    if STATE.load(Ordering::Acquire) == STATE_STOPPED {
        return;
    }
    GENERATION.fetch_add(1, Ordering::AcqRel);
    ACCEPTING.store(false, Ordering::Release);
    ARMED.store(false, Ordering::Release);
    QUEUE.clear();
    QUEUE.wake();
    STATE.store(STATE_STOPPED, Ordering::Release);
}

fn session_alive(my_gen: u64) -> bool {
    GENERATION.load(Ordering::Acquire) == my_gen
}

fn run_session(project_root: String, my_gen: u64) {
    // ── Phase 1: wait until we can start cleanly ──
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if !session_alive(my_gen) {
            return;
        }
        let dims_ready = DIM_W.load(Ordering::Acquire) > 0;
        if dims_ready && ARMED.load(Ordering::Acquire) {
            break;
        }
        if Instant::now() >= deadline {
            log_event!(codes::PLAYER_KEYFRAME_TIMEOUT);
            finish(my_gen);
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let width = DIM_W.load(Ordering::Acquire);
    let height = DIM_H.load(Ordering::Acquire);

    // ── Phase 2: launch ffplay and build the muxer ──
    let mut child = match spawn_ffplay(&project_root) {
        Ok(c) => c,
        Err(e) => {
            log_event!(codes::PLAYER_SPAWN_FAILED, "error" => e);
            finish(my_gen);
            return;
        }
    };
    let Some(stdin) = child.stdin.take() else {
        log_event!(codes::PLAYER_STDIN_UNAVAILABLE);
        let _ = child.kill();
        finish(my_gen);
        return;
    };

    let extradata = CSD.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut muxer = match unsafe { Muxer::new(width, height, &extradata, stdin) } {
        Ok(m) => m,
        Err(e) => {
            log_event!(codes::PLAYER_MUXER_FAILED, "error" => e);
            let _ = child.kill();
            let _ = child.wait();
            finish(my_gen);
            return;
        }
    };

    STATE.store(STATE_PLAYING, Ordering::Release);
    log_event!(codes::PLAYER_PLAYING, "width" => width, "height" => height);

    // ── Phase 3: pump ──
    // Video is stamped with arrival time; audio with its own sample count,
    // which is the steadier clock and the one ffplay syncs to.
    let t0 = Instant::now();
    let mut audio_samples: i64 = 0;
    let mut last_video_ms: i64 = -1;

    loop {
        if !session_alive(my_gen) {
            break;
        }
        if muxer.pipe_broken() {
            log_event!(codes::PLAYER_WINDOW_CLOSED);
            break;
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                log_event!(codes::PLAYER_EXITED);
                break;
            }
            Err(_) => break,
            Ok(None) => {}
        }

        let Some(item) = QUEUE.pop(Duration::from_millis(100)) else {
            continue;
        };

        let res = match item {
            Media::Video { data, keyframe } => {
                let mut ms = t0.elapsed().as_millis() as i64;
                // Matroska will not accept a timestamp that goes backwards.
                if ms <= last_video_ms {
                    ms = last_video_ms + 1;
                }
                last_video_ms = ms;
                unsafe { muxer.write(muxer.video_idx, &data, ms, 0, keyframe) }
            }
            Media::Audio(data) => {
                let samples = (data.len() / 4) as i64;
                let pts = audio_samples;
                audio_samples += samples;
                unsafe { muxer.write(muxer.audio_idx, &data, pts, samples, true) }
            }
        };

        if let Err(e) = res {
            if !muxer.pipe_broken() {
                log_event!(codes::PLAYER_MUX_ERROR, "error" => e);
            }
            break;
        }
    }

    // ── Teardown: trailer + close pipe, then reap the child ──
    drop(muxer);
    let _ = child.wait();
    log_event!(codes::PLAYER_SESSION_ENDED);
    finish(my_gen);
}

/// Reset shared state, but only if no newer session has claimed it.
fn finish(my_gen: u64) {
    if !session_alive(my_gen) {
        return;
    }
    ACCEPTING.store(false, Ordering::Release);
    ARMED.store(false, Ordering::Release);
    QUEUE.clear();
    STATE.store(STATE_STOPPED, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_parameter_sets() {
        let mut pkt = vec![0, 0, 0, 1, 32 << 1, 0xAA]; // VPS
        pkt.extend_from_slice(&[0, 0, 1, 33 << 1, 0xBB]); // SPS, 3-byte start
        pkt.extend_from_slice(&[0, 0, 0, 1, 34 << 1, 0xCC]); // PPS
        pkt.extend_from_slice(&[0, 0, 0, 1, 19 << 1, 0xDD]); // IDR — must be excluded

        let out = extract_parameter_sets(&pkt);
        assert_eq!(
            out,
            vec![
                0,
                0,
                0,
                1,
                32 << 1,
                0xAA, // VPS, normalised to a 4-byte start code
                0,
                0,
                0,
                1,
                33 << 1,
                0xBB, // SPS
                0,
                0,
                0,
                1,
                34 << 1,
                0xCC, // PPS
            ]
        );
    }

    #[test]
    fn extracts_nothing_from_a_plain_frame() {
        assert!(extract_parameter_sets(&[0, 0, 0, 1, 1 << 1, 0x01]).is_empty());
    }

    fn have(bin: &str) -> bool {
        Command::new(bin)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Split an Annex-B elementary stream at every VPS (NAL type 32), which
    /// with `repeat-headers` + `keyint=1` is exactly one access unit each.
    fn split_access_units(es: &[u8]) -> Vec<Vec<u8>> {
        const VPS: [u8; 5] = [0, 0, 0, 1, 32 << 1];
        let mut starts: Vec<usize> = Vec::new();
        let mut i = 0;
        while i + VPS.len() <= es.len() {
            if es[i..i + VPS.len()] == VPS {
                starts.push(i);
                i += VPS.len();
            } else {
                i += 1;
            }
        }
        starts
            .iter()
            .enumerate()
            .map(|(n, &s)| {
                let end = starts.get(n + 1).copied().unwrap_or(es.len());
                es[s..end].to_vec()
            })
            .collect()
    }

    /// Build a short all-IDR HEVC elementary stream plus matching PCM.
    /// Returns None when the encoder or ffmpeg itself is unavailable.
    fn make_fixture() -> Option<(Vec<Vec<u8>>, Vec<u8>)> {
        if !have("ffmpeg") {
            return None;
        }
        let dir = std::env::temp_dir().join("mirror_player_test");
        let _ = std::fs::create_dir_all(&dir);
        let es_path = dir.join("src.h265");
        let pcm_path = dir.join("src.pcm");

        let ok = Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x240:rate=30",
                "-t",
                "1",
                "-c:v",
                "libx265",
                "-x265-params",
                "keyint=1:repeat-headers=1:log-level=none",
                "-f",
                "hevc",
            ])
            .arg(&es_path)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return None;
        }
        Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "1",
                "-f",
                "f32le",
                "-ac",
                "1",
            ])
            .arg(&pcm_path)
            .status()
            .ok()?;

        let units = split_access_units(&std::fs::read(&es_path).ok()?);
        let pcm = std::fs::read(&pcm_path).ok()?;
        Some((units, pcm))
    }

    /// Full live path with a real ffplay window: start() -> arm on a keyframe
    /// -> learn dimensions -> spawn ffplay -> play -> stop().
    ///
    /// Opens a window for a couple of seconds, so it is opt-in:
    ///   cargo test --release -- --ignored live_playback
    #[test]
    #[ignore]
    fn live_playback_through_ffplay() {
        let Some((units, pcm)) = make_fixture() else {
            eprintln!("fixtures unavailable — skipping");
            return;
        };

        assert_eq!(state(), STATE_STOPPED);
        assert_eq!(start(""), 0, "player failed to start");
        assert!(is_active(), "player should be accepting packets");

        // Nothing should be queued before an armed keyframe is seen.
        assert!(
            needs_dimensions(),
            "should still be waiting for the frame size"
        );

        // The decoder reports the frame size in the live pipeline.
        note_dimensions(320, 240);

        // Feed roughly in real time so ffplay behaves like it would live.
        let block = 1024 * 4;
        let mut audio_off = 0usize;
        for au in &units {
            push_video(au);
            if audio_off + block <= pcm.len() {
                push_audio(&pcm[audio_off..audio_off + block]);
                audio_off += block;
            }
            std::thread::sleep(Duration::from_millis(33));
        }

        assert!(ARMED.load(Ordering::Acquire), "never armed on a keyframe");
        assert_eq!(
            state(),
            STATE_PLAYING,
            "ffplay session did not reach PLAYING"
        );

        stop();
        assert_eq!(state(), STATE_STOPPED);
        assert!(!is_active(), "player should stop accepting packets");
    }

    /// Drives the real Matroska muxer over a real pipe into a real ffmpeg,
    /// then has ffprobe verify what came out. This is the part that cannot be
    /// checked by opening an ffplay window on a headless machine.
    #[test]
    fn muxes_hevc_and_pcm_into_a_stream_ffmpeg_accepts() {
        if !have("ffmpeg") || !have("ffprobe") {
            eprintln!("ffmpeg/ffprobe unavailable — skipping");
            return;
        }

        let dir = std::env::temp_dir().join("mirror_player_test");
        let _ = std::fs::create_dir_all(&dir);
        let es_path = dir.join("src.h265");
        let pcm_path = dir.join("src.pcm");
        let out_path = dir.join("out.mkv");
        let _ = std::fs::remove_file(&out_path);

        // All-IDR so every access unit carries its own parameter sets.
        let enc = Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x240:rate=30",
                "-t",
                "0.5",
                "-c:v",
                "libx265",
                "-x265-params",
                "keyint=1:repeat-headers=1:log-level=none",
                "-f",
                "hevc",
            ])
            .arg(&es_path)
            .status();
        if !enc.map(|s| s.success()).unwrap_or(false) {
            eprintln!("libx265 unavailable — skipping");
            return;
        }

        assert!(Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "0.5",
                "-f",
                "f32le",
                "-ac",
                "1",
            ])
            .arg(&pcm_path)
            .status()
            .unwrap()
            .success());

        let es = std::fs::read(&es_path).unwrap();
        let pcm = std::fs::read(&pcm_path).unwrap();
        let units = split_access_units(&es);
        assert!(
            units.len() > 5,
            "expected several access units, got {}",
            units.len()
        );

        // Stand in for ffplay: same Matroska over the same kind of pipe.
        let mut child = Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "matroska",
                "-i",
                "-",
                "-c",
                "copy",
            ])
            .arg(&out_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pipe = child.stdin.take().unwrap();

        // Same path the live code takes: parameter sets come from the packet
        // that arms the session.
        let extradata = extract_parameter_sets(&units[0]);
        assert!(!extradata.is_empty(), "no parameter sets found in first AU");

        {
            let mut mux = unsafe { Muxer::new(320, 240, &extradata, pipe) }.expect("muxer setup");

            // Interleave audio the way the live path does: 1024-sample blocks.
            let block = 1024 * 4;
            let mut audio_off = 0usize;
            let mut audio_pts: i64 = 0;
            for (n, au) in units.iter().enumerate() {
                let ms = (n as i64) * 1000 / 30;
                unsafe { mux.write(mux.video_idx, au, ms, 0, true) }.expect("video write");

                if audio_off + block <= pcm.len() {
                    let chunk = &pcm[audio_off..audio_off + block];
                    unsafe { mux.write(mux.audio_idx, chunk, audio_pts, 1024, true) }
                        .expect("audio write");
                    audio_off += block;
                    audio_pts += 1024;
                }
            }
        } // Drop writes the trailer and closes the pipe.

        assert!(
            child.wait().unwrap().success(),
            "ffmpeg rejected our stream"
        );

        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name,width,height,sample_rate,channels",
                "-of",
                "csv=p=0",
            ])
            .arg(&out_path)
            .output()
            .unwrap();
        let report = String::from_utf8_lossy(&probe.stdout);

        assert!(report.contains("hevc"), "no HEVC stream: {report}");
        assert!(report.contains("320,240"), "wrong dimensions: {report}");
        assert!(report.contains("pcm_f32le"), "no PCM stream: {report}");
        assert!(report.contains("48000"), "wrong sample rate: {report}");

        // A container that parses is not enough — it has to decode.
        let decode = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&out_path)
            .args(["-f", "null", "-"])
            .output()
            .unwrap();
        let errs = String::from_utf8_lossy(&decode.stderr);
        assert!(
            decode.status.success() && errs.trim().is_empty(),
            "decode errors: {errs}"
        );
    }
}
