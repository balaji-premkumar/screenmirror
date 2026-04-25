#[cfg(target_os = "windows")]
use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::System::Memory::*,
};
use std::ptr;

pub struct WinShmem {
    handle: HANDLE,
    ptr: *mut u8,
    size: usize,
}

impl WinShmem {
    pub fn create(name: &str, size: usize) -> Result<Self> {
        let name_u16: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let handle = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                size as u32,
                PCWSTR(name_u16.as_ptr()),
            )?;

            if handle.is_invalid() {
                return Err(Error::from_win32());
            }

            let ptr = MapViewOfFile(
                handle,
                FILE_MAP_ALL_ACCESS,
                0,
                0,
                size,
            );

            if ptr.is_null() {
                let err = Error::from_win32();
                CloseHandle(handle);
                return Err(err);
            }

            Ok(WinShmem {
                handle,
                ptr: ptr as *mut u8,
                size,
            })
        }
    }

    pub fn ptr(&self) -> *mut u8 {
        self.ptr
    }

    pub fn size(&self) -> usize {
        self.size
    }
}

impl Drop for WinShmem {
    fn drop(&mut self) {
        unsafe {
            if !self.ptr.is_null() {
                let _ = UnmapViewOfFile(self.ptr as *const _);
            }
            if !self.handle.is_invalid() {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

unsafe impl Send for WinShmem {}
unsafe impl Sync for WinShmem {}
