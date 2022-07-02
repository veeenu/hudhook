use std::ffi::CString;
use std::ptr::{null, null_mut};

use log::*;
use simplelog::*;
use windows::core::PCSTR;
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
use windows::Win32::UI::WindowsAndMessaging::{FindWindowA, GetWindowThreadProcessId};

fn main() {
    simplelog::TermLogger::init(
        LevelFilter::Trace,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )
    .ok();
    let title = CString::new("DARK SOULS III").unwrap();
    let hwnd = unsafe { FindWindowA(None, PCSTR(title.as_ptr() as _)) };

    if hwnd.is_invalid() {
        error!("FindWindowA returned NULL: {}", unsafe { GetLastError().0 });
        return;
    }

    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid as *mut _ as _) };

    println!("{:?}", pid);

    let mut dll_path = std::env::current_exe().unwrap();
    dll_path.pop();
    dll_path.push("hook_you.dll");

    println!("{:?}", dll_path.canonicalize());

    let kernel32 = CString::new("Kernel32").unwrap();
    let loadlibraryw = CString::new("LoadLibraryW").unwrap();

    let proc_addr = unsafe {
        GetProcAddress(
            GetModuleHandleA(PCSTR(kernel32.as_ptr() as _)),
            PCSTR(loadlibraryw.as_ptr() as _),
        )
    };

    let dll_path =
        widestring::WideCString::from_os_str(dll_path.canonicalize().unwrap().as_os_str()).unwrap();

    let hproc = unsafe { OpenProcess(PROCESS_ALL_ACCESS, BOOL(0), pid) };
    let dllp = unsafe {
        VirtualAllocEx(
            hproc,
            null(),
            (MAX_PATH as usize) * std::mem::size_of::<u16>(),
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
            (MAX_PATH as usize) * std::mem::size_of::<u16>(),
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
    };

    unsafe {
        WaitForSingleObject(thread, INFINITE);
        let mut ec = 0u32;
        GetExitCodeThread(thread, &mut ec);
        CloseHandle(thread);
        VirtualFreeEx(hproc, dllp, 0, MEM_RELEASE);
        CloseHandle(hproc);
    };
}
