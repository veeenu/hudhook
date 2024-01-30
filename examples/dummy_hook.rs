use windows::core::PCSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxA, MB_OK};

#[no_mangle]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "stdcall" fn DllMain(_: HINSTANCE, reason: u32, _: *mut std::ffi::c_void) {
    if reason == DLL_PROCESS_ATTACH {
        std::thread::spawn(move || {
            MessageBoxA(HWND(0), PCSTR("Hello\0".as_ptr()), PCSTR("Hello\0".as_ptr()), MB_OK)
        });
    }
}
