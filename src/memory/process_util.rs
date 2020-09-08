use crate::util::bmh;

use std::ffi::c_void;
use std::ptr::null_mut;

use winapi::um::{
  libloaderapi::GetModuleHandleA,
  memoryapi::ReadProcessMemory,
  processthreadsapi::{CreateThread, GetCurrentProcess},
};

pub(crate) fn get_base_address<S>() -> *const S {
  unsafe { GetModuleHandleA(null_mut()) as _ }
}

// To be used with PointerChain
pub fn base_address() -> isize {
  return unsafe { std::mem::transmute(get_base_address::<c_void>()) };
}

/// Boyer-Moore implementation
pub fn aob_scan(pattern: &str, start: isize, length: usize) -> Option<usize> {
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

pub fn create_thread(func_ptr: isize) {
  // let proc = unsafe { GetCurrentProcess() };
  log::info!("CreateThread @ {:x}", func_ptr);

  unsafe {
    CreateThread(
      null_mut(),
      0,
      std::mem::transmute(func_ptr),
      null_mut(),
      0,
      null_mut(),
    )
  };
}
