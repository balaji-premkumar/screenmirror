//! What the backend reports about itself: the event log and the throughput
//! counters. Both are read by the interface over FFI and by nothing else, so
//! they are grouped away from the code that does the work.

pub mod log;
pub mod metrics;
