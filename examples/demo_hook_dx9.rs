use hudhook::*;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

pub fn setup_tracing() {
    tracing_subscriber::registry()
        .with(
            fmt::layer().event_format(
                fmt::format()
                    .with_level(true)
                    .with_thread_ids(true)
                    .with_file(true)
                    .with_line_number(true)
                    .with_thread_names(true),
            ),
        )
        .with(EnvFilter::from_default_env())
        .init();
}

pub struct HookExample(bool);

impl ImguiRenderLoop for HookExample {
    fn render(&mut self, ui: &mut imgui::Ui) {
        ui.show_demo_window(&mut self.0);
    }
}

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
        setup_tracing();
        alloc_console().unwrap();
        ::hudhook::tracing::trace!("DllMain()");
        ::std::thread::spawn(move || {
            if let Err(e) = ::hudhook::Hudhook::builder()
                .with::<hooks::dx9::ImguiDx9Hooks>(HookExample(true))
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
