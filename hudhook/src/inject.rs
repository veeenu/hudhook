//! Facilities for injecting compiled DLLs into target processes.

use std::ffi::CString;
use std::mem::size_of;
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use log::*;
use widestring::U16CString;
use windows::core::{Error, Result, HRESULT, PCSTR, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, BOOL, MAX_PATH};
use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeThread, OpenProcess, WaitForSingleObject, PROCESS_ALL_ACCESS,
};
use windows::Win32::System::WindowsProgramming::INFINITE;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId};

/// Inject the DLL stored at `dll_path` in the process that owns the window with
/// title `title`.
pub fn inject(title: &str, dll_path: PathBuf) -> Result<()> {
    let title = U16CString::from_str(title).unwrap();
    let hwnd = unsafe { FindWindowW(PCWSTR(null()), PCWSTR(title.as_ptr())) };

    if hwnd.0 == 0 {
        let last_error = unsafe { GetLastError() };
        return Err(Error::new(
            HRESULT(last_error.0 as _),
            format!("FindWindowW returned NULL: {}", last_error.0).into(),
        ));
    }

    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid as *mut _ as _) };

    println!("{:?}", pid);

    let kernel32 = CString::new("Kernel32").unwrap();
    let loadlibraryw = CString::new("LoadLibraryW").unwrap();

    let proc_addr = unsafe {
        GetProcAddress(
            GetModuleHandleA(PCSTR(kernel32.as_ptr() as _))?,
            PCSTR(loadlibraryw.as_ptr() as _),
        )
    };

    let dll_path =
        widestring::WideCString::from_os_str(dll_path.canonicalize().unwrap().as_os_str()).unwrap();

    let hproc = unsafe { OpenProcess(PROCESS_ALL_ACCESS, BOOL(0), pid) }?;
    let dllp = unsafe {
        VirtualAllocEx(
            hproc,
            null_mut(),
            (MAX_PATH as usize) * size_of::<u16>(),
            MEM_RESERVE | MEM_COMMIT,
            PAGE_READWRITE,
        )
    };

    let mut bytes_written = 0usize;
    let res = unsafe {
        WriteProcessMemory(
            hproc,
            dllp,
            dll_path.as_ptr() as *const std::ffi::c_void,
            (MAX_PATH as usize) * size_of::<u16>(),
            (&mut bytes_written) as *mut _,
        )
    };

    debug!("WriteProcessMemory: written {} bytes, returned {:x}", bytes_written, res.0);

    let thread = unsafe {
        CreateRemoteThread(
            hproc,
            null(),
            0,
            Some(std::mem::transmute(proc_addr)),
            dllp,
            0,
            null_mut(),
        )
    }?;

    unsafe {
        WaitForSingleObject(thread, INFINITE);
        let mut ec = 0u32;
        GetExitCodeThread(thread, &mut ec as *mut u32);
        CloseHandle(thread);
        VirtualFreeEx(hproc, dllp, 0, MEM_RELEASE);
        CloseHandle(hproc);
    };

    Ok(())
}
