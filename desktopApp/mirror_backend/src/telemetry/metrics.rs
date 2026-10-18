use once_cell::sync::Lazy;
use serde::Serialize;
use std::sync::Mutex;
use std::time::Instant;

pub static METRICS: Lazy<Mutex<MetricsManager>> = Lazy::new(|| Mutex::new(MetricsManager::new()));

#[derive(Serialize, Clone)]
pub struct MetricsSnapshot {
    pub throughput_mbps: f64,
    /// Time from a packet leaving the ingress queue to its decoded frame
    /// reaching the sinks — decode plus colour conversion plus write.
    ///
    /// Named for what it measures. The previous `pipeline_latency_ms` timed
    /// only the delivery function's own body, so it reported memcpy duration
    /// under a name that implied end-to-end latency. Genuine end-to-end
    /// latency is not measurable without a sender-side capture timestamp on
    /// the wire — see "Known gaps" in `docs/ARCHITECTURE.md`.
    pub decode_latency_ms: u64,
    /// Delay the phone's encoder and queues and the USB link add to a frame
    /// beyond the best case seen recently, from the sender's capture time.
    /// The two clocks are unrelated, so only the variation is meaningful: a
    /// steady 0 means frames arrive as fast as they ever do, and a climbing
    /// value means a queue somewhere between capture and the desktop is filling.
    pub link_delay_ms: u64,
    pub fps_actual: f64,
    pub frames_dropped: u64,
    pub buffer_health: f64, // 0.0 to 1.0
}

pub struct MetricsManager {
    pub total_bytes: u64, // decoded video bytes (for reference/debugging if needed)
    pub total_usb_bytes: u64, // actual network/usb payload bytes
    pub frame_count: u64,
    pub last_tick: Instant,
    pub start_time: Instant,
    pub dropped_count: u64,
    pub current_latency: u64,
    pub link_delay_ms: u64,
    last_snapshot: Option<MetricsSnapshot>,
}

impl Default for MetricsManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsManager {
    pub fn new() -> Self {
        Self {
            total_bytes: 0,
            total_usb_bytes: 0,
            frame_count: 0,
            last_tick: Instant::now(),
            start_time: Instant::now(),
            dropped_count: 0,
            current_latency: 0,
            link_delay_ms: 0,
            last_snapshot: None,
        }
    }

    pub fn record_usb_bytes(&mut self, bytes: usize) {
        self.total_usb_bytes += bytes as u64;
    }

    /// A decoded frame. Only latency and bytes: frames are counted on
    /// arrival ([`Self::record_video_packet`]), because nothing is decoded
    /// unless the OBS feed or the player wants it.
    pub fn record_frame(&mut self, bytes: usize, latency: u64) {
        self.total_bytes += bytes as u64;
        self.current_latency = latency;
    }

    /// A video packet received from the phone. This is what the dashboard's
    /// fps reports, and what tells it the phone is mirroring.
    pub fn record_video_packet(&mut self) {
        self.frame_count += 1;
    }

    pub fn record_link_delay(&mut self, ms: u64) {
        self.link_delay_ms = ms;
    }

    pub fn record_drop(&mut self) {
        self.dropped_count += 1;
    }

    pub fn reset(&mut self) {
        self.total_bytes = 0;
        self.total_usb_bytes = 0;
        self.frame_count = 0;
        self.last_tick = Instant::now();
        self.start_time = Instant::now();
        self.dropped_count = 0;
        self.current_latency = 0;
        self.link_delay_ms = 0;
        self.last_snapshot = None;
    }

    /// Rates are computed over the interval since the previous call, and the
    /// accumulators are then reset — so two callers polling concurrently would
    /// each see a fraction of the real figures. Returning the cached snapshot
    /// for calls that arrive too close together makes an extra poll harmless
    /// instead of silently halving throughput and fps.
    pub fn get_snapshot(&mut self, queue_len: usize, queue_cap: usize) -> MetricsSnapshot {
        const MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

        let now = Instant::now();
        if now.duration_since(self.last_tick) < MIN_INTERVAL {
            if let Some(cached) = &self.last_snapshot {
                return cached.clone();
            }
        }

        let elapsed = now.duration_since(self.last_tick).as_secs_f64();

        // Calculate throughput based on USB bytes (actual stream bandwidth)
        // 1 Mbps = 1,000,000 bits per second
        let throughput = if elapsed > 0.0 {
            (self.total_usb_bytes as f64 * 8.0) / (1_000_000.0 * elapsed)
        } else {
            0.0
        };

        let fps = if elapsed > 0.0 {
            self.frame_count as f64 / elapsed
        } else {
            0.0
        };

        // Reset counters for next tick
        self.total_bytes = 0;
        self.total_usb_bytes = 0;
        self.frame_count = 0;
        self.last_tick = now;

        // 1.0 = queue empty (decoder keeping up), 0.0 = queue full (dropping)
        let buffer_health = if queue_cap > 0 {
            1.0 - (queue_len as f64 / queue_cap as f64)
        } else {
            1.0
        };

        let snapshot = MetricsSnapshot {
            throughput_mbps: throughput,
            decode_latency_ms: self.current_latency,
            link_delay_ms: self.link_delay_ms,
            fps_actual: fps,
            frames_dropped: self.dropped_count,
            buffer_health,
        };
        self.last_snapshot = Some(snapshot.clone());
        snapshot
    }
}

/// Turns sender capture times into "how much slower than the best case".
///
/// The best case is the smallest `arrival - pts` seen, over a sliding pair of
/// windows so that slow clock drift between the two devices does not
/// accumulate into a permanent offset.
pub struct LinkDelay {
    floor: i64,
    window_min: i64,
    window_start: Option<i64>,
}

impl Default for LinkDelay {
    fn default() -> Self {
        Self::new()
    }
}

impl LinkDelay {
    const WINDOW_US: i64 = 30_000_000;

    pub fn new() -> Self {
        Self {
            floor: i64::MAX,
            window_min: i64::MAX,
            window_start: None,
        }
    }

    /// Microseconds this frame was later than the best case, or `None` when
    /// the sender gave no timestamp (`pts_us == 0`).
    pub fn observe(&mut self, arrival_us: i64, pts_us: u64) -> Option<u64> {
        if pts_us == 0 {
            return None;
        }
        let start = *self.window_start.get_or_insert(arrival_us);
        if arrival_us - start >= Self::WINDOW_US {
            self.floor = self.window_min;
            self.window_min = i64::MAX;
            self.window_start = Some(arrival_us);
        }
        let d = arrival_us - pts_us as i64;
        self.window_min = self.window_min.min(d);
        Some((d - self.floor.min(self.window_min)) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steady_arrivals_read_zero_and_a_stall_reads_as_the_extra_wait() {
        let mut l = LinkDelay::new();
        // A different clock origin must not matter: only differences count.
        let offset = 5_000_000_000;
        assert_eq!(l.observe(offset + 1_000, 1_000), Some(0));
        assert_eq!(l.observe(offset + 17_000, 17_000), Some(0));
        assert_eq!(l.observe(offset + 80_000, 33_000), Some(47_000));
    }

    #[test]
    fn missing_timestamp_is_not_a_measurement() {
        assert_eq!(LinkDelay::new().observe(10, 0), None);
    }

    #[test]
    fn the_floor_follows_the_clocks_instead_of_accumulating_drift() {
        let mut l = LinkDelay::new();
        l.observe(100, 50);
        // The clocks drift 4 ms apart. For one window that reads as delay...
        let w = LinkDelay::WINDOW_US as u64;
        assert_eq!(l.observe(w as i64 + 100, w + 100 - 50 - 4_000), Some(4_000));
        // ...and once the old window has rotated out, it is the new normal.
        assert_eq!(
            l.observe(2 * w as i64 + 200, 2 * w + 200 - 50 - 4_000),
            Some(0)
        );
    }
}
