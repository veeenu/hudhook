//! Facilities for injecting compiled DLLs into target processes.

use std::ffi::c_void;
use std::mem::{self, size_of};
use std::path::PathBuf;

use tracing::debug;
use windows::core::{s, w, Error, Result, HRESULT, HSTRING, PCSTR, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, MAX_PATH};
use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32First, Process32FirstW, Process32Next, Process32NextW,
    PROCESSENTRY32, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
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
        let proc_addr =
            unsafe { GetProcAddress(GetModuleHandleW(w!("Kernel32"))?, s!("LoadLibraryW")) };

        let dll_path = HSTRING::from(dll_path.canonicalize().unwrap().as_path());
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
                dll_path.as_ptr() as *const c_void,
                (MAX_PATH as usize) * size_of::<u16>(),
                Some(&mut bytes_written),
            )
        };

        debug!("WriteProcessMemory: written {} bytes, returned {:?}", bytes_written, res);

        let thread = unsafe {
            CreateRemoteThread(
                self.0,
                None,
                0,
                proc_addr.map(|proc_addr| {
                    mem::transmute::<
                        unsafe extern "system" fn() -> isize,
                        unsafe extern "system" fn(*mut c_void) -> u32,
                    >(proc_addr)
                }),
                Some(dll_path_buf),
                0,
                None,
            )
        }?;

        unsafe {
            WaitForSingleObject(thread, INFINITE);
            let mut exit_code = 0u32;
            GetExitCodeThread(thread, &mut exit_code as *mut u32)?;
            CloseHandle(thread)?;
            VirtualFreeEx(self.0, dll_path_buf, 0, MEM_RELEASE)?;

            Ok(())
        }
    }

    /// Retrieve the process handle.
    pub fn handle(&self) -> HANDLE {
        self.0
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0).expect("CloseHandle") };
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
    let title = HSTRING::from(title).to_os_string();
    let hwnd = FindWindowA(None, PCSTR(title.as_encoded_bytes().as_ptr()));

    if hwnd.0 == 0 {
        return Err(Error::from_win32());
    }

    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));

    OpenProcess(PROCESS_ALL_ACCESS, BOOL(0), pid)
}

// 64-bit implementation. Uses [`widestring::U16CString`] and `FindWindowW`.
unsafe fn get_process_by_title64(title: &str) -> Result<HANDLE> {
    let title = HSTRING::from(title);
    let hwnd = FindWindowW(None, PCWSTR(title.as_ptr()));

    if hwnd.0 == 0 {
        return Err(Error::from_win32());
    }

    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));

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
unsafe fn get_process_by_name32(name_str: &str) -> Result<HANDLE> {
    let name = HSTRING::from(name_str).to_os_string();
    let name = name.as_encoded_bytes();

    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
    let mut pe32 =
        PROCESSENTRY32 { dwSize: mem::size_of::<PROCESSENTRY32>() as u32, ..Default::default() };

    if Process32First(snapshot, &mut pe32).is_err() {
        CloseHandle(snapshot)?;
        return Err(Error::from_win32());
    }

    let pid = loop {
        let zero_idx = pe32.szExeFile.iter().position(|&x| x == 0).unwrap_or(pe32.szExeFile.len());
        let proc_name = &pe32.szExeFile[..zero_idx];

        if proc_name.iter().zip(name.iter()).fold(true, |v, (&a, &b)| v && (a as u8 == b)) {
            break Ok(pe32.th32ProcessID);
        }

        if Process32Next(snapshot, &mut pe32).is_err() {
            CloseHandle(snapshot)?;
            break Err(Error::from_hresult(HRESULT(-1)));
        }
    }?;

    CloseHandle(snapshot)?;

    OpenProcess(PROCESS_ALL_ACCESS, BOOL(0), pid)
}

// 64-bit implementation. Uses [`PROCESSENTRY32W`].
unsafe fn get_process_by_name64(name_str: &str) -> Result<HANDLE> {
    let name = HSTRING::from(name_str);

    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
    let mut pe32 =
        PROCESSENTRY32W { dwSize: mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };

    if Process32FirstW(snapshot, &mut pe32).is_err() {
        CloseHandle(snapshot)?;
        return Err(Error::from_win32());
    }

    let pid = loop {
        let zero_idx = pe32.szExeFile.iter().position(|&x| x == 0).unwrap_or(pe32.szExeFile.len());
        let proc_name = HSTRING::from_wide(&pe32.szExeFile[..zero_idx])?;

        if name == proc_name {
            break Ok(pe32.th32ProcessID);
        }

        if Process32NextW(snapshot, &mut pe32).is_err() {
            CloseHandle(snapshot)?;
            break Err(Error::from_hresult(HRESULT(-1)));
        }
    }?;

    CloseHandle(snapshot)?;

    OpenProcess(PROCESS_ALL_ACCESS, BOOL(0), pid)
}
