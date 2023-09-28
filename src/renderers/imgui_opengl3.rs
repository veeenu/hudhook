use alloc::boxed::Box;
use core::ffi::{c_void, CString};
use core::sync::OnceLock;

use windows::core::{s, PCSTR};
use windows::Win32::Foundation::{FARPROC, HINSTANCE};
use windows::Win32::Graphics::OpenGL::wglGetProcAddress;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

static OPENGL3_LIB: OnceLock<HINSTANCE> = OnceLock::new();

unsafe fn get_opengl3_lib() -> HINSTANCE {
    LoadLibraryA(s!("opengl32.dll\0")).expect("LoadLibraryA").into()
}

/// # Safety
///
/// Undefined behaviour
pub unsafe fn get_proc_address(function_string: CString) -> *const c_void {
    let module = OPENGL3_LIB.get_or_init(|| get_opengl3_lib());

    if let Some(wgl_proc_address) = wglGetProcAddress(PCSTR(function_string.as_ptr() as _)) {
        wgl_proc_address as _
    } else {
        let proc_address: FARPROC = GetProcAddress(*module, PCSTR(function_string.as_ptr() as _));
        proc_address.unwrap() as _
    }
}
