use hudhook::*;

mod support;

/// Entry point created by the `hudhook` library.
///
/// # Safety
///
/// haha
#[no_mangle]
pub unsafe extern "stdcall" fn DllMain(
    hmodule: ::hudhook::windows::Win32::Foundation::HINSTANCE,
    reason: u32,
    _: *mut ::std::ffi::c_void,
) {
    if reason == ::hudhook::windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH {
        support::setup_tracing();
        ::hudhook::tracing::trace!("DllMain()");
        ::std::thread::spawn(move || {
            if let Err(e) = ::hudhook::Hudhook::builder()
                .with::<hooks::opengl3::ImguiOpenGl3Hooks>(support::HookExample::new())
                .with_hmodule(hmodule)
                .build()
                .apply()
            {
                ::hudhook::tracing::error!("Couldn't apply hooks: {e:?}");
                ::hudhook::eject();
            }
        });
    }
}
