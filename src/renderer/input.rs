//! This module contains functions related to processing input events.

use std::ffi::c_void;
use std::mem::size_of;

use imgui::Io;
use parking_lot::MutexGuard;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTHEADER, RAWKEYBOARD, RAWMOUSE_0_0,
    RID_DEVICE_INFO_TYPE, RID_INPUT, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::renderer::{RenderEngine, RenderState};
use crate::ImguiRenderLoop;

pub type WndProcType =
    unsafe extern "system" fn(hwnd: HWND, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;

// Replication of the Win32 HIWORD macro.
#[inline]
fn hiword(l: u32) -> u16 {
    ((l >> 16) & 0xffff) as u16
}

////////////////////////////////////////////////////////////////////////////////
// Raw input
////////////////////////////////////////////////////////////////////////////////

// Handle raw mouse input events.
//
// Given the RAWINPUT structure, check each possible mouse flag status and
// update the Io object accordingly. Both the key_down indices associated to the
// mouse click (VK_...) and the values in mouse_down are updated.
fn handle_raw_mouse_input(io: &mut Io, raw_mouse: &RAWMOUSE_0_0) {
    let button_flags = raw_mouse.usButtonFlags as u32;

    let has_flag = |flag| button_flags & flag != 0;
    let mut set_key_down = |VIRTUAL_KEY(index), val: bool| io.keys_down[index as usize] = val;

    // Check whether any of the mouse buttons was pressed or released.
    if has_flag(RI_MOUSE_LEFT_BUTTON_DOWN) {
        set_key_down(VK_LBUTTON, true);
        io.mouse_down[0] = true;
    }
    if has_flag(RI_MOUSE_LEFT_BUTTON_UP) {
        set_key_down(VK_LBUTTON, false);
        io.mouse_down[0] = false;
    }
    if has_flag(RI_MOUSE_RIGHT_BUTTON_DOWN) {
        set_key_down(VK_RBUTTON, true);
        io.mouse_down[1] = true;
    }
    if has_flag(RI_MOUSE_RIGHT_BUTTON_UP) {
        set_key_down(VK_RBUTTON, false);
        io.mouse_down[1] = false;
    }
    if has_flag(RI_MOUSE_MIDDLE_BUTTON_DOWN) {
        set_key_down(VK_MBUTTON, true);
        io.mouse_down[2] = true;
    }
    if has_flag(RI_MOUSE_MIDDLE_BUTTON_UP) {
        set_key_down(VK_MBUTTON, false);
        io.mouse_down[2] = false;
    }
    if has_flag(RI_MOUSE_BUTTON_4_DOWN) {
        set_key_down(VK_XBUTTON1, true);
        io.mouse_down[3] = true;
    }
    if has_flag(RI_MOUSE_BUTTON_4_UP) {
        set_key_down(VK_XBUTTON1, false);
        io.mouse_down[3] = false;
    }
    if has_flag(RI_MOUSE_BUTTON_5_DOWN) {
        set_key_down(VK_XBUTTON2, true);
        io.mouse_down[4] = true;
    }
    if has_flag(RI_MOUSE_BUTTON_5_UP) {
        set_key_down(VK_XBUTTON2, false);
        io.mouse_down[4] = false;
    }

    // Apply vertical mouse scroll.
    if button_flags & RI_MOUSE_WHEEL != 0 {
        let wheel_delta = raw_mouse.usButtonData as i16 / WHEEL_DELTA as i16;
        io.mouse_wheel += wheel_delta as f32;
    }

    // Apply horizontal mouse scroll.
    if button_flags & RI_MOUSE_HWHEEL != 0 {
        let wheel_delta = raw_mouse.usButtonData as i16 / WHEEL_DELTA as i16;
        io.mouse_wheel_h += wheel_delta as f32;
    }
}

// Handle raw keyboard input.
fn handle_raw_keyboard_input(io: &mut Io, raw_keyboard: &RAWKEYBOARD) {
    // Ignore messages without a valid key code
    if raw_keyboard.VKey == 0 {
        return;
    }

    // Extract the keyboard flags.
    let flags = raw_keyboard.Flags as u32;

    // Compute the scan code, applying the prefix if it is present.
    let scan_code = {
        let mut code = raw_keyboard.MakeCode as u32;
        // Necessary to check LEFT/RIGHT keys on CTRL & ALT & others (not shift)
        if flags & RI_KEY_E0 != 0 {
            code |= 0xe000;
        }
        if flags & RI_KEY_E1 != 0 {
            code |= 0xe100;
        }
        code
    };

    // Check the key status.
    let is_key_down = flags == RI_KEY_MAKE;
    let is_key_up = flags & RI_KEY_BREAK != 0;

    // Map the virtual key if necessary.
    let virtual_key = match VIRTUAL_KEY(raw_keyboard.VKey) {
        virtual_key @ (VK_SHIFT | VK_CONTROL | VK_MENU) => {
            match unsafe { MapVirtualKeyA(scan_code, MAPVK_VSC_TO_VK_EX) } {
                0 => virtual_key.0,
                i => i as u16,
            }
        },
        VIRTUAL_KEY(virtual_key) => virtual_key,
    } as usize;

    // If the virtual key is in the allowed array range, set the appropriate status
    // of key_down for that virtual key.
    if virtual_key < 0xFF {
        if is_key_down {
            io.keys_down[virtual_key] = true;
        }
        if is_key_up {
            io.keys_down[virtual_key] = false;
        }
    }
}

// Handle WM_INPUT events.
fn handle_raw_input(io: &mut Io, WPARAM(wparam): WPARAM, LPARAM(lparam): LPARAM) {
    let mut raw_data = RAWINPUT { ..Default::default() };
    let mut raw_data_size = size_of::<RAWINPUT>() as u32;
    let raw_data_header_size = size_of::<RAWINPUTHEADER>() as u32;

    // Read the raw input data.
    let r = unsafe {
        GetRawInputData(
            HRAWINPUT(lparam),
            RID_INPUT,
            Some(&mut raw_data as *mut _ as *mut c_void),
            &mut raw_data_size,
            raw_data_header_size,
        )
    };

    // If GetRawInputData errors out, return false.
    if r == u32::MAX {
        return;
    }

    // Ignore messages when window is not focused.
    if (wparam as u32 & 0xFFu32) != RIM_INPUT {
        return;
    }

    // Dispatch to the appropriate raw input processing method.
    match RID_DEVICE_INFO_TYPE(raw_data.header.dwType) {
        RIM_TYPEMOUSE => {
            handle_raw_mouse_input(io, unsafe { &raw_data.data.mouse.Anonymous.Anonymous });
        },
        RIM_TYPEKEYBOARD => {
            handle_raw_keyboard_input(io, unsafe { &raw_data.data.keyboard });
        },
        _ => {},
    }
}

////////////////////////////////////////////////////////////////////////////////
// Regular input
////////////////////////////////////////////////////////////////////////////////

fn map_vkey(wparam: u16, lparam: usize) -> VIRTUAL_KEY {
    match VIRTUAL_KEY(wparam) {
        VK_SHIFT => unsafe {
            match MapVirtualKeyA(((lparam & 0x00ff0000) >> 16) as u32, MAPVK_VSC_TO_VK_EX) {
                0 => VIRTUAL_KEY(wparam),
                i => VIRTUAL_KEY(i as _),
            }
        },
        VK_CONTROL => {
            if lparam & 0x01000000 != 0 {
                VK_RCONTROL
            } else {
                VK_LCONTROL
            }
        },
        VK_MENU => {
            if lparam & 0x01000000 != 0 {
                VK_RMENU
            } else {
                VK_LMENU
            }
        },
        _ => VIRTUAL_KEY(wparam),
    }
}

// Handle WM_(SYS)KEYDOWN/WM_(SYS)KEYUP events.
fn handle_input(io: &mut Io, state: u32, WPARAM(wparam): WPARAM, LPARAM(lparam): LPARAM) {
    let pressed = (state == WM_KEYDOWN) || (state == WM_SYSKEYDOWN);
    let key_pressed = map_vkey(wparam as _, lparam as _);
    io.keys_down[key_pressed.0 as usize] = pressed;

    // According to the winit implementation [1], it's ok to check twice, and the
    // logic isn't flawed either.
    //
    // [1] https://github.com/imgui-rs/imgui-rs/blob/b1e66d050e84dbb2120001d16ce59d15ef6b5303/imgui-winit-support/src/lib.rs#L401-L404
    match key_pressed {
        VK_CONTROL | VK_LCONTROL | VK_RCONTROL => io.key_ctrl = pressed,
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => io.key_shift = pressed,
        VK_MENU | VK_LMENU | VK_RMENU => io.key_alt = pressed,
        VK_LWIN | VK_RWIN => io.key_super = pressed,
        _ => (),
    };
}

////////////////////////////////////////////////////////////////////////////////
// Window procedure
////////////////////////////////////////////////////////////////////////////////

#[must_use]
pub fn imgui_wnd_proc_impl<T>(
    hwnd: HWND,
    umsg: u32,
    WPARAM(wparam): WPARAM,
    LPARAM(lparam): LPARAM,
    wnd_proc: WndProcType,
    mut render_engine: MutexGuard<RenderEngine>,
    imgui_render_loop: T,
) -> LRESULT
where
    T: AsRef<dyn Send + Sync + ImguiRenderLoop + 'static>,
{
    let ctx = render_engine.ctx();
    let mut ctx = ctx.borrow_mut();
    let io = ctx.io_mut();
    match umsg {
        WM_INPUT => handle_raw_input(io, WPARAM(wparam), LPARAM(lparam)),
        state @ (WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP) if wparam < 256 => {
            handle_input(io, state, WPARAM(wparam), LPARAM(lparam))
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
            let btn = if hiword(wparam as _) == XBUTTON1 { 3 } else { 4 };
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
            let btn = if hiword(wparam as _) == XBUTTON1 { 3 } else { 4 };
            io.mouse_down[btn] = false;
        },
        WM_MOUSEWHEEL => {
            // This `hiword` call is equivalent to GET_WHEEL_DELTA_WPARAM
            let wheel_delta_wparam = hiword(wparam as _);
            let wheel_delta = WHEEL_DELTA as f32;
            io.mouse_wheel += (wheel_delta_wparam as i16 as f32) / wheel_delta;
        },
        WM_MOUSEHWHEEL => {
            // This `hiword` call is equivalent to GET_WHEEL_DELTA_WPARAM
            let wheel_delta_wparam = hiword(wparam as _);
            let wheel_delta = WHEEL_DELTA as f32;
            io.mouse_wheel_h += (wheel_delta_wparam as i16 as f32) / wheel_delta;
        },
        WM_CHAR => io.add_input_character(wparam as u8 as char),
        WM_SIZE => {
            drop(ctx);
            drop(render_engine);
            RenderState::resize();
            return LRESULT(1);
        },
        _ => {},
    };

    let should_block_messages = imgui_render_loop.as_ref().should_block_messages(io);

    imgui_render_loop.as_ref().on_wnd_proc(hwnd, umsg, WPARAM(wparam), LPARAM(lparam));

    drop(ctx);
    drop(render_engine);

    if should_block_messages {
        return LRESULT(1);
    }

    unsafe { CallWindowProcW(Some(wnd_proc), hwnd, umsg, WPARAM(wparam), LPARAM(lparam)) }
}
