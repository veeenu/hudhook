use std::ffi::c_void;
use std::ptr::null_mut;

use winapi::um::{
  memoryapi::ReadProcessMemory, memoryapi::WriteProcessMemory, processthreadsapi::GetCurrentProcess,
};

pub struct PointerChain<T> {
  proc: *const c_void,
  base: *mut T,
  offsets: Vec<usize>,
}

impl<T> PointerChain<T> {
  pub fn new(chain: &[usize]) -> PointerChain<T> {
    let mut it = chain.iter();
    let base = *it.next().unwrap() as usize as *mut T;
    PointerChain {
      proc: unsafe { GetCurrentProcess() },
      base,
      offsets: it.map(|x| *x).collect(),
    }
  }

  fn safe_read(&self, addr: usize, offs: usize) -> Option<usize> {
    let mut value: usize = unsafe { std::mem::zeroed() };
    let result = unsafe {
      ReadProcessMemory(
        self.proc as _,
        addr as _,
        &mut value as *mut _ as _,
        std::mem::size_of::<T>(),
        null_mut(),
      )
    };

    match result {
      0 => None,
      _ => Some(value + offs as usize),
    }
  }

  pub fn eval(&self) -> Option<*mut T> {
    self
      .offsets
      .iter()
      .fold(Some(self.base as usize), |addr, &offs| {
        if let Some(addr) = addr {
          // Some(unsafe { std::ptr::read::<*const u8>(addr as _).offset(*offs as isize) })
          self.safe_read(addr, offs)
        } else {
          None
        }
      })
      .and_then(|addr| Some(addr as *mut T))
  }

  /// Evaluates the pointer chain and attempts to read the datum.
  /// Returns `None` if either the evaluation or the read failed.
  pub fn read(&self) -> Option<T> {
    if let Some(ptr) = self.eval() {
      let mut value: T = unsafe { std::mem::zeroed() };
      let result = unsafe {
        ReadProcessMemory(
          self.proc as _,
          ptr as _,
          &mut value as *mut _ as _,
          std::mem::size_of::<T>(),
          null_mut(),
        )
      };

      match result {
        0 => None,
        _ => Some(value),
      }
    } else {
      None
    }
  }

  /// Evaluates the pointer chain and attempts to write the datum.
  /// Returns `None` if either the evaluation or the write failed.
  pub fn write(&self, mut value: T) -> Option<()> {
    if let Some(ptr) = self.eval() {
      let result = unsafe {
        WriteProcessMemory(
          self.proc as _,
          ptr as _,
          &mut value as *mut _ as _,
          std::mem::size_of::<T>(),
          null_mut(),
        )
      };

      match result {
        0 => None,
        _ => Some(()),
      }
    } else {
      None
    }
  }
}
