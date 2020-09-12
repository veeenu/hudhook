use std::ffi::c_void;
use std::ptr::null_mut;

use winapi::um::{
  memoryapi::ReadProcessMemory, memoryapi::WriteProcessMemory, processthreadsapi::GetCurrentProcess,
};

/// Wraps CheatEngine's concept of pointer with nested offsets. Evaluates,
/// if the evaluation does not fail, to a mutable pointer of type `T`.
///
/// At runtime, it evaluates the final address of the chain by reading the
/// base pointer, then recursively reading the next memory address in the
/// chain at an offset from there. For example,
///
/// ```
/// PointerChain::<T>::new(&[a, b, c, d, e])
/// ```
///
/// evaluates to
///
/// ```
/// *(*(*(*(*a + b) + c) + d) + e)
/// ```
///
/// This is useful for managing reverse engineered structures which are not
/// fully known.
pub struct PointerChain<T> {
  proc: *const c_void,
  base: *mut T,
  offsets: Vec<usize>,
}

impl<T> PointerChain<T> {
  /// Creates a new pointer chain given an array of addresses.
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
        &mut value as *mut usize as _,
        std::mem::size_of::<usize>(),
        null_mut(),
      )
    };

    match result {
      0 => None,
      _ => Some(value + offs as usize),
    }
  }

  /// Safely evaluates the pointer chain.
  /// Relies on `ReadProcessMemory` instead of pointer dereferencing for crash
  /// safety.  Returns `None` if the evaluation failed.
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
