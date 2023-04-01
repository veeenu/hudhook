#![feature(once_cell)]
#![allow(non_snake_case, unused_variables, unreachable_patterns)]

use std::ffi::c_void;
use std::sync::LazyLock;

use windows::core::{GUID, HRESULT, PCSTR};
use windows::s;
use windows::Win32::Foundation::{BOOL, HINSTANCE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

static ENTRY_POINT: LazyLock<DirectInput8Create> = LazyLock::new(|| unsafe {
    let handle = LoadLibraryA(s!("C:\\Windows\\System32\\dinput8.dll\0")).unwrap();
    std::mem::transmute(GetProcAddress(handle, PCSTR(b"DirectInput8Create\0".as_ptr())).unwrap())
});

type DirectInput8Create = unsafe extern "stdcall" fn(
    hinst: HINSTANCE,
    dwversion: u32,
    riidltf: *const GUID,
    ppvout: *mut *mut c_void,
    punkouter: HINSTANCE,
) -> HRESULT;

#[no_mangle]
#[export_name = "DirectInput8Create"]
unsafe extern "stdcall" fn direct_input8_create(
    hinst: HINSTANCE,
    dwversion: u32,
    riidltf: *const GUID,
    ppvout: *mut *mut c_void,
    punkouter: HINSTANCE,
) -> HRESULT {
    return ENTRY_POINT(hinst, dwversion, riidltf, ppvout, punkouter);
}

#[no_mangle]
extern "stdcall" fn DllMain(
    hmodule: HINSTANCE,
    ul_reason_for_call: u32,
    lpreserved: *mut c_void,
) -> BOOL {
    match ul_reason_for_call {
        DLL_PROCESS_ATTACH => (),
        DLL_PROCESS_DETACH => (),
        _ => (),
    }
    BOOL::from(true)
}
