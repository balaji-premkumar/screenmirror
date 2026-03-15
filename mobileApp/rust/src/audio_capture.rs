#[cfg(target_os = "android")]
use oboe::{
    AudioInputCallback, AudioInputStreamSafe, AudioStreamBuilder, InputPreset, Mono, PerformanceMode,
    SharingMode, AudioStream,
};
#[cfg(target_os = "android")]
use std::sync::Arc;
#[cfg(target_os = "android")]
use std::sync::Mutex;

#[cfg(target_os = "android")]
pub struct AudioCapture {
    _stream: Box<dyn AudioStream>,
}

#[cfg(target_os = "android")]
pub struct AudioCallback {
    callback: Arc<Mutex<Box<dyn Fn(&[u8]) + Send + 'static>>>,
}

#[cfg(target_os = "android")]
impl AudioInputCallback for AudioCallback {
    type FrameType = (f32, Mono);

    fn on_audio_ready(
        &mut self,
        _stream: &mut dyn AudioInputStreamSafe,
        frames: &[f32],
    ) -> oboe::DataCallbackResult {
        // Convert f32 PCM samples to raw bytes
        let bytes: Vec<u8> = frames.iter().flat_map(|f| f.to_le_bytes().to_vec()).collect();
        if let Ok(cb) = self.callback.lock() {
            cb(&bytes);
        }
        oboe::DataCallbackResult::Continue
    }
}

#[cfg(target_os = "android")]
impl AudioCapture {
    /// Start audio capture with a callback that receives raw PCM bytes.
    /// The callback will be invoked on the audio thread, so it must be fast.
    pub fn start<F: Fn(&[u8]) + Send + 'static>(on_audio: F) -> Result<Self, anyhow::Error> {
        let callback = AudioCallback { 
            callback: Arc::new(Mutex::new(Box::new(on_audio)))
        };
        
        let stream = AudioStreamBuilder::default()
            .set_input()
            .set_format::<f32>()
            .set_channel_count::<Mono>()
            .set_performance_mode(PerformanceMode::LowLatency)
            .set_sharing_mode(SharingMode::Exclusive)
            .set_input_preset(InputPreset::VoiceRecognition)
            .set_callback(callback)
            .open_stream()
            .map_err(|e| anyhow::anyhow!("Failed to open audio stream: {:?}", e))?;

        let mut stream_boxed: Box<dyn AudioStream> = Box::new(stream);
        stream_boxed.start().map_err(|e| anyhow::anyhow!("Failed to start audio stream: {:?}", e))?;

        Ok(AudioCapture { _stream: stream_boxed })
    }
}

// Fallback for non-android builds
#[cfg(not(target_os = "android"))]
pub struct AudioCapture {}

#[cfg(not(target_os = "android"))]
impl AudioCapture {
    pub fn start<F: Fn(&[u8]) + Send + 'static>(_on_audio: F) -> Result<Self, anyhow::Error> {
        Ok(AudioCapture {})
    }
}
