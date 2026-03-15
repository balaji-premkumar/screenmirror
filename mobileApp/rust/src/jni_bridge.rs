use jni::JNIEnv;
use jni::objects::{JClass, JByteArray};
use crate::usb_loop::push_video_to_muxer;

#[no_mangle]
pub extern "system" fn Java_com_mirror_stream_1mobile_1app_service_MirrorForegroundService_pushToUsb(
    mut env: JNIEnv,
    _class: JClass,
    data: JByteArray,
) -> jni::sys::jboolean {
    let bytes = match env.convert_byte_array(&data) {
        Ok(b) => b,
        Err(_) => return jni::sys::JNI_FALSE,
    };

    // Push encoded H.265 video data directly into the Muxer pipeline
    // which frames it and sends it to the USB write loop
    if push_video_to_muxer(&bytes) {
        jni::sys::JNI_TRUE
    } else {
        jni::sys::JNI_FALSE
    }
}
