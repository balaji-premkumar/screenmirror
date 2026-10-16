use crate::usb_loop::{push_audio_to_muxer, push_video_to_muxer, KEYFRAME_REQUESTED};
use jni::objects::{JByteArray, JClass};
use jni::sys::{jint, jlong};
use jni::JNIEnv;
use std::cell::RefCell;

thread_local! {
    /// Scratch buffer for the JNI copy, reused across calls.
    ///
    /// The Kotlin side now hands us a reusable array plus a length instead of
    /// a freshly allocated one per frame, and `convert_byte_array` (which
    /// allocates a fresh Vec every call) is replaced by a region read into
    /// this buffer. At 120 fps that removes two allocations per frame from the
    /// MediaCodec drain path.
    static SCRATCH: RefCell<Vec<i8>> = RefCell::new(Vec::new());
}

/// Copy `len` bytes out of `data` into the thread-local scratch buffer and
/// hand them to `sink`. Returns false if the region read fails.
fn with_bytes<F>(env: &JNIEnv, data: &JByteArray, len: jint, sink: F) -> bool
where
    F: FnOnce(&[u8]) -> bool,
{
    if len <= 0 {
        return false;
    }
    let n = len as usize;

    SCRATCH.with(|cell| {
        let mut buf = cell.borrow_mut();
        if buf.len() < n {
            buf.resize(n, 0);
        }
        if env.get_byte_array_region(data, 0, &mut buf[..n]).is_err() {
            return false;
        }
        // jbyte is i8; the payload is opaque bytes either way.
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, n) };
        sink(bytes)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_mirror_stream_1mobile_1app_service_MirrorForegroundService_pushToUsb(
    env: JNIEnv,
    _class: JClass,
    data: JByteArray,
    len: jint,
    pts_us: jlong,
) -> jni::sys::jboolean {
    // Push encoded H.265 video data directly into the Muxer pipeline,
    // which frames it and sends it to the USB write loop. `pts_us` is the
    // capture time from MediaCodec; it travels in the frame header.
    if with_bytes(&env, &data, len, |b| {
        push_video_to_muxer(b, pts_us.max(0) as u64)
    }) {
        jni::sys::JNI_TRUE
    } else {
        jni::sys::JNI_FALSE
    }
}

/// Audio ingress from the Android AudioPlaybackCapture thread.
/// `data` is raw PCM: f32 little-endian, mono, 48 kHz — the same format the
/// desktop expects on the wire.
#[no_mangle]
pub extern "system" fn Java_com_mirror_stream_1mobile_1app_service_MirrorForegroundService_pushAudioToUsb(
    env: JNIEnv,
    _class: JClass,
    data: JByteArray,
    len: jint,
    pts_us: jlong,
) -> jni::sys::jboolean {
    if with_bytes(&env, &data, len, |b| {
        push_audio_to_muxer(b, pts_us.max(0) as u64)
    }) {
        jni::sys::JNI_TRUE
    } else {
        jni::sys::JNI_FALSE
    }
}

/// True once per keyframe request from the desktop. The encoder thread polls
/// this between output buffers and asks MediaCodec for a sync frame.
#[no_mangle]
pub extern "system" fn Java_com_mirror_stream_1mobile_1app_service_MirrorForegroundService_takeKeyframeRequest(
    _env: JNIEnv,
    _class: JClass,
) -> jni::sys::jboolean {
    if KEYFRAME_REQUESTED.swap(false, std::sync::atomic::Ordering::AcqRel) {
        jni::sys::JNI_TRUE
    } else {
        jni::sys::JNI_FALSE
    }
}
