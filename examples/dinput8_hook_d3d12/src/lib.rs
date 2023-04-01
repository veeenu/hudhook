#![feature(once_cell)]
#![allow(non_snake_case, unused_variables, unreachable_patterns)]

use std::ffi::c_void;
use std::sync::LazyLock;
use std::thread;

use hudhook::hooks::dx12::ImguiDx12Hooks;
use hudhook::hooks::{self, ImguiRenderLoop, ImguiRenderLoopFlags};
use hudhook::log::trace;
use hudhook::reexports::DLL_PROCESS_ATTACH;
use hudhook::renderers::imgui_dx12::imgui;
use imgui::Condition;
use simplelog::{Color, ColorChoice, ConfigBuilder, Level, LevelFilter, TermLogger, TerminalMode};
use windows::core::{GUID, HRESULT, PCSTR};
use windows::s;
use windows::Win32::Foundation::{BOOL, HINSTANCE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
use windows::Win32::UI::WindowsAndMessaging::{LoadCursorW, SetCursor, IDC_ARROW};
struct Dx12HookExample;

impl Dx12HookExample {
    fn new() -> Self {
        hudhook::utils::alloc_console();
        let config = ConfigBuilder::new()
            .set_level_color(Level::Error, Some(Color::Magenta))
            .set_level_color(Level::Trace, Some(Color::Green))
            .set_target_level(LevelFilter::Error)
            .build();
        TermLogger::init(LevelFilter::Error, config, TerminalMode::Mixed, ColorChoice::Auto).ok();

        Dx12HookExample
    }
}

impl ImguiRenderLoop for Dx12HookExample {
    fn should_block_messages(&self, _io: &mut imgui::Io) -> bool {
        if _io.want_capture_mouse {
            unsafe {
                if let Ok(e) = LoadCursorW(None, IDC_ARROW) {
                    SetCursor(e);
                } else {
                    SetCursor(None);
                }
            }
            _io.mouse_draw_cursor = true;
            return true;
        } else {
            _io.mouse_draw_cursor = false;
            return false;
        }
    }

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
        DLL_PROCESS_ATTACH => {
            thread::spawn(move || {
                thread::sleep(std::time::Duration::from_secs(5));
                // commandqueue nullptr
                hudhook::lifecycle::global_state::set_module(hmodule);
                trace!("DllMain()");
                thread::spawn(move || {
                    let hooks: Box<dyn hooks::Hooks> =
                        { Dx12HookExample::new().into_hook::<ImguiDx12Hooks>() };
                    unsafe { hooks.hook() };
                    hudhook::lifecycle::global_state::set_hooks(hooks);
                });
            });
        },
        DLL_PROCESS_DETACH => (),
        _ => (),
    }

    BOOL::from(true)
}
