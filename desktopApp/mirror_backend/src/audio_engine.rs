use rubato::{Resampler, SincFixedIn, InterpolationType, InterpolationParameters, WindowFunction};
use std::sync::{Arc, Mutex};
use std::collections::VecDeque;

pub struct AudioEngine {
    resampler: Option<SincFixedIn<f32>>,
    jitter_buffer: Arc<Mutex<VecDeque<f32>>>,
    input_rate: u32,
    output_rate: u32,
    channels: usize,
}

impl AudioEngine {
    pub fn new(input_rate: u32, output_rate: u32, channels: usize) -> Self {
        let mut engine = AudioEngine {
            resampler: None,
            jitter_buffer: Arc::new(Mutex::new(VecDeque::with_capacity(96000))),
            input_rate,
            output_rate,
            channels,
        };
        engine.init_resampler();
        engine
    }

    fn init_resampler(&mut self) {
        if self.input_rate != self.output_rate {
            let params = InterpolationParameters {
                sinc_len: 256,
                f_cutoff: 0.95,
                interpolation: InterpolationType::Linear,
                oversampling_factor: 256,
                window: WindowFunction::BlackmanHarris2,
            };
            
            let resampler = SincFixedIn::new(
                self.output_rate as f64 / self.input_rate as f64,
                2.0,
                params,
                1024,
                self.channels,
            ).unwrap();
            
            self.resampler = Some(resampler);
        }
    }

    pub fn push_samples(&self, samples: &[f32]) {
        let mut buffer = self.jitter_buffer.lock().unwrap();
        // Adaptive jitter buffer: if it's growing too large, we drop older samples
        // Maintaining ~100ms of audio (4800 samples)
        if buffer.len() > 9600 { // 200ms
           let to_drop = buffer.len() - 4800;
           buffer.drain(0..to_drop);
        }
        buffer.extend(samples);
    }

    pub fn process_resampling(&mut self, samples: Vec<Vec<f32>>) -> Vec<Vec<f32>> {
        if let Some(ref mut resampler) = self.resampler {
            resampler.process(&samples, None).unwrap()
        } else {
            samples
        }
    }

    pub fn get_next_buffer(&self, count: usize) -> Vec<f32> {
        let mut buffer = self.jitter_buffer.lock().unwrap();
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            out.push(buffer.pop_front().unwrap_or(0.0));
        }
        out
    }
}

pub fn audio_output_connect(input_rate: u32) -> AudioEngine {
    // 48kHz output is fixed for WASAPI
    AudioEngine::new(input_rate, 48000, 1) // mono input for now
}
