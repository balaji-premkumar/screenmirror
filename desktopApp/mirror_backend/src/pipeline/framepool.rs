//! Reusable BGRA frame buffers for the decoder → OBS shared-memory path.
//!
//! Decoded frames are large (8 MB at 1080p, 33 MB at 4K); allocating one per
//! frame under load is what used to make the decode loop miss its deadline.
//! Buffers are handed out here and returned once the frame has been written.

use concurrent_queue::ConcurrentQueue;
use once_cell::sync::Lazy;

const POOL_SLOTS: usize = 8;
const PREALLOC: usize = 4;
const PREALLOC_BYTES: usize = 8 * 1024 * 1024; // 1080p BGRA

static FREE_QUEUE: Lazy<ConcurrentQueue<Vec<u8>>> = Lazy::new(|| {
    let q = ConcurrentQueue::bounded(POOL_SLOTS);
    for _ in 0..PREALLOC {
        let _ = q.push(Vec::with_capacity(PREALLOC_BYTES));
    }
    q
});

/// Take a buffer from the pool, or allocate one if the pool is dry.
/// The returned buffer is empty but retains its capacity.
pub fn acquire(min_capacity: usize) -> Vec<u8> {
    let mut buf = FREE_QUEUE
        .pop()
        .unwrap_or_else(|_| Vec::with_capacity(min_capacity));
    buf.clear();
    buf.reserve(min_capacity.saturating_sub(buf.capacity()));
    buf
}

/// Return a buffer to the pool. Dropped if the pool is already full.
pub fn release(mut buf: Vec<u8>) {
    buf.clear();
    let _ = FREE_QUEUE.push(buf);
}
