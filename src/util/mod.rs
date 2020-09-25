use std::ffi::CString;
use std::ptr::null_mut;

use log::*;
use winapi::{
  shared::minwindef::{HMODULE, MAX_PATH},
  um::errhandlingapi::GetLastError,
  um::libloaderapi::{
    GetModuleFileNameA, GetModuleHandleExA, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
  },
};

/// Trivial to-string implementation of an error.
///
/// Will be superseded by a more articulate structure when the need will arise.
#[derive(Debug)]
pub struct Error(pub String);

impl From<String> for Error {
  fn from(s: String) -> Error {
    Error(s)
  }
}

impl std::fmt::Display for Error {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", self.0)
  }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Returns the path of the implementor's DLL.
pub fn get_dll_path() -> Option<String> {
  let mut hmodule: HMODULE = null_mut();
  // SAFETY
  // This is reckless, but it should never fail, and if it does, it's ok to crash and burn.
  let gmh_result = unsafe {
    GetModuleHandleExA(
      GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT | GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
      "DllMain".as_ptr() as _,
      &mut hmodule,
    )
  };

  if gmh_result == 0 {
    error!("get_dll_path: GetModuleHandleExA error: {:x}", unsafe {
      GetLastError()
    },);
    return None;
  }

  let mut sz_filename = [0u8; MAX_PATH];
  // SAFETY
  // pointer to sz_filename always defined and MAX_PATH bounds manually checked
  let len =
    unsafe { GetModuleFileNameA(hmodule, sz_filename.as_mut_ptr() as _, MAX_PATH as _) } as usize;

  Some(String::from_utf8_lossy(&sz_filename[..len]).to_string())
}

#[repr(C)]
pub(crate) struct VERTEX_CONSTANT_BUFFER(pub [[f32; 4]; 4]);

/// A reckless implementation of a conversion from
/// a string to raw C char data. Pls only use with
/// static const strings.

pub(crate) unsafe fn reckless_string(s: &str) -> CString {
  CString::new(s).unwrap()
}

/// Convert pointer to ref, emit error if null

pub(crate) fn ptr_as_ref<'a, T>(ptr: *const T) -> Result<&'a T> {
  match unsafe { ptr.as_ref() } {
    Some(t) => Ok(t),
    None => Err(format!("Null pointer").into()),
  }
}

pub(crate) mod bmh;
