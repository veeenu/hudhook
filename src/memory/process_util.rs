use crate::util::bmh;

use std::ffi::c_void;
use std::ptr::null_mut;

use winapi::um::{
  libloaderapi::GetModuleHandleA, memoryapi::ReadProcessMemory,
  processthreadsapi::GetCurrentProcess,
};

pub(crate) fn get_base_address<S>() -> *const S {
  unsafe { GetModuleHandleA(null_mut()) as _ }
}

/// Returns the base address for the main process module.
///
/// Intended to be used as an utility along with
/// [`PointerChain`](struct.PointerChain.html).
pub fn base_address() -> usize {
  // unsafe { std::mem::transmute(get_base_address::<c_void>()) }
  get_base_address::<c_void>() as usize
}

/// Boyer-Moore implementation for [scanning array of bytes](https://wiki.cheatengine.org/index.php?title=Tutorials:AOBs).
///
/// Memory-safe, but still needs to be thoroughly tested. Tread carefully.
pub fn aob_scan(pattern: &str, start: usize, length: usize) -> Option<usize> {
  let mut buffer = vec![0u8; length];
  let mut n = length;
  unsafe {
    ReadProcessMemory(
      GetCurrentProcess(),
      start as _,
      buffer.as_mut_ptr() as _,
      length,
      &mut n as *mut _ as _,
    )
  };

  bmh::bmh(&buffer[0..n], &bmh::into_needle(pattern))
}

/*/// Create a thread from an evaluated function pointer.
///
/// A simple wrapper to `CreateThread`.
pub(crate) unsafe fn create_thread(func_ptr: usize) {
  log::debug!("CreateThread @ {:x}", func_ptr);

  CreateThread(
    null_mut(),
    0,
    std::mem::transmute(func_ptr),
    null_mut(),
    0,
    null_mut(),
  );
}*/
