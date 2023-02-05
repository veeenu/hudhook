use std::ffi::{c_void, CString, OsStr};
use std::os::windows::prelude::OsStrExt;

use windows::core::{PCSTR, PCWSTR};
use windows::Win32::Foundation::{FARPROC, PROC};
use windows::Win32::Graphics::OpenGL::wglGetProcAddress;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

pub unsafe fn get_proc_address(function_string: &str) -> *const c_void {
    let mut opengl_wide_string: Vec<u16> = OsStr::new("opengl32.dll").encode_wide().collect();
    opengl_wide_string.push(0);

    let module = LoadLibraryW(PCWSTR(opengl_wide_string.as_ptr())).unwrap();

    let function_c_string = CString::new(function_string).unwrap();
    let function_bytes_with_nul: &[u8] = function_c_string.as_bytes_with_nul();

    let function_name_ptr: *const u8 = function_bytes_with_nul.as_ptr() as *const u8;

    let wgl_proc_address: PROC = wglGetProcAddress(PCSTR(function_name_ptr));

    if wgl_proc_address.is_none() {
        let proc_address: FARPROC = GetProcAddress(module, PCSTR(function_name_ptr));
        proc_address.unwrap() as _
    } else {
        wgl_proc_address.unwrap() as _
    }
}
