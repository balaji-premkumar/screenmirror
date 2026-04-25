use shared_memory::{Shmem, ShmemConf};
use std::sync::atomic::{AtomicI32, Ordering, fence};
use anyhow::{Result, anyhow};

#[repr(C)]
pub struct FrameHeader {
    pub magic: [u8; 4],     // "MIRR"
    pub width: u32,
    pub height: u32,
    pub timestamp: u64,
    pub data_size: u32,
    pub _pad: [u8; 8],      // Pad to 32 bytes for alignment
}

#[repr(C)]
pub struct ShmControl {
    pub magic: [u8; 4],        // "MPRO" for Mirror Pro
    pub latest_index: AtomicI32, // -1 if no frame, 0, 1, or 2 for slots
    pub _pad: [u8; 56],        // pad to 64 bytes
}

pub const MAX_WIDTH: u32 = 3840;
pub const MAX_HEIGHT: u32 = 2160;
pub const MAX_FRAME_SIZE: usize = (MAX_WIDTH * MAX_HEIGHT * 4) as usize; // 4K BGRA
pub const SLOT_HEADER_SIZE: usize = 64; // Alignment for slot start
pub const SLOT_SIZE: usize = SLOT_HEADER_SIZE + MAX_FRAME_SIZE;
pub const TOTAL_SHM_SIZE: usize = 64 + (3 * SLOT_SIZE);

pub struct TripleBufferManager {
    shmem: Shmem,
}

impl TripleBufferManager {
    pub fn create(os_id: &str) -> Result<Self> {
        let shmem = ShmemConf::new()
            .size(TOTAL_SHM_SIZE)
            .os_id(os_id)
            .create()
            .or_else(|_| ShmemConf::new().os_id(os_id).open())
            .map_err(|e| anyhow!("Failed to create/open shared memory: {}", e))?;

        let mut mgr = TripleBufferManager { shmem };
        mgr.init_control();
        Ok(mgr)
    }

    fn init_control(&mut self) {
        let ptr = self.shmem.as_ptr();
        unsafe {
            let ctrl = &*(ptr as *const ShmControl);
            if ctrl.magic != *b"MPRO" {
                std::ptr::write_bytes(ptr, 0, 64);
                let ctrl_mut = &mut *(ptr as *mut ShmControl);
                ctrl_mut.magic = *b"MPRO";
                ctrl_mut.latest_index.store(-1, Ordering::SeqCst);
            }
        }
    }

    pub fn write_frame(&self, width: u32, height: u32, timestamp: u64, data: &[u8]) -> Result<i32> {
        if data.len() > MAX_FRAME_SIZE {
            return Err(anyhow!("Frame size too large: {}", data.len()));
        }

        let ptr = self.shmem.as_ptr();
        let ctrl = unsafe { &*(ptr as *const ShmControl) };
        
        // Pick next slot
        let current = ctrl.latest_index.load(Ordering::Acquire);
        let next_slot = ((current + 1) % 3).max(0) as usize;

        let slot_offset = 64 + (next_slot * SLOT_SIZE);
        let slot_ptr = unsafe { ptr.add(slot_offset) };

        // Write FrameHeader
        let header = FrameHeader {
            magic: *b"MIRR",
            width,
            height,
            timestamp,
            data_size: data.len() as u32,
            _pad: [0u8; 8],
        };

        unsafe {
            std::ptr::copy_nonoverlapping(
                &header as *const FrameHeader as *const u8,
                slot_ptr,
                std::mem::size_of::<FrameHeader>(),
            );
            
            // Write Pixel Data
            let data_ptr = slot_ptr.add(std::mem::size_of::<FrameHeader>());
            std::ptr::copy_nonoverlapping(data.as_ptr(), data_ptr, data.len());
        }

        // Standard memory barrier to ensure data is visible before index update
        fence(Ordering::Release);
        ctrl.latest_index.store(next_slot as i32, Ordering::Release);

        Ok(next_slot as i32)
    }
}
