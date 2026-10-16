use anyhow::{anyhow, Result};
use shared_memory::{Shmem, ShmemConf};
use std::sync::atomic::{fence, AtomicI32, AtomicU32, Ordering};

/// Per-slot frame header. Occupies the full 64-byte slot header so there is
/// no implicit compiler padding to disagree about between Rust and C.
///
/// Layout (must match `struct mpro_frame_header` in obs_plugin/mirror_source.c):
///   offset 0   magic      [u8;4]  "MIRR"
///   offset 4   seq        u32     per-slot seqlock (odd = write in progress)
///   offset 8   width      u32
///   offset 12  height     u32
///   offset 16  timestamp  u64
///   offset 24  data_size  u32
///   offset 28  _pad       [u8;36] reserved — pads header to 64 bytes
#[repr(C)]
pub struct FrameHeader {
    pub magic: [u8; 4],
    pub seq: u32,
    pub width: u32,
    pub height: u32,
    pub timestamp: u64,
    pub data_size: u32,
    pub _pad: [u8; 36],
}

const _: () = assert!(std::mem::size_of::<FrameHeader>() == 64);

#[repr(C)]
pub struct ShmControl {
    pub magic: [u8; 4],          // "MPRO" for Mirror Pro
    pub latest_index: AtomicI32, // -1 if no frame, 0, 1, or 2 for slots
    /// Changes every time the backend creates the segment afresh. The OBS
    /// plugin uses it to notice that it is holding a mapping from a previous
    /// run of the app and that its cached slot state is meaningless.
    pub session: u64,
    pub _pad: [u8; 48],
}

const _: () = assert!(std::mem::size_of::<ShmControl>() == 64);

/// A value that differs between runs. Not a security boundary — it only has
/// to change when the app restarts.
fn new_session_id() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
        | 1
}

pub const MAX_WIDTH: u32 = 3840;
pub const MAX_HEIGHT: u32 = 2160;
pub const MAX_FRAME_SIZE: usize = (MAX_WIDTH * MAX_HEIGHT * 4) as usize; // 4K BGRA
pub const SLOT_HEADER_SIZE: usize = 64;
pub const SLOT_SIZE: usize = SLOT_HEADER_SIZE + MAX_FRAME_SIZE;
pub const TOTAL_SHM_SIZE: usize = 64 + (3 * SLOT_SIZE);

pub struct TripleBufferManager {
    shmem: Shmem,
    last_written: std::sync::atomic::AtomicUsize,
}

// The mapping pointer is only written through synchronized seqlock protocol.
unsafe impl Send for TripleBufferManager {}
unsafe impl Sync for TripleBufferManager {}

impl TripleBufferManager {
    pub fn create(os_id: &str) -> Result<Self> {
        // Falling back to open() picks up a segment left behind by a crashed
        // run. That is fine only if it is at least as large as we expect — an
        // older/smaller segment would make write_frame() run off the end of
        // the mapping, so verify the size instead of trusting the name.
        let shmem = match ShmemConf::new().size(TOTAL_SHM_SIZE).os_id(os_id).create() {
            Ok(s) => s,
            Err(create_err) => {
                let existing = ShmemConf::new().os_id(os_id).open().map_err(|open_err| {
                    anyhow!("Failed to create ({create_err}) or open ({open_err}) shared memory")
                })?;
                if existing.len() < TOTAL_SHM_SIZE {
                    return Err(anyhow!(
                        "Existing shared memory segment is {} bytes, need {} — \
                         it is from an incompatible build. Close OBS and retry.",
                        existing.len(),
                        TOTAL_SHM_SIZE
                    ));
                }
                existing
            }
        };

        let mgr = TripleBufferManager {
            shmem,
            last_written: std::sync::atomic::AtomicUsize::new(2),
        };
        mgr.init_control();
        Ok(mgr)
    }

    fn init_control(&self) {
        let ptr = self.shmem.as_ptr();
        unsafe {
            // Always re-stamp: a segment inherited from a previous run has a
            // valid magic but stale slot contents, and a reader still holding
            // it needs to see that this is a different session.
            std::ptr::write_bytes(ptr, 0, 64);
            for slot in 0..3 {
                std::ptr::write_bytes(ptr.add(64 + slot * SLOT_SIZE), 0, SLOT_HEADER_SIZE);
            }
            let ctrl_mut = &mut *(ptr as *mut ShmControl);
            ctrl_mut.magic = *b"MPRO";
            ctrl_mut.session = new_session_id();
            ctrl_mut.latest_index.store(-1, Ordering::SeqCst);
        }
    }

    /// Write a frame into the next slot using a per-slot seqlock.
    ///
    /// The seqlock lets a reader detect that the writer lapped it mid-copy
    /// (the old design could tear when the writer wrapped around within one
    /// reader copy):
    ///   Writer: seq += 1 (odd) → write header + pixels → fence → seq += 1 (even)
    ///   Reader: read seq (skip if odd) → copy → fence → re-read seq → discard on mismatch
    pub fn write_frame(&self, width: u32, height: u32, timestamp: u64, data: &[u8]) -> Result<i32> {
        if data.len() > MAX_FRAME_SIZE {
            return Err(anyhow!("Frame size too large: {}", data.len()));
        }

        let ptr = self.shmem.as_ptr();
        let ctrl = unsafe { &*(ptr as *const ShmControl) };

        let next_slot = (self.last_written.load(Ordering::Relaxed) + 1) % 3;
        let slot_offset = 64 + (next_slot * SLOT_SIZE);
        let slot_ptr = unsafe { ptr.add(slot_offset) };

        unsafe {
            // Seqlock begin: mark slot as being written (odd)
            let seq_ptr = &*(slot_ptr.add(4) as *const AtomicU32);
            let seq = seq_ptr.load(Ordering::Relaxed);
            seq_ptr.store(seq.wrapping_add(1), Ordering::Release);
            fence(Ordering::Release);

            let header = FrameHeader {
                magic: *b"MIRR",
                seq: 0, // written via the atomic above; field kept for layout
                width,
                height,
                timestamp,
                data_size: data.len() as u32,
                _pad: [0u8; 36],
            };

            // Write header fields except seq (bytes 0..4 then 8..64)
            let hdr_bytes =
                std::slice::from_raw_parts(&header as *const FrameHeader as *const u8, 64);
            std::ptr::copy_nonoverlapping(hdr_bytes.as_ptr(), slot_ptr, 4);
            std::ptr::copy_nonoverlapping(hdr_bytes.as_ptr().add(8), slot_ptr.add(8), 56);

            // Pixel data
            let data_ptr = slot_ptr.add(SLOT_HEADER_SIZE);
            std::ptr::copy_nonoverlapping(data.as_ptr(), data_ptr, data.len());

            // Seqlock end: mark slot complete (even)
            fence(Ordering::Release);
            seq_ptr.store(seq.wrapping_add(2), Ordering::Release);
        }

        self.last_written.store(next_slot, Ordering::Relaxed);

        // Publish: readers pick up the newest complete slot
        fence(Ordering::Release);
        ctrl.latest_index.store(next_slot as i32, Ordering::Release);

        Ok(next_slot as i32)
    }
}
