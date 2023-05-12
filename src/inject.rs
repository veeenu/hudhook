//! Facilities for injecting compiled DLLs into target processes.

use std::ffi::{CStr, CString};
use std::mem::{self, size_of};
use std::path::PathBuf;
use std::ptr::null;

use tracing::debug;
use widestring::{U16CStr, U16CString};
use windows::core::{Error, Result, HRESULT, PCSTR, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, BOOL, HANDLE, MAX_PATH};
use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32First, Process32FirstW, Process32Next, Process32NextW,
    PROCESSENTRY32, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeThread, OpenProcess, WaitForSingleObject, INFINITE,
    PROCESS_ALL_ACCESS,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowA, FindWindowW, GetWindowThreadProcessId};

/// A process, open with the permissions appropriate for injection.
pub struct Process(HANDLE);

impl Process {
    /// Retrieve the process ID by window title, returning the first match, and
    /// open it with the appropriate permissions.
    pub fn by_title(title: &str) -> Result<Self> {
        get_process_by_title(title).map(Self)
    }

    /// Retrieve the process ID by executable name, returning the first match,
    /// and open it with the appropriate permissions.
    pub fn by_name(name: &str) -> Result<Self> {
        get_process_by_name(name).map(Self)
    }

    /// Inject the DLL in the process.
    pub fn inject(&self, dll_path: PathBuf) -> Result<()> {
        let kernel32 = CString::new("Kernel32").unwrap();
        let loadlibraryw = CString::new("LoadLibraryW").unwrap();

        let proc_addr = unsafe {
            GetProcAddress(
                GetModuleHandleA(PCSTR(kernel32.as_ptr() as _))?,
                PCSTR(loadlibraryw.as_ptr() as _),
            )
        };

        let dll_path =
            widestring::WideCString::from_os_str(dll_path.canonicalize().unwrap().as_os_str())
                .unwrap();
        let dll_path_buf = unsafe {
            VirtualAllocEx(
                self.0,
                None,
                (MAX_PATH as usize) * size_of::<u16>(),
                MEM_RESERVE | MEM_COMMIT,
                PAGE_READWRITE,
            )
        };

        let mut bytes_written = 0usize;
        let res = unsafe {
            WriteProcessMemory(
                self.0,
                dll_path_buf,
                dll_path.as_ptr() as *const std::ffi::c_void,
                (MAX_PATH as usize) * size_of::<u16>(),
                Some((&mut bytes_written) as *mut _),
            )
        };

        debug!("WriteProcessMemory: written {} bytes, returned {:x}", bytes_written, res.0);

        let thread = unsafe {
            CreateRemoteThread(
                self.0,
                None,
                0,
                Some(std::mem::transmute(proc_addr)),
                Some(dll_path_buf),
                0,
                None,
            )
        }?;

        unsafe {
            WaitForSingleObject(thread, INFINITE);
            let mut exit_code = 0u32;
            GetExitCodeThread(thread, &mut exit_code as *mut u32);
            CloseHandle(thread);
            VirtualFreeEx(self.0, dll_path_buf, 0, MEM_RELEASE);

            Ok(())
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
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
    GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut _));

    OpenProcess(PROCESS_ALL_ACCESS, BOOL(0), pid)
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
    GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut _));

    OpenProcess(PROCESS_ALL_ACCESS, BOOL(0), pid)
}

// Find process given the process name.
fn get_process_by_name(name: &str) -> Result<HANDLE> {
    if cfg!(target_arch = "x86") {
        unsafe { get_process_by_name32(name) }
    } else if cfg!(any(target_arch = "x86_64", target_arch = "aarch64")) {
        unsafe { get_process_by_name64(name) }
    } else {
        panic!("This architecture is not supported")
    }
}

// 32-bit implementation. Uses [`PROCESSENTRY32`].
unsafe fn get_process_by_name32(name: &str) -> Result<HANDLE> {
    let name = name.split_once('\0').map(|(t, _)| t).unwrap_or(name);
    let name = CString::new(name).unwrap();

    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
    let mut pe32 =
        PROCESSENTRY32 { dwSize: mem::size_of::<PROCESSENTRY32>() as u32, ..Default::default() };

    if !Process32First(snapshot, &mut pe32).as_bool() {
        CloseHandle(snapshot);
        return Err(Error::new(
            HRESULT(GetLastError().0 as _),
            "Process32First failed".to_string().into(),
        ));
    }

    let pid = loop {
        let proc_name = CStr::from_ptr(pe32.szExeFile.as_ptr() as *const i8);

        if proc_name == name.as_ref() {
            break Ok(pe32.th32ProcessID);
        }

        if !Process32Next(snapshot, &mut pe32).as_bool() {
            CloseHandle(snapshot);
            break Err(Error::new(
                HRESULT(0),
                format!("Process {} not found", name.to_string_lossy()).into(),
            ));
        }
    }?;

    CloseHandle(snapshot);

    OpenProcess(PROCESS_ALL_ACCESS, BOOL(0), pid)
}

// 64-bit implementation. Uses [`PROCESSENTRY32W`] and
// [`widestring::U16CString`].
unsafe fn get_process_by_name64(name: &str) -> Result<HANDLE> {
    let name = U16CString::from_str_truncate(name);

    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
    let mut pe32 =
        PROCESSENTRY32W { dwSize: mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };

    if !Process32FirstW(snapshot, &mut pe32).as_bool() {
        CloseHandle(snapshot);
        return Err(Error::new(
            HRESULT(GetLastError().0 as _),
            "Process32First failed".to_string().into(),
        ));
    }

    let pid = loop {
        let proc_name =
            U16CStr::from_ptr_truncate(pe32.szExeFile.as_ptr(), pe32.szExeFile.len()).unwrap();

        if proc_name == name {
            break Ok(pe32.th32ProcessID);
        }

        if !Process32NextW(snapshot, &mut pe32).as_bool() {
            CloseHandle(snapshot);
            break Err(Error::new(
                HRESULT(0),
                format!("Process {} not found", name.to_string_lossy()).into(),
            ));
        }
    }?;

    CloseHandle(snapshot);

    OpenProcess(PROCESS_ALL_ACCESS, BOOL(0), pid)
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::Duration;

    use super::*;

    #[test]
    #[ignore]
    fn test_get_process_by_title() {
        // Notepad doesn't expose its title anymore, so we ignore this test for the time
        // being.
        let mut child = Command::new("Notepad.exe").spawn().expect("Couldn't start notepad");
        std::thread::sleep(Duration::from_millis(500));

        let pbt32 =
            unsafe { get_process_by_title32("Untitled - Notepad\0I don't care about this stuff") };
        println!("{pbt32:?}");

        let pbt64 =
            unsafe { get_process_by_title64("Untitled - Notepad\0I don't care about this stuff") };
        println!("{pbt64:?}");

        child.kill().expect("Couldn't kill notepad");

        assert!(pbt32.is_ok());
        assert!(pbt64.is_ok());
    }

    #[test]
    fn test_get_process_by_name() {
        let mut child = Command::new("Notepad.exe").spawn().expect("Couldn't start notepad");
        std::thread::sleep(Duration::from_millis(500));

        let pbn32 = unsafe { get_process_by_name32("Notepad.exe\0I don't care about this stuff") };
        println!("{pbn32:?}");

        let pbn64 = unsafe { get_process_by_name64("Notepad.exe\0I don't care about this stuff") };
        println!("{pbn64:?}");

        child.kill().expect("Couldn't kill notepad");

        assert!(pbn32.is_ok());
        assert!(pbn64.is_ok());
    }
}
