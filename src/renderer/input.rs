//! This module contains functions related to processing input events.

use std::ffi::c_void;
use std::mem::size_of;

use imgui::{Io, Key, MouseButton};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, MOUSE_MOVE_ABSOLUTE, RAWINPUT, RAWINPUTHEADER, RAWKEYBOARD,
    RAWMOUSE, RID_DEVICE_INFO_TYPE, RID_INPUT, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::keys::vk_to_imgui;
use crate::renderer::{Pipeline, RenderEngine};

pub type WndProcType =
    unsafe extern "system" fn(hwnd: HWND, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;

// Replication of the Win32 HIWORD macro.
#[inline]
pub fn hiword(l: u32) -> u16 {
    ((l >> 16) & 0xffff) as u16
}

// Replication of the Win32 LOWORD macro.
#[inline]
pub fn loword(l: u32) -> u16 {
    (l & 0xffff) as u16
}

// Replication of the Win32 HIWORD macro, returning signed values.
#[inline]
pub fn hiwordi(l: u32) -> i16 {
    ((l >> 16) & 0xffff) as i16
}

// Replication of the Win32 LOWORD macro, returning signed values.
#[inline]
pub fn lowordi(l: u32) -> i16 {
    (l & 0xffff) as i16
}

////////////////////////////////////////////////////////////////////////////////
// Raw input
////////////////////////////////////////////////////////////////////////////////

// Handle raw mouse input events.
//
// Given the RAWINPUT structure, check each possible mouse flag status and
// update the Io object accordingly. Both the key_down indices associated to the
// mouse click (VK_...) and the values in mouse_down are updated.
fn handle_raw_mouse_input(io: &mut Io, raw_mouse: &RAWMOUSE) {
    let button_data = unsafe { raw_mouse.Anonymous.Anonymous };
    let button_flags = button_data.usButtonFlags as u32;

    let mut event = |flag, button, state| {
        if (button_flags & flag) != 0 {
            io.add_mouse_button_event(button, state);
        }
    };

    // Check whether any of the mouse buttons was pressed or released.
    event(RI_MOUSE_LEFT_BUTTON_DOWN, MouseButton::Left, true);
    event(RI_MOUSE_LEFT_BUTTON_UP, MouseButton::Left, false);
    event(RI_MOUSE_RIGHT_BUTTON_DOWN, MouseButton::Right, true);
    event(RI_MOUSE_RIGHT_BUTTON_UP, MouseButton::Right, false);
    event(RI_MOUSE_MIDDLE_BUTTON_DOWN, MouseButton::Middle, true);
    event(RI_MOUSE_MIDDLE_BUTTON_UP, MouseButton::Middle, false);
    event(RI_MOUSE_BUTTON_4_DOWN, MouseButton::Extra1, true);
    event(RI_MOUSE_BUTTON_4_UP, MouseButton::Extra1, false);
    event(RI_MOUSE_BUTTON_5_DOWN, MouseButton::Extra2, true);
    event(RI_MOUSE_BUTTON_5_UP, MouseButton::Extra2, false);

    // Apply vertical mouse scroll.
    let wheel_delta_x = if button_flags & RI_MOUSE_WHEEL != 0 {
        let wheel_delta = button_data.usButtonData as i16 / WHEEL_DELTA as i16;
        wheel_delta as f32
    } else {
        0.0
    };

    // Apply horizontal mouse scroll.
    let wheel_delta_y = if button_flags & RI_MOUSE_HWHEEL != 0 {
        let wheel_delta = button_data.usButtonData as i16 / WHEEL_DELTA as i16;
        wheel_delta as f32
    } else {
        0.0
    };

    io.add_mouse_wheel_event([wheel_delta_x, wheel_delta_y]);

    let mouse_flags = raw_mouse.usFlags;
    let (last_x, last_y) = (raw_mouse.lLastX as f32, raw_mouse.lLastY as f32);

    if (mouse_flags.0 & MOUSE_MOVE_ABSOLUTE.0) != 0 {
        io.add_mouse_pos_event([last_x, last_y]);
    } else {
        io.add_mouse_pos_event([io.mouse_pos[0] + last_x, io.mouse_pos[1] + last_y]);
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
            match unsafe { MapVirtualKeyW(scan_code, MAPVK_VSC_TO_VK_EX) } {
                0 => virtual_key.0,
                i => i as u16,
            }
        },
        VIRTUAL_KEY(virtual_key) => virtual_key,
    } as usize;

    // If the virtual key is in the allowed array range, set the appropriate status
    // of key_down for that virtual key.
    if virtual_key < 0xFF {
        if let Some(key) = vk_to_imgui(VIRTUAL_KEY(virtual_key as _)) {
            if is_key_down {
                io.add_key_event(key, true);
            }
            if is_key_up {
                io.add_key_event(key, false);
            }
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
            handle_raw_mouse_input(io, unsafe { &raw_data.data.mouse });
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
            match MapVirtualKeyW(((lparam & 0x00ff0000) >> 16) as u32, MAPVK_VSC_TO_VK_EX) {
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

fn is_vk_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vk.0 as i32) < 0 }
}

// Handle WM_(SYS)KEYDOWN/WM_(SYS)KEYUP events.
fn handle_input(io: &mut Io, state: u32, WPARAM(wparam): WPARAM, LPARAM(lparam): LPARAM) {
    let is_key_down = (state == WM_KEYDOWN) || (state == WM_SYSKEYDOWN);
    let scancode = map_vkey(wparam as _, lparam as _);

    if let Some(key) = vk_to_imgui(scancode) {
        io.add_key_event(key, is_key_down);
    }

    io.add_key_event(Key::ModCtrl, is_vk_down(VK_CONTROL));
    io.add_key_event(Key::ModShift, is_vk_down(VK_SHIFT));
    io.add_key_event(Key::ModAlt, is_vk_down(VK_MENU));
    io.add_key_event(Key::ModSuper, is_vk_down(VK_APPS));

    if scancode == VK_SHIFT {
        if is_vk_down(VK_LSHIFT) == is_key_down {
            io.add_key_event(Key::LeftShift, is_key_down);
        }
        if is_vk_down(VK_RSHIFT) == is_key_down {
            io.add_key_event(Key::RightShift, is_key_down);
        }
    } else if scancode == VK_CONTROL {
        if is_vk_down(VK_LCONTROL) == is_key_down {
            io.add_key_event(Key::LeftCtrl, is_key_down);
        }
        if is_vk_down(VK_RCONTROL) == is_key_down {
            io.add_key_event(Key::RightCtrl, is_key_down);
        }
    } else if scancode == VK_MENU {
        if is_vk_down(VK_LMENU) == is_key_down {
            io.add_key_event(Key::LeftAlt, is_key_down);
        }
        if is_vk_down(VK_RMENU) == is_key_down {
            io.add_key_event(Key::RightAlt, is_key_down);
        }
    }

    // TODO: Workarounds https://github.com/ocornut/imgui/blob/da29b776eed289db16a8527e5f16a0e1fa540251/backends/imgui_impl_win32.cpp#L263
}

////////////////////////////////////////////////////////////////////////////////
// Window procedure
////////////////////////////////////////////////////////////////////////////////

pub fn imgui_wnd_proc_impl<T: RenderEngine>(
    hwnd: HWND,
    umsg: u32,
    WPARAM(wparam): WPARAM,
    LPARAM(lparam): LPARAM,
    pipeline: &mut Pipeline<T>,
) {
    let io = pipeline.context().io_mut();

    match umsg {
        WM_INPUT => handle_raw_input(io, WPARAM(wparam), LPARAM(lparam)),
        state @ (WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP) if wparam < 256 => {
            handle_input(io, state, WPARAM(wparam), LPARAM(lparam))
        },
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            io.add_mouse_button_event(MouseButton::Left, true);
        },
        WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => {
            io.add_mouse_button_event(MouseButton::Right, true);
        },
        WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => {
            io.add_mouse_button_event(MouseButton::Middle, true);
        },
        WM_XBUTTONDOWN | WM_XBUTTONDBLCLK => {
            let btn = if hiword(wparam as _) == XBUTTON1 {
                MouseButton::Extra1
            } else {
                MouseButton::Extra2
            };
            io.add_mouse_button_event(btn, true);
        },
        WM_LBUTTONUP => {
            io.add_mouse_button_event(MouseButton::Left, false);
        },
        WM_RBUTTONUP => {
            io.add_mouse_button_event(MouseButton::Right, false);
        },
        WM_MBUTTONUP => {
            io.add_mouse_button_event(MouseButton::Middle, false);
        },
        WM_XBUTTONUP => {
            let btn = if hiword(wparam as _) == XBUTTON1 {
                MouseButton::Extra1
            } else {
                MouseButton::Extra2
            };
            io.add_mouse_button_event(btn, false);
        },
        WM_MOUSEWHEEL => {
            // This `hiword` call is equivalent to GET_WHEEL_DELTA_WPARAM
            let wheel_delta_wparam = hiword(wparam as _);
            let wheel_delta = WHEEL_DELTA as f32;
            io.add_mouse_wheel_event([0.0, (wheel_delta_wparam as i16 as f32) / wheel_delta]);
        },
        WM_MOUSEHWHEEL => {
            // This `hiword` call is equivalent to GET_WHEEL_DELTA_WPARAM
            let wheel_delta_wparam = hiword(wparam as _);
            let wheel_delta = WHEEL_DELTA as f32;
            io.add_mouse_wheel_event([(wheel_delta_wparam as i16 as f32) / wheel_delta, 0.0]);
        },
        WM_MOUSEMOVE => {
            let x = lowordi(lparam as u32) as f32;
            let y = hiwordi(lparam as u32) as f32;
            io.add_mouse_pos_event([x, y]);
        },
        WM_CHAR => io.add_input_character(char::from_u32(wparam as u32).unwrap()),
        WM_SIZE => {
            pipeline.resize(loword(lparam as u32) as u32, hiword(lparam as u32) as u32);
        },
        _ => {},
    };

    pipeline.render_loop().on_wnd_proc(hwnd, umsg, WPARAM(wparam), LPARAM(lparam));
}
