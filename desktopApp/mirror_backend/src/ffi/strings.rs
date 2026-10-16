//! Converting strings across the C ABI.
//!
//! Two rules, both enforced here rather than repeated at every call site:
//!
//! 1. A null pointer in is a default value, never a panic. The interface can
//!    and does pass null for optional arguments.
//! 2. A string out never contains an interior NUL. `CString::new` rejects one,
//!    and the old code turned that rejection into an empty string — silently
//!    dropping the whole payload. Stripping the offending bytes instead keeps
//!    the rest, which matters when the payload is a device list or a log
//!    batch.

/// Reads a C string, substituting `fallback` for null or invalid UTF-8.
///
/// # Safety
///
/// `ptr` must be a valid NUL-terminated C string, or null.
pub(super) unsafe fn cstr_to_str(ptr: *const libc::c_char, fallback: &str) -> &str {
    if ptr.is_null() {
        return fallback;
    }
    std::ffi::CStr::from_ptr(ptr).to_str().unwrap_or(fallback)
}

/// Allocates a C string the caller must free with `free_string`.
///
/// Interior NUL bytes are removed rather than allowed to fail the conversion:
/// truncating at the NUL would hand back a silently short payload, and
/// returning nothing would drop a whole log batch because one message
/// contained a stray byte.
pub(super) fn to_c_string(s: impl Into<String>) -> *mut libc::c_char {
    let mut s = s.into();
    if s.as_bytes().contains(&0) {
        s = s.replace('\0', "");
    }
    // Unreachable after the strip above; an empty string keeps the contract
    // that the return value is always freeable.
    std::ffi::CString::new(s).unwrap_or_default().into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trips a pointer from `to_c_string` back into a Rust `String`,
    /// freeing it the way the interface does.
    fn round_trip(s: &str) -> String {
        let ptr = to_c_string(s);
        unsafe {
            let owned = std::ffi::CString::from_raw(ptr);
            owned.to_string_lossy().into_owned()
        }
    }

    #[test]
    fn plain_strings_survive_unchanged() {
        assert_eq!(
            round_trip("Accessory|Pixel 8|18d1:2d01"),
            "Accessory|Pixel 8|18d1:2d01"
        );
        assert_eq!(round_trip(""), "");
        assert_eq!(round_trip("unicode: 1080×1920 —"), "unicode: 1080×1920 —");
    }

    #[test]
    fn an_interior_nul_costs_one_byte_not_the_whole_payload() {
        // The old code returned an empty string here, so a single stray byte
        // anywhere in a log batch discarded every entry in it.
        assert_eq!(round_trip("before\0after"), "beforeafter");
    }

    #[test]
    fn a_null_pointer_reads_as_the_fallback() {
        unsafe {
            assert_eq!(cstr_to_str(std::ptr::null(), "default"), "default");
        }
    }

    #[test]
    fn a_valid_pointer_reads_as_its_contents() {
        let owned = std::ffi::CString::new("/home/user/app").unwrap();
        unsafe {
            assert_eq!(cstr_to_str(owned.as_ptr(), "fallback"), "/home/user/app");
        }
    }
}
