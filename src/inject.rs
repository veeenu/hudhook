#![allow(clippy::needless_doctest_main)]

use crate::util::Error;

use std::ffi::{CStr, CString};
use std::mem;

use log::*;
use winapi::ctypes::*;
use winapi::shared::minwindef::*;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::handleapi::*;
use winapi::um::libloaderapi::{GetModuleHandleA, GetProcAddress};
use winapi::um::memoryapi;
use winapi::um::minwinbase::LPSECURITY_ATTRIBUTES;
use winapi::um::processthreadsapi;
use winapi::um::psapi;
use winapi::um::synchapi::WaitForSingleObject;
use winapi::um::winbase::INFINITE;
use winapi::um::winnt::*;

fn find_process(s: &str) -> Option<DWORD> {
  let mut lpid_process = [0u32; 65535];
  let mut cb_needed = 0u32;
  let mut ret = 0;

  unsafe {
    psapi::EnumProcesses(
      lpid_process.as_mut_ptr(),
      mem::size_of_val(&lpid_process) as DWORD,
      &mut cb_needed as *mut DWORD,
    );
  }

  let process_count = cb_needed as usize / mem::size_of::<DWORD>();
  trace!("{} processes", process_count);

  for (i, &pid) in lpid_process.iter().enumerate().take(process_count) {
    // let pid = lpid_process[i];
    let hproc = unsafe {
      processthreadsapi::OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid)
    };

    if hproc == std::ptr::null_mut::<c_void>() {
      trace!("GetLastError: {:x}", unsafe { GetLastError() });
      continue;
    }

    let mut hmodule = [0 as HMODULE; 64];
    let mut pmcb_needed = 0u32;
    let mut modname = [0i8; 128];
    unsafe {
      psapi::EnumProcessModules(
        hproc,
        hmodule.as_mut_ptr(),
        mem::size_of_val(&hmodule) as DWORD,
        &mut pmcb_needed as *mut DWORD,
      );
      psapi::GetModuleBaseNameA(
        hproc,
        hmodule[0],
        modname.as_mut_ptr(),
        mem::size_of_val(&modname) as DWORD,
      );
    }
    let mn = unsafe { CStr::from_ptr(modname.as_ptr()) }.to_string_lossy();
    trace!("find_process[{:>5}]: {} -> {}", i, mn, s);

    if mn == s {
      let mut mi = psapi::MODULEINFO {
        lpBaseOfDll: 0 as LPVOID,
        SizeOfImage: 0u32,
        EntryPoint: 0 as LPVOID,
      };
      unsafe {
        psapi::GetModuleInformation(
          hproc,
          hmodule[0],
          &mut mi as psapi::LPMODULEINFO,
          mem::size_of_val(&mi) as DWORD,
        );
      }
      ret = pid;
    }

    unsafe {
      CloseHandle(hproc);
    }
  }

  match ret {
    0 => None,
    i => Some(i),
  }
}

/// Inject `dll` inside the `process_name` process.
///
/// Create a bin target for your mod and use this function to build an
/// executable which launches your mod's dll.
///
/// Example:
/// ```no_run
/// use hudhook::prelude::inject;
///
/// pub fn main() {
///   inject("DarkSoulsIII.exe", "darksoulsiii-practice-tool.dll");
/// }
/// ```
pub fn inject(process_name: &str, dll: &str) -> Result<(), Error> {
  let pid: DWORD = find_process(process_name)
    .ok_or_else(|| Error(format!("Couldn't find process: {}", process_name)))?;
  let pathstr = std::fs::canonicalize(dll).map_err(|e| Error::from(format!("{:?}", e)))?;
  let mut path = [0i8; MAX_PATH];
  for (dest, src) in path.iter_mut().zip(
    CString::new(pathstr.to_str().unwrap())
      .unwrap()
      .into_bytes()
      .into_iter(),
  ) {
    *dest = src as _;
  }

  let hproc = unsafe { processthreadsapi::OpenProcess(PROCESS_ALL_ACCESS, 0, pid) };
  let dllp = unsafe {
    memoryapi::VirtualAllocEx(
      hproc,
      0 as LPVOID,
      MAX_PATH,
      MEM_RESERVE | MEM_COMMIT,
      PAGE_READWRITE,
    )
  };

  unsafe {
    memoryapi::WriteProcessMemory(
      hproc,
      dllp,
      std::mem::transmute(&path),
      MAX_PATH,
      std::ptr::null_mut::<usize>(),
    );
  }

  let thread = unsafe {
    let kernel32 = CString::new("Kernel32").unwrap();
    let loadlibrarya = CString::new("LoadLibraryA").unwrap();
    let proc_addr = GetProcAddress(GetModuleHandleA(kernel32.as_ptr()), loadlibrarya.as_ptr());
    processthreadsapi::CreateRemoteThread(
      hproc,
      0 as LPSECURITY_ATTRIBUTES,
      0,
      Some(std::mem::transmute(proc_addr)),
      dllp,
      0,
      std::ptr::null_mut::<DWORD>(),
    )
  };
  // println!("{:?}", thread);

  unsafe {
    WaitForSingleObject(thread, INFINITE);
    let mut ec = 0u32;
    processthreadsapi::GetExitCodeThread(thread, &mut ec as *mut DWORD);
    CloseHandle(thread);
    memoryapi::VirtualFreeEx(hproc, dllp, 0, MEM_RELEASE);
    CloseHandle(hproc);
  };

  Ok(())
}
