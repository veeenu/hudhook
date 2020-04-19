use std::ffi::c_void;
use std::ptr::null_mut;

use winapi::um::{
  libloaderapi::GetModuleHandleA,
  memoryapi::ReadProcessMemory,
  memoryapi::WriteProcessMemory,
  processthreadsapi::GetCurrentProcess,
};

pub fn get_base_address<S>() -> *const S {
  unsafe { GetModuleHandleA(null_mut()) as _ }
}

pub struct PointerChain<T> {
  proc: *const c_void,
  base: *mut T,
  offsets: Vec<isize>
}

impl<T> PointerChain<T> {

  pub fn new(chain: Vec<isize>) -> PointerChain<T> {
    let mut it = chain.into_iter();
    let base = it.next().unwrap() as usize as *mut T;
    PointerChain {
      proc: unsafe { GetCurrentProcess() },
      base,
      offsets: it.collect()
    }
  }

  pub fn eval(&self) -> Option<*mut T> {
    self.offsets.iter()
      .fold(Some(self.base as *const u8), |addr, offs| {
        if let Some(addr) = addr {
          Some(unsafe { std::ptr::read::<*const u8>(addr as _).offset(*offs) })
        } else {
          None
        }
      })
      .and_then(|addr| Some(addr as *mut T))
  }

  pub fn read(&self) -> Option<T> {
    if let Some(ptr) = self.eval() {
      let mut value: T = unsafe { std::mem::zeroed() };
      let result = unsafe {
        ReadProcessMemory(
          self.proc as _,
          ptr as _,
          &mut value as *mut _ as _,
          std::mem::size_of::<T>(),
          null_mut()
        )
      };

      match result {
        0 => None,
        _ => Some(value)
      }
    } else {
      None
    }
  }

  pub fn write(&self, mut value: T) -> Option<()> {
    if let Some(ptr) = self.eval() {
      let result = unsafe {
        WriteProcessMemory(
          self.proc as _,
          ptr as _,
          &mut value as *mut _ as _,
          std::mem::size_of::<T>(),
          null_mut()
        )
      };

      match result {
        0 => None,
        _ => Some(())
      }
    } else {
      None
    }
  }
}