use std::sync::Mutex;
use once_cell::sync::Lazy;
use serde::Serialize;
use std::time::{Instant, Duration};

pub static METRICS: Lazy<Mutex<MetricsManager>> = Lazy::new(|| Mutex::new(MetricsManager::new()));

#[derive(Serialize, Clone)]
pub struct MetricsSnapshot {
    pub throughput_mbps: f64,
    pub pipeline_latency_ms: u64,
    pub fps_actual: f64,
    pub frames_dropped: u64,
    pub buffer_health: f64, // 0.0 to 1.0
}

pub struct MetricsManager {
    pub total_bytes: u64,
    pub frame_count: u64,
    pub last_tick: Instant,
    pub start_time: Instant,
    pub dropped_count: u64,
    pub current_latency: u64,
}

impl MetricsManager {
    pub fn new() -> Self {
        Self {
            total_bytes: 0,
            frame_count: 0,
            last_tick: Instant::now(),
            start_time: Instant::now(),
            dropped_count: 0,
            current_latency: 0,
        }
    }

    pub fn record_frame(&mut self, bytes: usize, latency: u64) {
        self.total_bytes += bytes as u64;
        self.frame_count += 1;
        self.current_latency = latency;
    }

    pub fn record_drop(&mut self) {
        self.dropped_count += 1;
    }

    pub fn get_snapshot(&mut self) -> MetricsSnapshot {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_tick).as_secs_f64();
        
        let throughput = if elapsed > 0.0 {
            (self.total_bytes as f64 * 8.0) / (1024.0 * 1024.0 * elapsed)
        } else { 0.0 };

        let fps = if elapsed > 0.0 {
            self.frame_count as f64 / elapsed
        } else { 0.0 };

        // Reset counters for next tick
        self.total_bytes = 0;
        self.frame_count = 0;
        self.last_tick = now;

        MetricsSnapshot {
            throughput_mbps: throughput,
            pipeline_latency_ms: self.current_latency,
            fps_actual: fps,
            frames_dropped: self.dropped_count,
            buffer_health: 0.85, // Mock for now, will link to Jitter buffer later
        }
    }
}
