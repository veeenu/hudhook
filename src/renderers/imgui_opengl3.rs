use std::ffi::{c_void, CString, OsStr};
use std::os::windows::prelude::OsStrExt;

use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use windows::core::{PCSTR, PCWSTR};
use windows::Win32::Foundation::{FARPROC, HINSTANCE};
use windows::Win32::Graphics::OpenGL::wglGetProcAddress;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

static mut OPENGL3_LIB: OnceCell<Mutex<HINSTANCE>> = OnceCell::new();

unsafe fn get_opengl3_lib() -> HINSTANCE {
    let mut opengl_wide_string: Vec<u16> = OsStr::new("opengl32.dll").encode_wide().collect();
    opengl_wide_string.push(0);

    LoadLibraryW(PCWSTR(opengl_wide_string.as_ptr() as _)).unwrap()
}

/// # Safety
///
/// Help me out lol
pub unsafe fn get_proc_address(function_string: CString) -> *const c_void {
    let module = OPENGL3_LIB.get_or_init(|| Mutex::new(get_opengl3_lib())).lock();

    if let Some(wgl_proc_address) = wglGetProcAddress(PCSTR(function_string.as_ptr() as _)) {
        wgl_proc_address as _
    } else {
        let proc_address: FARPROC = GetProcAddress(*module, PCSTR(function_string.as_ptr() as _));
        proc_address.unwrap() as _
    }
}
