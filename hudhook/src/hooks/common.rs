use imgui::Key;
use parking_lot::MutexGuard;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{WHEEL_DELTA, WM_XBUTTONDBLCLK, XBUTTON1, *};

use super::{get_wheel_delta_wparam, hiword, loword};

pub(crate) type WndProcType =
    unsafe extern "system" fn(hwnd: HWND, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;

pub(crate) trait ImguiRendererInterface {
    fn io_mut(&mut self) -> &mut imgui::Io;
    fn get_focus_mut(&mut self) -> &mut bool;
    fn get_focus(&self) -> bool;
    fn get_wnd_proc(&self) -> WndProcType;

    fn setup_io(&mut self) {
        let mut io = ImguiRendererInterface::io_mut(self);

        io.nav_active = true;
        io.nav_visible = true;

        // Initialize keys
        io[Key::Tab] = VK_TAB.0 as _;
        io[Key::LeftArrow] = VK_LEFT.0 as _;
        io[Key::RightArrow] = VK_RIGHT.0 as _;
        io[Key::UpArrow] = VK_UP.0 as _;
        io[Key::DownArrow] = VK_DOWN.0 as _;
        io[Key::PageUp] = VK_PRIOR.0 as _;
        io[Key::PageDown] = VK_NEXT.0 as _;
        io[Key::Home] = VK_HOME.0 as _;
        io[Key::End] = VK_END.0 as _;
        io[Key::Insert] = VK_INSERT.0 as _;
        io[Key::Delete] = VK_DELETE.0 as _;
        io[Key::Backspace] = VK_BACK.0 as _;
        io[Key::Space] = VK_SPACE.0 as _;
        io[Key::Enter] = VK_RETURN.0 as _;
        io[Key::Escape] = VK_ESCAPE.0 as _;
        io[Key::A] = VK_A.0 as _;
        io[Key::C] = VK_C.0 as _;
        io[Key::V] = VK_V.0 as _;
        io[Key::X] = VK_X.0 as _;
        io[Key::Y] = VK_Y.0 as _;
        io[Key::Z] = VK_Z.0 as _;
    }
}

#[must_use]
pub(crate) fn imgui_wnd_proc_impl(
    hwnd: HWND,
    umsg: u32,
    WPARAM(wparam): WPARAM,
    LPARAM(lparam): LPARAM,
    mut imgui_renderer: MutexGuard<Box<impl ImguiRendererInterface>>,
) -> LRESULT {
    let mut io = imgui_renderer.io_mut();
    match umsg {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if wparam < 256 {
                io.keys_down[wparam as usize] = true;
            }
        },
        WM_KEYUP | WM_SYSKEYUP => {
            if wparam < 256 {
                io.keys_down[wparam as usize] = false;
            }
        },
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            io.mouse_down[0] = true;
        },
        WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => {
            io.mouse_down[1] = true;
        },
        WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => {
            io.mouse_down[2] = true;
        },
        WM_XBUTTONDOWN | WM_XBUTTONDBLCLK => {
            let btn = if hiword(wparam as _) == XBUTTON1.0 as u16 { 3 } else { 4 };
            io.mouse_down[btn] = true;
        },
        WM_LBUTTONUP => {
            io.mouse_down[0] = false;
        },
        WM_RBUTTONUP => {
            io.mouse_down[1] = false;
        },
        WM_MBUTTONUP => {
            io.mouse_down[2] = false;
        },
        WM_XBUTTONUP => {
            let btn = if hiword(wparam as _) == XBUTTON1.0 as u16 { 3 } else { 4 };
            io.mouse_down[btn] = false;
        },
        WM_MOUSEWHEEL => {
            let wheel_delta_wparam = get_wheel_delta_wparam(wparam as _);
            let wheel_delta = WHEEL_DELTA as f32;
            io.mouse_wheel += (wheel_delta_wparam as i16 as f32) / wheel_delta;
        },
        WM_MOUSEHWHEEL => {
            let wheel_delta_wparam = get_wheel_delta_wparam(wparam as _);
            let wheel_delta = WHEEL_DELTA as f32;
            io.mouse_wheel_h += (wheel_delta_wparam as i16 as f32) / wheel_delta;
        },
        WM_CHAR => io.add_input_character(wparam as u8 as char),
        WM_ACTIVATE => {
            if loword(wparam as _) == WA_INACTIVE as u16 {
                *imgui_renderer.get_focus_mut() = false;
            } else {
                *imgui_renderer.get_focus_mut() = true;
            }
            return LRESULT(1);
        },
        _ => {},
    };

    let wnd_proc = imgui_renderer.get_wnd_proc();
    drop(imgui_renderer);

    unsafe { CallWindowProcW(Some(wnd_proc), hwnd, umsg, WPARAM(wparam), LPARAM(lparam)) }
}
