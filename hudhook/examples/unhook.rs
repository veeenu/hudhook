use std::ffi::{CStr, CString};
use std::mem;
use std::ptr::{null, null_mut};

use hudhook::reexports::HINSTANCE;
use log::*;
use simplelog::*;
use windows::core::PCSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, MAX_PATH};
use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows::Win32::System::ProcessStatus::{K32EnumProcessModules, K32GetModuleBaseNameA};
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
    let freelibrary = CString::new("FreeLibrary").unwrap();

    let proc_addr = unsafe {
        GetProcAddress(
            GetModuleHandleA(PCSTR(kernel32.as_ptr() as _)),
            PCSTR(freelibrary.as_ptr() as _),
        )
    };

    let _dll_path =
        widestring::WideCString::from_os_str(dll_path.canonicalize().unwrap().as_os_str()).unwrap();

    let hproc = unsafe { OpenProcess(PROCESS_ALL_ACCESS, None, pid) };

    let mut hmodule = [HINSTANCE(0); 256];
    let mut modname = [0u8; 128];
    let mut pmcb_needed = 0u32;
    unsafe {
        K32EnumProcessModules(
            hproc,
            hmodule.as_mut_ptr(),
            mem::size_of_val(&hmodule) as u32,
            &mut pmcb_needed as *mut u32,
        );

        println!("{:?}", pmcb_needed);
        for i in 0..(pmcb_needed / mem::size_of::<HINSTANCE>() as u32) {
            K32GetModuleBaseNameA(hproc, hmodule[i as usize], &mut modname);
            let mn = CStr::from_ptr(modname.as_ptr() as _).to_string_lossy();
            println!("{:?}", mn);

            if mn == "hook_you.dll" {
                let ptr = VirtualAllocEx(
                    hproc,
                    null(),
                    std::mem::size_of::<HINSTANCE>(),
                    MEM_RESERVE | MEM_COMMIT,
                    PAGE_READWRITE,
                );

                let mut bytes_written = 0usize;
                let _res = WriteProcessMemory(
                    hproc,
                    ptr,
                    (&hmodule[i as usize]) as *const _ as *const std::ffi::c_void,
                    (MAX_PATH as usize) * std::mem::size_of::<u16>(),
                    (&mut bytes_written) as *mut _,
                );
                let thread = CreateRemoteThread(
                    hproc,
                    null(),
                    0,
                    Some(std::mem::transmute(proc_addr)),
                    ptr,
                    0,
                    null_mut(),
                );

                WaitForSingleObject(thread, INFINITE);
                let mut ec = 0u32;
                GetExitCodeThread(thread, &mut ec);
                CloseHandle(thread);
                VirtualFreeEx(hproc, ptr, 0, MEM_RELEASE);
                CloseHandle(hproc);
            }
        }
    }
}
