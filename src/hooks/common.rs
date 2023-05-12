use std::hint;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicI16, AtomicU8, Ordering};

use imgui::{Context, Io, Key, Ui};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use tracing::{debug, info, trace};
use windows::Win32::Foundation::{
    CloseHandle, BOOL, HANDLE, HINSTANCE, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT, POINT, RECT,
    WPARAM,
};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenThread, THREAD_QUERY_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RID_DEVICE_INFO_TYPE, RID_INPUT, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
};
use windows::Win32::UI::WindowsAndMessaging::{WHEEL_DELTA, WM_XBUTTONDBLCLK, XBUTTON1, *};

use super::dx11::ImguiDx11Hooks;
use super::dx12::ImguiDx12Hooks;
use super::dx9::ImguiDx9Hooks;
use super::opengl3::ImguiOpenGl3Hooks;
use super::{get_wheel_delta_wparam, hiword, Hooks};
use crate::mh::{MhHook, MhHooks};

pub static mut HHOOKS: OnceCell<Mutex<Vec<HHOOK>>> = OnceCell::new();

pub static mut INPUT: OnceCell<Mutex<Input>> = OnceCell::new();

#[derive(Debug)]
pub struct Input {
    block_mouse: bool,
    block_keyboard: bool,
    keys: [u8; 256],
    input_character: AtomicU8,
    mouse_wheel_delta: AtomicI16,
    mouse_wheel_delta_h: AtomicI16,
    mouse_position: POINT,
    last_mouse_position: POINT,
}

impl Input {
    pub fn new() -> Self {
        Input {
            block_mouse: false,
            block_keyboard: false,
            keys: [0x08; 256],
            input_character: AtomicU8::new(0),
            mouse_wheel_delta: AtomicI16::new(0),
            mouse_wheel_delta_h: AtomicI16::new(0),
            mouse_position: POINT::default(),
            last_mouse_position: POINT::default(),
        }
    }

    pub fn is_blocking_mouse_input(&self) -> bool {
        self.block_mouse
    }

    pub fn block_mouse_input(&mut self, enabled: bool) {
        self.block_mouse = enabled
    }

    pub fn is_blocking_keyboard_input(&self) -> bool {
        self.block_keyboard
    }

    pub fn block_keyboard_input(&mut self, enabled: bool) {
        self.block_keyboard = enabled
    }

    pub fn is_key_down(&self, keycode: usize) -> bool {
        (self.keys[keycode] & 0x80) == 0x80
    }

    pub fn is_mouse_button_down(&self, button: usize) -> bool {
        if button < 2 {
            return self.is_key_down(VK_LBUTTON.0 as usize + button);
        } else {
            self.is_key_down(VK_LBUTTON.0 as usize + button + 1)
        }
    }

    pub fn get_input_character(&self) -> u8 {
        self.input_character.swap(0, Ordering::SeqCst)
    }

    pub fn set_input_character(&mut self, new_input_character: u8) {
        self.input_character.store(new_input_character, Ordering::SeqCst)
    }

    pub fn get_mouse_wheel_delta(&self) -> f32 {
        self.mouse_wheel_delta.swap(0, Ordering::SeqCst) as f32
    }

    pub fn set_mouse_wheel_delta(&mut self, new_mouse_wheel_delta: i16) {
        self.mouse_wheel_delta.store(new_mouse_wheel_delta, Ordering::SeqCst)
    }

    pub fn get_mouse_wheel_delta_h(&self) -> f32 {
        self.mouse_wheel_delta_h.swap(0, Ordering::SeqCst) as f32
    }

    pub fn set_mouse_wheel_delta_h(&mut self, new_mouse_wheel_delta: i16) {
        self.mouse_wheel_delta.store(new_mouse_wheel_delta, Ordering::SeqCst)
    }

    pub fn get_mouse_position(&self) -> POINT {
        self.mouse_position
    }

    pub fn set_mouse_position(&mut self, new_position: POINT) {
        self.mouse_position = new_position;
    }

    pub fn get_last_mouse_position(&self) -> POINT {
        self.last_mouse_position
    }

    pub fn set_last_mouse_position(&mut self, new_position: POINT) {
        self.last_mouse_position = new_position;
    }
}

pub(crate) trait ImguiWindowsEventHandler {
    fn io(&self) -> &imgui::Io;
    fn io_mut(&mut self) -> &mut imgui::Io;

    fn focus(&self) -> bool;
    fn focus_mut(&mut self) -> &mut bool;

    fn setup_io(&mut self) {
        let mut io = ImguiWindowsEventHandler::io_mut(self);

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

    unsafe fn update_io(
        &mut self,
        render_loop: &mut Box<dyn ImguiRenderLoop + Send + Sync>,
        game_hwnd: HWND,
        window_rect: RECT,
    ) {
        let mut io = ImguiWindowsEventHandler::io_mut(self);

        let mut input = INPUT.get_mut().unwrap().lock();

        // Update misc states //

        io.display_size = [
            (window_rect.right - window_rect.left) as f32,
            (window_rect.bottom - window_rect.top) as f32,
        ];

        if render_loop.should_block_messages(&io) {
            io.mouse_draw_cursor = true;
            input.block_mouse = true;
            input.block_keyboard = true;
        } else {
            io.mouse_draw_cursor = false;
            input.block_mouse = false;
            input.block_keyboard = false;
        }

        // Update keyboard states //

        for i in 0..256 {
            io.keys_down[i] = input.is_key_down(i);
        }

        for i in 0..5 {
            io.mouse_down[i] = input.is_mouse_button_down(i);
        }

        let character = input.get_input_character();

        if character != 0 {
            io.add_input_character(character as char);
        }

        // Update mouse states //

        let mut pos = input.get_mouse_position();

        let active_window = GetForegroundWindow();
        if !HANDLE(active_window.0).is_invalid()
            && (active_window == game_hwnd || IsChild(active_window, game_hwnd).as_bool())
        {
            ScreenToClient(active_window, &mut pos);

            io.mouse_pos[0] = pos.x as f32;
            io.mouse_pos[1] = pos.y as f32;
        }

        io.mouse_wheel += input.get_mouse_wheel_delta();
        io.mouse_wheel_h += input.get_mouse_wheel_delta_h();
    }
}

#[must_use]
pub(crate) unsafe fn handle_window_message(lpmsg: &mut MSG) -> bool {
    let msg = lpmsg.message;

    let mut is_mouse_message = msg >= WM_MOUSEFIRST && msg <= WM_MOUSELAST;
    let mut is_keyboard_message = msg >= WM_KEYFIRST && msg <= WM_KEYLAST;

    if msg != WM_INPUT && !is_mouse_message && !is_keyboard_message {
        return false;
    }

    let mut input = INPUT.get_mut().unwrap().lock();

    let wparam = lpmsg.wParam;
    let lparam = lpmsg.lParam;

    input.set_mouse_position(POINT { x: lpmsg.pt.x, y: lpmsg.pt.y });

    match msg {
        WM_INPUT => 'wm_input: {
            let mut raw_data = RAWINPUT { ..Default::default() };
            let mut raw_data_size = size_of::<RAWINPUT>() as u32;
            let raw_data_header_size = size_of::<RAWINPUTHEADER>() as u32;

            // Ignore messages when window is not focused
            if wparam.0 as u32 & 0xFF != RIM_INPUT
                || GetRawInputData(
                    HRAWINPUT(lparam.0),
                    RID_INPUT,
                    &mut raw_data as *mut _ as _,
                    &mut raw_data_size,
                    raw_data_header_size,
                ) == std::u32::MAX
            {
                break 'wm_input;
            }

            match RID_DEVICE_INFO_TYPE(raw_data.header.dwType) {
                RIM_TYPEMOUSE => {
                    is_mouse_message = true;

                    let button_flags = raw_data.data.mouse.Anonymous.Anonymous.usButtonFlags as u32;

                    if button_flags & RI_MOUSE_LEFT_BUTTON_DOWN != 0 {
                        input.keys[VK_LBUTTON.0 as usize] = 0x88;
                    } else if button_flags & RI_MOUSE_LEFT_BUTTON_UP != 0 {
                        input.keys[VK_LBUTTON.0 as usize] = 0x08;
                    }
                    if button_flags & RI_MOUSE_RIGHT_BUTTON_DOWN != 0 {
                        input.keys[VK_RBUTTON.0 as usize] = 0x88;
                    } else if button_flags & RI_MOUSE_RIGHT_BUTTON_UP != 0 {
                        input.keys[VK_RBUTTON.0 as usize] = 0x08;
                    }
                    if button_flags & RI_MOUSE_MIDDLE_BUTTON_DOWN != 0 {
                        input.keys[VK_MBUTTON.0 as usize] = 0x88;
                    } else if button_flags & RI_MOUSE_MIDDLE_BUTTON_UP != 0 {
                        input.keys[VK_MBUTTON.0 as usize] = 0x08;
                    }

                    if button_flags & RI_MOUSE_BUTTON_4_DOWN != 0 {
                        input.keys[VK_XBUTTON1.0 as usize] = 0x88;
                    } else if button_flags & RI_MOUSE_BUTTON_4_UP != 0 {
                        input.keys[VK_XBUTTON1.0 as usize] = 0x08;
                    }

                    if button_flags & RI_MOUSE_BUTTON_5_DOWN != 0 {
                        input.keys[VK_XBUTTON2.0 as usize] = 0x88;
                    } else if button_flags & RI_MOUSE_BUTTON_5_UP != 0 {
                        input.keys[VK_XBUTTON2.0 as usize] = 0x08;
                    }

                    if button_flags & RI_MOUSE_WHEEL != 0 {
                        let wheel_delta = raw_data.data.mouse.Anonymous.Anonymous.usButtonData
                            as i16
                            / WHEEL_DELTA as i16;
                        input.set_mouse_wheel_delta(wheel_delta);
                    }

                    if button_flags & RI_MOUSE_HWHEEL != 0 {
                        let wheel_delta = raw_data.data.mouse.Anonymous.Anonymous.usButtonData
                            as i16
                            / WHEEL_DELTA as i16;
                        input.set_mouse_wheel_delta_h(wheel_delta);
                    }
                },
                RIM_TYPEKEYBOARD => 'rim_keyboard: {
                    // Ignore messages without a valid key code
                    if raw_data.data.keyboard.VKey == 0 {
                        break 'rim_keyboard;
                    }

                    is_keyboard_message = true;

                    let virtual_key = raw_data.data.keyboard.VKey;
                    let mut scan_code = raw_data.data.keyboard.MakeCode as u32;
                    let flags = raw_data.data.keyboard.Flags as u32;

                    // Necessary to check LEFT/RIGHT keys on CTRL & ALT & others (not shift)
                    scan_code |= if flags & RI_KEY_E0 != 0 { 0xe000 } else { 0 };
                    scan_code |= if flags & RI_KEY_E1 != 0 { 0xe100 } else { 0 };

                    let virtual_key = match VIRTUAL_KEY(virtual_key) {
                        VK_SHIFT | VK_CONTROL | VK_MENU => unsafe {
                            match MapVirtualKeyA(scan_code, MAPVK_VSC_TO_VK_EX) {
                                0 => virtual_key,
                                i => VIRTUAL_KEY(i as _).0,
                            }
                        },
                        _ => virtual_key,
                    };

                    // Stops key up from getting blocked if we didn't block key down previously
                    if input.is_blocking_keyboard_input()
                        && (flags & RI_KEY_BREAK) != 0
                        && virtual_key < 0xFF
                        && (input.keys[virtual_key as usize] & 0x04) == 0
                    {
                        is_keyboard_message = false;
                    }

                    // Filter out prefix messages without a key code
                    if raw_data.data.keyboard.VKey < 0xFF {
                        input.keys[virtual_key as usize] =
                            if (flags & RI_KEY_BREAK) == 0 { 0x88 } else { 0x08 };
                    }

                    let mut ch: [u16; 1] = [0];

                    // Only necessary if legacy keyboard messages are disabled I believe - will need
                    // this later when we hook into rawinputdevices properly
                    if (flags & RI_KEY_BREAK) == 0
                        && ToUnicode(virtual_key as u32, scan_code, &input.keys, &mut ch, 0x2) != 0
                    {
                        input.set_input_character(ch[0] as u8);
                    }
                },
                _ => {},
            }
        },
        state @ (WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP) if wparam.0 < 256 => {
            fn map_vkey(wparam: u16, lparam: usize) -> VIRTUAL_KEY {
                match VIRTUAL_KEY(wparam) {
                    VK_SHIFT => unsafe {
                        match MapVirtualKeyA(
                            ((lparam & 0x00ff0000) >> 16) as u32,
                            MAPVK_VSC_TO_VK_EX,
                        ) {
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

            let key_down = (state == WM_KEYDOWN) || (state == WM_SYSKEYDOWN);
            let keycode = map_vkey(wparam.0 as _, lparam.0 as _);

            if key_down {
                input.keys[keycode.0 as usize] = 0x88;
            } else {
                // Stops key up from getting blocked if we didn't block key down previously
                if input.is_blocking_keyboard_input()
                    && (input.keys[keycode.0 as usize] & 0x04) == 0
                {
                    is_keyboard_message = false;
                }
                input.keys[keycode.0 as usize] = 0x08;
            }
        },
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            input.keys[VK_LBUTTON.0 as usize] = 0x88;
        },
        WM_LBUTTONUP => {
            input.keys[VK_LBUTTON.0 as usize] = 0x08;
        },
        WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => {
            input.keys[VK_RBUTTON.0 as usize] = 0x88;
        },
        WM_RBUTTONUP => {
            input.keys[VK_RBUTTON.0 as usize] = 0x08;
        },
        WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => {
            input.keys[VK_MBUTTON.0 as usize] = 0x88;
        },
        WM_MBUTTONUP => {
            input.keys[VK_MBUTTON.0 as usize] = 0x08;
        },
        WM_XBUTTONDOWN | WM_XBUTTONDBLCLK => {
            input.keys[(VK_XBUTTON1.0 + (hiword(wparam.0 as _) - XBUTTON1.0 as u16)) as usize] =
                0x88;
        },
        WM_XBUTTONUP => {
            input.keys[(VK_XBUTTON1.0 + (hiword(wparam.0 as _) - XBUTTON1.0 as u16)) as usize] =
                0x08;
        },
        WM_MOUSEWHEEL => {
            let wheel_delta = get_wheel_delta_wparam(wparam.0 as _) as i16 / WHEEL_DELTA as i16;
            input.set_mouse_wheel_delta(wheel_delta);
        },
        WM_MOUSEHWHEEL => {
            let wheel_delta = get_wheel_delta_wparam(wparam.0 as _) as i16 / WHEEL_DELTA as i16;
            input.set_mouse_wheel_delta_h(wheel_delta);
        },
        WM_CHAR => input.set_input_character(wparam.0 as u8),
        _ => {},
    }

    return (input.is_blocking_mouse_input() && is_mouse_message)
        || (input.is_blocking_keyboard_input() && is_keyboard_message);
}

/// Holds information useful to the render loop which can't be retrieved from
/// `imgui::Ui`.
pub struct ImguiRenderLoopFlags {
    /// Whether the hooked program's window is currently focused.
    pub focused: bool,
}

pub trait HookableBackend: Hooks {
    fn from_struct<T: ImguiRenderLoop + Send + Sync + Sized + 'static>(t: T) -> Self;
}

impl HookableBackend for ImguiDx9Hooks {
    fn from_struct<T: ImguiRenderLoop + Send + Sync + Sized + 'static>(t: T) -> Self {
        unsafe { ImguiDx9Hooks::new(t) }
    }
}

impl HookableBackend for ImguiDx11Hooks {
    fn from_struct<T: ImguiRenderLoop + Send + Sync + Sized + 'static>(t: T) -> Self {
        unsafe { ImguiDx11Hooks::new(t) }
    }
}

impl HookableBackend for ImguiDx12Hooks {
    fn from_struct<T: ImguiRenderLoop + Send + Sync + Sized + 'static>(t: T) -> Self {
        unsafe { ImguiDx12Hooks::new(t) }
    }
}

impl HookableBackend for ImguiOpenGl3Hooks {
    fn from_struct<T: ImguiRenderLoop + Send + Sync + Sized + 'static>(t: T) -> Self {
        unsafe { ImguiOpenGl3Hooks::new(t) }
    }
}

/// Implement your `imgui` rendering logic via this trait.
pub trait ImguiRenderLoop {
    /// Called once at the first occurrence of the hook. Implement this to
    /// initialize your data.
    fn initialize(&mut self, _ctx: &mut Context) {}
    /// Called every frame. Use the provided `ui` object to build your UI.
    fn render(&mut self, ui: &mut Ui, flags: &ImguiRenderLoopFlags);

    /// If this function returns true, the WndProc function will not call the
    /// procedure of the parent window.
    fn should_block_messages(&self, _io: &Io) -> bool {
        false
    }

    fn into_hook<T>(self) -> Box<T>
    where
        T: HookableBackend,
        Self: Send + Sync + Sized + 'static,
    {
        Box::<T>::new(HookableBackend::from_struct(self))
    }
}

/// Spin-loop based synchronization struct.
///
/// Call [`Fence::lock`] in a thread to indicate some operation is in progress,
/// and [`Fence::wait`] on a different thread to create a spin-loop that waits
/// for the lock to be dropped.
pub(crate) struct Fence(AtomicBool);

impl Fence {
    pub(crate) const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    /// Create a [`FenceGuard`].
    pub(crate) fn lock(&self) -> FenceGuard<'_> {
        FenceGuard::new(self)
    }

    /// Wait in a spin-loop for the [`FenceGuard`] created by [`Fence::lock`] to
    /// be dropped.
    pub(crate) fn wait(&self) {
        while self.0.load(Ordering::SeqCst) {
            hint::spin_loop();
        }
    }
}

/// A RAII implementation of a spin-loop for a [`Fence`]. When this is dropped,
/// the wait on a [`Fence`] will terminate.
pub(crate) struct FenceGuard<'a>(&'a Fence);

impl<'a> FenceGuard<'a> {
    fn new(fence: &'a Fence) -> Self {
        fence.0.store(true, Ordering::SeqCst);
        Self(fence)
    }
}

impl<'a> Drop for FenceGuard<'a> {
    fn drop(&mut self) {
        self.0 .0.store(false, Ordering::SeqCst);
    }
}

type SetCursorPosFn = unsafe extern "system" fn(x: i32, y: i32) -> BOOL;
type GetCursorPosFn = unsafe extern "system" fn(lppoint: *mut POINT) -> BOOL;
type ClipCursorFn = unsafe extern "system" fn(rect: *const RECT) -> BOOL;

type PostMessageFn =
    unsafe extern "system" fn(hwnd: HWND, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> BOOL;
type PeekMessageFn = unsafe extern "system" fn(
    lpmsg: *mut MSG,
    hwnd: HWND,
    wmsgfiltermin: u32,
    wmsgfiltermax: u32,
    wremovemsg: PEEK_MESSAGE_REMOVE_TYPE,
) -> BOOL;
type GetMessageFn = unsafe extern "system" fn(
    lpmsg: *mut MSG,
    hwnd: HWND,
    wmsgfiltermin: u32,
    wmsgfiltermax: u32,
) -> BOOL;

type RegisterRawInputDevicesFn =
    unsafe extern "system" fn(prawinputdevices: &[RAWINPUTDEVICE], cbsize: u32) -> BOOL;

static SET_CURSOR_POS_TRAMPOLINE: OnceCell<SetCursorPosFn> = OnceCell::new();
static GET_CURSOR_POS_TRAMPOLINE: OnceCell<GetCursorPosFn> = OnceCell::new();
static CLIP_CURSOR_TRAMPOLINE: OnceCell<ClipCursorFn> = OnceCell::new();

static POST_MESSAGE_A_TRAMPOLINE: OnceCell<PostMessageFn> = OnceCell::new();
static POST_MESSAGE_W_TRAMPOLINE: OnceCell<PostMessageFn> = OnceCell::new();

static PEEK_MESSAGE_A_TRAMPOLINE: OnceCell<PeekMessageFn> = OnceCell::new();
static PEEK_MESSAGE_W_TRAMPOLINE: OnceCell<PeekMessageFn> = OnceCell::new();

static GET_MESSAGE_A_TRAMPOLINE: OnceCell<GetMessageFn> = OnceCell::new();
static GET_MESSAGE_W_TRAMPOLINE: OnceCell<GetMessageFn> = OnceCell::new();

static REGISTER_RAW_INPUT_DEVICES_TRAMPOLINE: OnceCell<RegisterRawInputDevicesFn> = OnceCell::new();

unsafe extern "system" fn set_cursor_pos_impl(x: i32, y: i32) -> BOOL {
    trace!("SetCursorPos invoked");

    let mut input = INPUT.get_mut().unwrap().lock();

    input.set_last_mouse_position(POINT { x, y });

    if input.is_blocking_mouse_input() {
        return BOOL::from(true);
    }

    let trampoline = SET_CURSOR_POS_TRAMPOLINE.get().expect("SetCursorPos unitialized");
    trampoline(x, y)
}

unsafe extern "system" fn get_cursor_pos_impl(lppoint: *mut POINT) -> BOOL {
    trace!("GetCursorPos invoked");

    let input = INPUT.get().unwrap().lock();

    if input.is_blocking_mouse_input() {
        *lppoint = input.get_last_mouse_position();

        return BOOL::from(true);
    }

    let trampoline = GET_CURSOR_POS_TRAMPOLINE.get().expect("GetCursorPos unitialized");
    trampoline(lppoint)
}

unsafe extern "system" fn clip_cursor_impl(mut rect: *const RECT) -> BOOL {
    trace!("ClipCursor invoked");

    let input = INPUT.get().unwrap().lock();

    if input.is_blocking_mouse_input() {
        rect = std::ptr::null();
    }

    let trampoline = CLIP_CURSOR_TRAMPOLINE.get().expect("ClipCursor unitialized");
    trampoline(rect)
}

unsafe extern "system" fn post_message_a_impl(
    hwnd: HWND,
    umsg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> BOOL {
    trace!("PostMessageA invoked");

    let input = INPUT.get().unwrap().lock();

    if input.is_blocking_mouse_input() && umsg == WM_MOUSEMOVE {
        return BOOL::from(true);
    }

    let trampoline = POST_MESSAGE_A_TRAMPOLINE.get().expect("PostMessageA unitialized");
    trampoline(hwnd, umsg, wparam, lparam)
}

unsafe extern "system" fn post_message_w_impl(
    hwnd: HWND,
    umsg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> BOOL {
    trace!("PostMessageW invoked");

    let input = INPUT.get().unwrap().lock();

    if input.is_blocking_mouse_input() && umsg == WM_MOUSEMOVE {
        return BOOL::from(true);
    }

    let trampoline = POST_MESSAGE_W_TRAMPOLINE.get().expect("PostMessageW unitialized");
    trampoline(hwnd, umsg, wparam, lparam)
}

unsafe extern "system" fn peek_message_a_impl(
    lpmsg: *mut MSG,
    hwnd: HWND,
    wmsgfiltermin: u32,
    wmsgfiltermax: u32,
    wremovemsg: PEEK_MESSAGE_REMOVE_TYPE,
) -> BOOL {
    trace!("PeekMessageA invoked");

    let trampoline = PEEK_MESSAGE_A_TRAMPOLINE.get().expect("PeekMessageA unitialized");
    if !trampoline(lpmsg, hwnd, wmsgfiltermin, wmsgfiltermax, wremovemsg).as_bool() {
        return BOOL::from(false);
    }

    // if !IsWindow((*lpmsg).hwnd).as_bool()
    //     && wremovemsg & PM_REMOVE != PEEK_MESSAGE_REMOVE_TYPE(0)
    //     && handle_window_message(lpmsg)
    // {
    //     TranslateMessage(lpmsg);

    //     (*lpmsg).message = WM_NULL;
    // }

    BOOL::from(true)
}

unsafe extern "system" fn peek_message_w_impl(
    lpmsg: *mut MSG,
    hwnd: HWND,
    wmsgfiltermin: u32,
    wmsgfiltermax: u32,
    wremovemsg: PEEK_MESSAGE_REMOVE_TYPE,
) -> BOOL {
    trace!("PeekMessageW invoked");

    let trampoline = PEEK_MESSAGE_W_TRAMPOLINE.get().expect("PeekMessageW unitialized");
    if !trampoline(lpmsg, hwnd, wmsgfiltermin, wmsgfiltermax, wremovemsg).as_bool() {
        return BOOL::from(false);
    }

    // if !IsWindow((*lpmsg).hwnd).as_bool()
    //     && wremovemsg & PM_REMOVE != PEEK_MESSAGE_REMOVE_TYPE(0)
    //     && handle_window_message(lpmsg)
    // {
    //     TranslateMessage(lpmsg);

    //     (*lpmsg).message = WM_NULL;
    // }

    BOOL::from(true)
}

unsafe extern "system" fn get_message_a_impl(
    lpmsg: *mut MSG,
    hwnd: HWND,
    wmsgfiltermin: u32,
    wmsgfiltermax: u32,
) -> BOOL {
    trace!("GetMessageA invoked");

    while !PeekMessageA(lpmsg, hwnd, wmsgfiltermin, wmsgfiltermax, PM_REMOVE).as_bool() {
        MsgWaitForMultipleObjects(&[HANDLE(0)], BOOL::from(false), 500, QS_ALLINPUT);
    }

    if (*lpmsg).message != WM_QUIT {
        std::ptr::write_bytes(lpmsg, 0, size_of::<MSG>());
    }

    return BOOL::from((*lpmsg).message != WM_QUIT);
}

unsafe extern "system" fn get_message_w_impl(
    lpmsg: *mut MSG,
    hwnd: HWND,
    wmsgfiltermin: u32,
    wmsgfiltermax: u32,
) -> BOOL {
    trace!("GetMessageW invoked");

    while !PeekMessageW(lpmsg, hwnd, wmsgfiltermin, wmsgfiltermax, PM_REMOVE).as_bool() {
        MsgWaitForMultipleObjects(&[HANDLE(0)], BOOL::from(false), 500, QS_ALLINPUT);
    }

    if (*lpmsg).message != WM_QUIT {
        std::ptr::write_bytes(lpmsg, 0, size_of::<MSG>());
    }

    return BOOL::from((*lpmsg).message != WM_QUIT);
}

unsafe extern "system" fn register_raw_input_devices_impl(
    prawinputdevices: &[RAWINPUTDEVICE],
    cbsize: u32,
) -> BOOL {
    trace!("RegisterRawInputDevices invoked");

    let trampoline =
        REGISTER_RAW_INPUT_DEVICES_TRAMPOLINE.get().expect("RegisterRawInputDevices unitialized");
    if !trampoline(prawinputdevices, cbsize).as_bool() {
        return BOOL::from(false);
    }

    return BOOL::from(true);
}

pub struct CommonHooks(MhHooks);

impl CommonHooks {
    pub unsafe fn new() -> Self {
        let set_cursor_pos_address: SetCursorPosFn = std::mem::transmute(SetCursorPos as usize);
        let get_cursor_pos_address: GetCursorPosFn = std::mem::transmute(GetCursorPos as usize);
        let clip_cursor_address: ClipCursorFn = std::mem::transmute(ClipCursor as usize);

        let post_message_a_address: PostMessageFn =
            std::mem::transmute(PostMessageA::<HWND, WPARAM, LPARAM> as usize);
        let post_message_w_address: PostMessageFn =
            std::mem::transmute(PostMessageW::<HWND, WPARAM, LPARAM> as usize);

        let peek_message_a_address: PeekMessageFn =
            std::mem::transmute(PeekMessageA::<HWND> as usize);
        let peek_message_w_address: PeekMessageFn =
            std::mem::transmute(PeekMessageW::<HWND> as usize);

        let get_message_a_address: GetMessageFn = std::mem::transmute(GetMessageA::<HWND> as usize);
        let get_message_w_address: GetMessageFn = std::mem::transmute(GetMessageW::<HWND> as usize);

        let register_raw_input_devices_address: RegisterRawInputDevicesFn =
            std::mem::transmute(RegisterRawInputDevices as usize);

        let set_cursor_pos =
            MhHook::new(set_cursor_pos_address as *mut _, set_cursor_pos_impl as *mut _).expect(
                "couldn't
        create SetCursorPos hook",
            );
        let get_cursor_pos =
            MhHook::new(get_cursor_pos_address as *mut _, get_cursor_pos_impl as *mut _).expect(
                "couldn't
        create GetCursorPos hook",
            );
        let clip_cursor = MhHook::new(clip_cursor_address as *mut _, clip_cursor_impl as *mut _)
            .expect(
                "couldn't create ClipCursor
        hook",
            );

        let post_message_a =
            MhHook::new(post_message_a_address as *mut _, post_message_a_impl as *mut _).expect(
                "couldn't
create PostMessageA hook",
            );
        let post_message_w =
            MhHook::new(post_message_w_address as *mut _, post_message_w_impl as *mut _).expect(
                "couldn't
create PostMessageW hook",
            );

        let peek_message_a =
            MhHook::new(peek_message_a_address as *mut _, peek_message_a_impl as *mut _).expect(
                "couldn't
create PeekMessageA hook",
            );
        let peek_message_w =
            MhHook::new(peek_message_w_address as *mut _, peek_message_w_impl as *mut _).expect(
                "couldn't
create PeekMessageW hook",
            );
        let get_message_a =
            MhHook::new(get_message_a_address as *mut _, get_message_a_impl as *mut _).expect(
                "couldn't
    create GetMessageA hook",
            );
        let get_message_w =
            MhHook::new(get_message_w_address as *mut _, get_message_w_impl as *mut _).expect(
                "couldn't
    create GetMessageW hook",
            );
        let register_raw_input_devices = MhHook::new(
            register_raw_input_devices_address as *mut _,
            register_raw_input_devices_impl as *mut _,
        )
        .expect(
            "couldn't
    create RegisterRawInputDevices hook",
        );

        SET_CURSOR_POS_TRAMPOLINE.get_or_init(|| std::mem::transmute(set_cursor_pos.trampoline()));
        GET_CURSOR_POS_TRAMPOLINE.get_or_init(|| std::mem::transmute(get_cursor_pos.trampoline()));
        CLIP_CURSOR_TRAMPOLINE.get_or_init(|| std::mem::transmute(clip_cursor.trampoline()));

        POST_MESSAGE_A_TRAMPOLINE.get_or_init(|| std::mem::transmute(post_message_a.trampoline()));
        POST_MESSAGE_W_TRAMPOLINE.get_or_init(|| std::mem::transmute(post_message_w.trampoline()));

        PEEK_MESSAGE_A_TRAMPOLINE.get_or_init(|| std::mem::transmute(peek_message_a.trampoline()));
        PEEK_MESSAGE_W_TRAMPOLINE.get_or_init(|| std::mem::transmute(peek_message_w.trampoline()));

        GET_MESSAGE_A_TRAMPOLINE.get_or_init(|| std::mem::transmute(get_message_a.trampoline()));
        GET_MESSAGE_W_TRAMPOLINE.get_or_init(|| std::mem::transmute(get_message_w.trampoline()));

        REGISTER_RAW_INPUT_DEVICES_TRAMPOLINE
            .get_or_init(|| std::mem::transmute(register_raw_input_devices.trampoline()));

        Self(
            MhHooks::new([
                set_cursor_pos,
                get_cursor_pos,
                // clip_cursor, Hooking clip_cursor is crashing us right now because it doesn't
                // like getting passed a NULL ptr. Windows-rs bump may fix this
                // post_message_a,
                // post_message_w,
                // peek_message_a,
                // peek_message_w,
                // get_message_a,
                // get_message_w,
                // register_raw_input_devices,
            ])
            .expect("couldn't create hooks"),
        )
    }
}

impl Hooks for CommonHooks {
    unsafe fn hook(&self) {
        self.0.apply();
    }

    unsafe fn unhook(&mut self) {
        trace!("Disabling hooks...");
        self.0.unapply();
    }
}
