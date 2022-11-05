//! Facilities for injecting compiled DLLs into target processes.

use std::ffi::{c_void, CString};
use std::mem::size_of;
use std::os::windows::prelude::OsStrExt;
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use log::*;
use widestring::U16CString;
use windows::core::{Error, Result, HRESULT, PCSTR, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, BOOL, HANDLE, MAX_PATH};
use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeThread, OpenProcess, WaitForSingleObject, PROCESS_ALL_ACCESS,
};
use windows::Win32::System::WindowsProgramming::INFINITE;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowA, FindWindowW, GetWindowThreadProcessId};

/// Inject the DLL stored at `dll_path` in the process that owns the window with
/// title `title`.
pub fn inject(title: &str, dll_path: PathBuf) -> Result<()> {
    let hproc = get_process_by_title(title)?;
    let proc_addr = unsafe {
        GetProcAddress(
            GetModuleHandleA(PCSTR("Kernel32\0".as_ptr()))?,
            PCSTR(
                if cfg!(target_arch = "x86") {
                    "LoadLibraryA\0"
                } else if cfg!(any(target_arch = "x86_64", target_arch = "aarch64")) {
                    "LoadLibraryW\0"
                } else {
                    panic!("This architecture is not supported")
                }
                .as_ptr(),
            ),
        )
    };

    let dll_path = dll_path.canonicalize().unwrap().as_os_str().encode_wide().collect::<Vec<_>>();

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
            dll_path.as_ptr().cast::<c_void>(),
            (dll_path.len() + 1) * size_of::<u16>(),
            (&mut bytes_written) as *mut _,
        )
    };

    debug!("WriteProcessMemory: written {} bytes, returned {:x}", bytes_written, res.0);

    unsafe {
        let thread = CreateRemoteThread(
            hproc,
            null(),
            0,
            Some(std::mem::transmute(proc_addr)),
            dllp,
            0,
            null_mut(),
        )?;
        WaitForSingleObject(thread, INFINITE);
        let mut exit_code = 0u32;
        GetExitCodeThread(thread, &mut exit_code as *mut u32);
        CloseHandle(thread);
        VirtualFreeEx(hproc, dllp, 0, MEM_RELEASE);
        CloseHandle(hproc);
    };

    Ok(())
}

// Find process given the title of one of its windows.
fn get_process_by_title(title: &str) -> Result<HANDLE> {
    if cfg!(target_arch = "x86") {
        unsafe { get_process_by_title32(title) }
    } else if cfg!(any(target_arch = "x86_64", target_arch = "aarch64")) {
        unsafe { get_process_by_title64(title) }
    } else {
        panic!("This architecture is not supported")
    }
}

// 64-bit implementation. Uses [`widestring::U16CString`] and `FindWindowW`.
unsafe fn get_process_by_title64(title: &str) -> Result<HANDLE> {
    let title = U16CString::from_str_truncate(title);
    let hwnd = FindWindowW(PCWSTR(null()), PCWSTR(title.as_ptr()));

    if hwnd.0 == 0 {
        let last_error = GetLastError();
        return Err(Error::new(
            HRESULT(last_error.0 as _),
            format!("FindWindowW returned NULL: {}", last_error.0).into(),
        ));
    }

    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, &mut pid as *mut _ as _);

    OpenProcess(PROCESS_ALL_ACCESS, BOOL(0), pid)
}

// 32-bit implementation. Uses [`std::ffi::CString`] and `FindWindowA`.
unsafe fn get_process_by_title32(title: &str) -> Result<HANDLE> {
    let title = title.split_once('\0').map(|(t, _)| t).unwrap_or(title);
    let title = CString::new(title).unwrap();
    let hwnd = FindWindowA(PCSTR(null()), PCSTR(title.as_bytes_with_nul().as_ptr()));

    if hwnd.0 == 0 {
        let last_error = GetLastError();
        return Err(Error::new(
            HRESULT(last_error.0 as _),
            format!("FindWindowA returned NULL: {}", last_error.0).into(),
        ));
    }

    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, &mut pid as *mut _ as _);

    OpenProcess(PROCESS_ALL_ACCESS, BOOL(0), pid)
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::Duration;

    use super::*;

    #[test]
    fn test_get_process_by_title() {
        let mut child = Command::new("notepad.exe").spawn().expect("Couldn't start notepad");
        std::thread::sleep(Duration::from_millis(500));

        let pbt32 =
            unsafe { get_process_by_title32("Untitled - Notepad\0I don't care about this stuff") };
        println!("{:?}", pbt32);

        let pbt64 =
            unsafe { get_process_by_title64("Untitled - Notepad\0I don't care about this stuff") };
        println!("{:?}", pbt64);

        child.kill().expect("Couldn't kill notepad");

        assert!(pbt32.is_ok());
        assert!(pbt64.is_ok());
    }
}
