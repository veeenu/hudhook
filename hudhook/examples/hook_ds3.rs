use std::ffi::CString;
use std::ptr::null_mut;

use hudhook::mh::LPVOID;

use log::*;
use simplelog::*;
use winapi::shared::minwindef::{DWORD, MAX_PATH};
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::handleapi::CloseHandle;
use winapi::um::libloaderapi::{GetModuleHandleA, GetProcAddress};
use winapi::um::minwinbase::LPSECURITY_ATTRIBUTES;
use winapi::um::synchapi::WaitForSingleObject;
use winapi::um::winbase::INFINITE;
use winapi::um::winnt::{MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, PROCESS_ALL_ACCESS};
use winapi::um::winuser::{FindWindowA, GetWindowThreadProcessId};
use winapi::um::{memoryapi, processthreadsapi};

fn main() {
    simplelog::TermLogger::init(
        LevelFilter::Trace,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )
    .ok();
    let title = CString::new("DARK SOULS III").unwrap();
    let hwnd = unsafe { FindWindowA(null_mut(), title.as_ptr() as *const i8) };

    if hwnd == null_mut() {
        error!("FindWindowA returned NULL: {}", unsafe { GetLastError() });
        return;
    }

    let mut pid: DWORD = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid as *mut _ as _) };

    println!("{:?}", pid);

    let mut dll_path = std::env::current_exe().unwrap();
    dll_path.pop();
    dll_path.push("hook_you.dll");

    println!("{:?}", dll_path.canonicalize());

    let kernel32 = CString::new("Kernel32").unwrap();
    let loadlibraryw = CString::new("LoadLibraryW").unwrap();

    let proc_addr =
        unsafe { GetProcAddress(GetModuleHandleA(kernel32.as_ptr()), loadlibraryw.as_ptr()) };

    let dll_path =
        widestring::WideCString::from_os_str(dll_path.canonicalize().unwrap().as_os_str()).unwrap();

    let hproc = unsafe { processthreadsapi::OpenProcess(PROCESS_ALL_ACCESS, 0, pid) };
    let dllp = unsafe {
        memoryapi::VirtualAllocEx(
            hproc,
            0 as LPVOID,
            MAX_PATH * std::mem::size_of::<u16>(),
            MEM_RESERVE | MEM_COMMIT,
            PAGE_READWRITE,
        )
    };

    let mut bytes_written = 0usize;
    let res = unsafe {
        memoryapi::WriteProcessMemory(
            hproc,
            dllp,
            dll_path.as_ptr() as *const std::ffi::c_void,
            MAX_PATH * std::mem::size_of::<u16>(),
            (&mut bytes_written) as *mut _,
        )
    };

    debug!(
        "WriteProcessMemory: written {} bytes, returned {:x}",
        bytes_written, res
    );

    let thread = unsafe {
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

    unsafe {
        WaitForSingleObject(thread, INFINITE);
        let mut ec = 0u32;
        processthreadsapi::GetExitCodeThread(thread, &mut ec as *mut DWORD);
        CloseHandle(thread);
        memoryapi::VirtualFreeEx(hproc, dllp, 0, MEM_RELEASE);
        CloseHandle(hproc);
    };
}
