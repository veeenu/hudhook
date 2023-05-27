//! This module contains functions related to processing input events.

use std::mem::size_of;

use imgui::Io;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyA, VIRTUAL_KEY, VK_CONTROL, VK_LBUTTON, VK_MBUTTON, VK_MENU, VK_RBUTTON, VK_SHIFT,
    VK_XBUTTON1, VK_XBUTTON2,
};
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTHEADER, RAWKEYBOARD, RAWMOUSE_0_0,
    RID_DEVICE_INFO_TYPE, RID_INPUT, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    MAPVK_VSC_TO_VK_EX, RIM_INPUT, RI_KEY_BREAK, RI_KEY_E0, RI_KEY_E1, RI_KEY_MAKE,
    RI_MOUSE_BUTTON_4_DOWN, RI_MOUSE_BUTTON_4_UP, RI_MOUSE_BUTTON_5_DOWN, RI_MOUSE_BUTTON_5_UP,
    RI_MOUSE_HWHEEL, RI_MOUSE_LEFT_BUTTON_DOWN, RI_MOUSE_LEFT_BUTTON_UP,
    RI_MOUSE_MIDDLE_BUTTON_DOWN, RI_MOUSE_MIDDLE_BUTTON_UP, RI_MOUSE_RIGHT_BUTTON_DOWN,
    RI_MOUSE_RIGHT_BUTTON_UP, RI_MOUSE_WHEEL, WHEEL_DELTA,
};

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
pub(crate) fn handle_raw_input(io: &mut Io, WPARAM(wparam): WPARAM, LPARAM(lparam): LPARAM) {
    let mut raw_data = RAWINPUT { ..Default::default() };
    let mut raw_data_size = size_of::<RAWINPUT>() as u32;
    let raw_data_header_size = size_of::<RAWINPUTHEADER>() as u32;

    // Read the raw input data.
    let r = unsafe {
        GetRawInputData(
            HRAWINPUT(lparam),
            RID_INPUT,
            &mut raw_data as *mut _ as _,
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
