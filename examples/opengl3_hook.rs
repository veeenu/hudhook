#![feature(lazy_cell)]

use hudhook::hooks::opengl3::ImguiOpenGl3Hooks;
use hudhook::hooks::{ImguiRenderLoop, ImguiRenderLoopFlags};
use imgui::Condition;
struct HookYou;

impl HookYou {
    fn new() -> Self {
        println!("Initializing");
        hudhook::utils::alloc_console();
        #[cfg(feature = "simplelog")]
        hudhook::utils::simplelog();

        HookYou
    }
}

impl ImguiRenderLoop for HookYou {
    fn render(&mut self, ui: &mut imgui::Ui, _: &ImguiRenderLoopFlags) {
        ui.window("Hello world").size([300.0, 110.0], Condition::FirstUseEver).build(|| {
            ui.text("Hello world!");
            ui.text("こんにちは世界！");
            ui.text("This...is...imgui-rs!");
            ui.separator();
            let mouse_pos = ui.io().mouse_pos;
            ui.text(format!("Mouse Position: ({:.1},{:.1})", mouse_pos[0], mouse_pos[1]));
        });
    }
}

use hudhook::reexports::*;
use hudhook::*;
use tracing::trace;
/// Entry point created by the `hudhook` library.
#[no_mangle]
pub unsafe extern "stdcall" fn DllMain(hmodule: HINSTANCE, reason: u32, _: *mut std::ffi::c_void) {
    if reason == DLL_PROCESS_ATTACH {
        hudhook::lifecycle::global_state::set_module(hmodule);
        let (non_blocking, _guard) = tracing_appender::non_blocking(std::io::stdout());
        tracing_subscriber::fmt().with_writer(non_blocking).init();

        trace!("DllMain()");
        std::thread::spawn(move || {
            let hooks: Box<dyn hooks::Hooks> = { HookYou::new().into_hook::<ImguiOpenGl3Hooks>() };
            hooks.hook();
            hudhook::lifecycle::global_state::set_hooks(hooks);
        });
    }
}
