use std::hint;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicI16, AtomicU8, Ordering};

use imgui::{Context, Io, Key, Ui};
use log::{debug, info};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use windows::core::{PCSTR, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, BOOL, FILETIME, HANDLE, HINSTANCE, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT,
    POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Dxgi::DXGI_SWAP_CHAIN_DESC;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetModuleHandleW};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetThreadTimes, OpenThread, THREAD_QUERY_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Input::{RegisterRawInputDevices, RAWINPUTDEVICE};
use windows::Win32::UI::WindowsAndMessaging::{WHEEL_DELTA, WM_XBUTTONDBLCLK, XBUTTON1, *};

use super::dx11::ImguiDx11Hooks;
use super::dx12::ImguiDx12Hooks;
use super::dx9::ImguiDx9Hooks;
use super::opengl3::ImguiOpenGl3Hooks;
use super::{get_wheel_delta_wparam, Hooks};
use crate::mh::{MH_ApplyQueued, MH_QueueEnableHook, MhHook};

pub static mut LAST_CURSOR_POS: OnceCell<Mutex<POINT>> = OnceCell::new();
pub static GAME_MOUSE_BLOCKED: AtomicBool = AtomicBool::new(false);

pub static mut KEYS: OnceCell<Mutex<[usize; 256]>> = OnceCell::new();
pub static mut MOUSE_WHEEL_DELTA: AtomicI16 = AtomicI16::new(0);
pub static mut MOUSE_WHEEL_DELTA_H: AtomicI16 = AtomicI16::new(0);
pub static mut INPUT_CHARACTER: AtomicU8 = AtomicU8::new(0);

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
}

#[must_use]
pub(crate) unsafe fn handle_window_message(lpmsg: *mut MSG) -> bool {
    let msg = (*lpmsg).message;

    let is_mouse_message = msg >= WM_MOUSEFIRST && msg <= WM_MOUSELAST;
    let is_keyboard_message = msg >= WM_KEYFIRST && msg <= WM_KEYLAST;

    if msg != WM_INPUT && !is_mouse_message && !is_keyboard_message {
        return false;
    }
    let mut keys = KEYS.get_mut().unwrap().lock();

    let wparam = (*lpmsg).wParam;
    let lparam = (*lpmsg).lParam;

    // println!("Mouse: {:?}", is_mouse_message);
    // println!("Keyboard: {:?}", is_keyboard_message);

    *LAST_CURSOR_POS.get_mut().unwrap().lock() = POINT { x: (*lpmsg).pt.x, y: (*lpmsg).pt.y };

    match msg {
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
                keys[keycode.0 as usize] = 0x88;
            } else {
                keys[keycode.0 as usize] = 0x08;
            }

            // match key_pressed {
            //     VK_CONTROL | VK_LCONTROL | VK_RCONTROL => io.key_ctrl =
            // pressed,     VK_SHIFT | VK_LSHIFT | VK_RSHIFT =>
            // io.key_shift = pressed,     VK_MENU | VK_LMENU |
            // VK_RMENU => io.key_alt = pressed,     VK_LWIN |
            // VK_RWIN => io.key_super = pressed,     _ => (),
            // };
        },
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            keys[VK_LBUTTON.0 as usize] = 0x88;
        },
        WM_LBUTTONUP => {
            keys[VK_LBUTTON.0 as usize] = 0x08;
        },
        WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => {
            keys[VK_RBUTTON.0 as usize] = 0x88;
        },
        WM_RBUTTONUP => {
            keys[VK_RBUTTON.0 as usize] = 0x08;
        },
        WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => {
            keys[VK_MBUTTON.0 as usize] = 0x88;
        },
        WM_MBUTTONUP => {
            keys[VK_MBUTTON.0 as usize] = 0x08;
        },
        WM_MOUSEWHEEL => {
            let wheel_delta = get_wheel_delta_wparam(wparam.0 as _) as i16 / WHEEL_DELTA as i16;
            MOUSE_WHEEL_DELTA.store(wheel_delta, Ordering::SeqCst);
        },
        WM_MOUSEHWHEEL => {
            let wheel_delta = get_wheel_delta_wparam(wparam.0 as _) as i16 / WHEEL_DELTA as i16;
            MOUSE_WHEEL_DELTA_H.store(wheel_delta, Ordering::SeqCst);
        },
        WM_CHAR => INPUT_CHARACTER.store(wparam.0 as u8, Ordering::SeqCst),
        _ => {},
    }

    return GAME_MOUSE_BLOCKED.load(Ordering::SeqCst);
}

pub unsafe fn is_key_down(keycode: usize) -> bool {
    return (KEYS.get().unwrap().lock()[keycode] & 0x80) == 0x80;
}

pub unsafe fn is_key_pressed(keycode: usize) -> bool {
    return (KEYS.get().unwrap().lock()[keycode] & 0x88) == 0x88;
}

pub unsafe fn is_mouse_button_down(button: usize) -> bool {
    if button < 2 {
        return is_key_down(VK_LBUTTON.0 as usize + button);
    } else {
        is_key_down(VK_LBUTTON.0 as usize + button + 1)
    }
}

pub unsafe fn is_mouse_button_pressed(button: usize) -> bool {
    if button < 2 {
        return is_key_pressed(VK_LBUTTON.0 as usize + button);
    } else {
        is_key_pressed(VK_LBUTTON.0 as usize + button + 1)
    }
}

pub unsafe fn update_imgui_io(
    io: &mut Io,
    render_loop: &mut Box<dyn ImguiRenderLoop + Send + Sync>,
) {
    for i in 0..256 {
        io.keys_down[i] = is_key_down(i);
    }

    for i in 0..5 {
        io.mouse_down[i] = is_mouse_button_down(i);
    }

    let char = INPUT_CHARACTER.swap(0, Ordering::SeqCst);

    if char != 0 {
        io.add_input_character(char as char);
    }

    io.mouse_wheel += MOUSE_WHEEL_DELTA.swap(0, Ordering::SeqCst) as f32;
    io.mouse_wheel_h += MOUSE_WHEEL_DELTA_H.swap(0, Ordering::SeqCst) as f32;

    if render_loop.should_block_messages(&io) {
        if !io.mouse_draw_cursor {
            io.mouse_draw_cursor = true;
            GAME_MOUSE_BLOCKED.store(true, Ordering::SeqCst);
        }
    } else {
        if io.mouse_draw_cursor {
            io.mouse_draw_cursor = false;
            GAME_MOUSE_BLOCKED.store(false, Ordering::SeqCst);
        }
    }
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
pub type PeekMessageFn = unsafe extern "system" fn(
    lpmsg: *mut MSG,
    hwnd: HWND,
    wmsgfiltermin: u32,
    wmsgfiltermax: u32,
    wremovemsg: PEEK_MESSAGE_REMOVE_TYPE,
) -> BOOL;
pub type GetMessageFn = unsafe extern "system" fn(
    lpmsg: *mut MSG,
    hwnd: HWND,
    wmsgfiltermin: u32,
    wmsgfiltermax: u32,
) -> BOOL;

type RegisterRawInputDevicesFn =
    unsafe extern "system" fn(prawinputdevices: &[RAWINPUTDEVICE], cbsize: u32) -> BOOL;

pub type RegisterClassFn = unsafe extern "system" fn(lpwndclass: *const WNDCLASSW) -> u16;

static SET_CURSOR_POS_TRAMPOLINE: OnceCell<SetCursorPosFn> = OnceCell::new();
static GET_CURSOR_POS_TRAMPOLINE: OnceCell<GetCursorPosFn> = OnceCell::new();
static CLIP_CURSOR_TRAMPOLINE: OnceCell<ClipCursorFn> = OnceCell::new();

static POST_MESSAGE_A_TRAMPOLINE: OnceCell<PostMessageFn> = OnceCell::new();
static POST_MESSAGE_W_TRAMPOLINE: OnceCell<PostMessageFn> = OnceCell::new();

static PEEK_MESSAGE_A_TRAMPOLINE: OnceCell<PeekMessageFn> = OnceCell::new();
pub static PEEK_MESSAGE_W_TRAMPOLINE: OnceCell<PeekMessageFn> = OnceCell::new();

static GET_MESSAGE_A_TRAMPOLINE: OnceCell<GetMessageFn> = OnceCell::new();
static GET_MESSAGE_W_TRAMPOLINE: OnceCell<GetMessageFn> = OnceCell::new();

static REGISTER_RAW_INPUT_DEVICES_TRAMPOLINE: OnceCell<RegisterRawInputDevicesFn> = OnceCell::new();

static REGISTER_CLASS_A_TRAMPOLINE: OnceCell<RegisterClassFn> = OnceCell::new();
static REGISTER_CLASS_W_TRAMPOLINE: OnceCell<RegisterClassFn> = OnceCell::new();
static REGISTER_CLASS_EX_A_TRAMPOLINE: OnceCell<RegisterClassFn> = OnceCell::new();
static REGISTER_CLASS_EX_W_TRAMPOLINE: OnceCell<RegisterClassFn> = OnceCell::new();

unsafe extern "system" fn set_cursor_pos_impl(x: i32, y: i32) -> BOOL {
    info!("SetCursorPos invoked");

    // LAST_CURSOR_POS.get_mut().unwrap().lock().x = x;
    // LAST_CURSOR_POS.get_mut().unwrap().lock().y = y;

    if GAME_MOUSE_BLOCKED.load(Ordering::SeqCst) {
        return BOOL::from(true);
    }

    let trampoline = SET_CURSOR_POS_TRAMPOLINE.get().expect("SetCursorPos unitialized");
    trampoline(x, y)
}

unsafe extern "system" fn get_cursor_pos_impl(lppoint: *mut POINT) -> BOOL {
    // info!("GetCursorPos invoked");

    if GAME_MOUSE_BLOCKED.load(Ordering::SeqCst) {
        *lppoint = POINT { x: 500, y: 500 };

        return BOOL::from(true);
    }

    let trampoline = GET_CURSOR_POS_TRAMPOLINE.get().expect("GetCursorPos unitialized");
    trampoline(lppoint)
}

unsafe extern "system" fn clip_cursor_impl(mut rect: *const RECT) -> BOOL {
    info!("ClipCursor invoked");

    if GAME_MOUSE_BLOCKED.load(Ordering::SeqCst) {
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
    info!("PostMessageA invoked");

    if GAME_MOUSE_BLOCKED.load(Ordering::Relaxed) && umsg == WM_MOUSEMOVE {
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
    info!("PostMessageW invoked");

    if GAME_MOUSE_BLOCKED.load(Ordering::Relaxed) && umsg == WM_MOUSEMOVE {
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
    info!("PeekMessageA invoked");

    let trampoline = PEEK_MESSAGE_A_TRAMPOLINE.get().expect("PeekMessageA unitialized");
    if !trampoline(lpmsg, hwnd, wmsgfiltermin, wmsgfiltermax, wremovemsg).as_bool() {
        return BOOL::from(false);
    }

    if !IsWindow((*lpmsg).hwnd).as_bool()
        && wremovemsg & PM_REMOVE != PEEK_MESSAGE_REMOVE_TYPE(0)
        && handle_window_message(lpmsg)
    {
        TranslateMessage(lpmsg);

        (*lpmsg).message = WM_NULL;
    }

    BOOL::from(true)
}

pub unsafe extern "system" fn peek_message_w_impl(
    lpmsg: *mut MSG,
    hwnd: HWND,
    wmsgfiltermin: u32,
    wmsgfiltermax: u32,
    wremovemsg: PEEK_MESSAGE_REMOVE_TYPE,
) -> BOOL {
    // info!("PeekMessageW invoked");

    let trampoline = PEEK_MESSAGE_W_TRAMPOLINE.get().expect("PeekMessageW unitialized");
    if !trampoline(lpmsg, hwnd, wmsgfiltermin, wmsgfiltermax, wremovemsg).as_bool() {
        return BOOL::from(false);
    }

    if !IsWindow((*lpmsg).hwnd).as_bool()
        && wremovemsg & PM_REMOVE != PEEK_MESSAGE_REMOVE_TYPE(0)
        && handle_window_message(lpmsg)
    {
        TranslateMessage(lpmsg);

        (*lpmsg).message = WM_NULL;
    }

    BOOL::from(true)
}

unsafe extern "system" fn get_message_a_impl(
    lpmsg: *mut MSG,
    hwnd: HWND,
    wmsgfiltermin: u32,
    wmsgfiltermax: u32,
) -> BOOL {
    info!("GetMessageA invoked");

    while !PeekMessageA(lpmsg, hwnd, wmsgfiltermin, wmsgfiltermax, PM_REMOVE).as_bool() {
        MsgWaitForMultipleObjects(&[HANDLE(0)], BOOL::from(false), 500, QS_ALLINPUT);
    }

    if (*lpmsg).message != WM_QUIT {
        std::ptr::write_bytes(lpmsg, 0, size_of::<MSG>());
    }

    return BOOL::from((*lpmsg).message != WM_QUIT);
}

pub unsafe extern "system" fn get_message_w_impl(
    lpmsg: *mut MSG,
    hwnd: HWND,
    wmsgfiltermin: u32,
    wmsgfiltermax: u32,
) -> BOOL {
    info!("GetMessageW invoked");

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
    info!("RegisterRawInputDevices invoked");

    let trampoline =
        REGISTER_RAW_INPUT_DEVICES_TRAMPOLINE.get().expect("RegisterRawInputDevices unitialized");
    if !trampoline(prawinputdevices, cbsize).as_bool() {
        return BOOL::from(false);
    }

    return BOOL::from(true);
}

unsafe extern "system" fn register_class_a_impl(lpwndclass: *const WNDCLASSW) -> u16 {
    info!("RegisterClassA invoked");

    let mut wndclass = *lpwndclass;

    if wndclass.hInstance == GetModuleHandleA(PCSTR::null()).unwrap() {
        wndclass.style |= CS_OWNDC;
    }

    return (*REGISTER_CLASS_W_TRAMPOLINE.get().expect("RegisterClassA unitialized")) as u16;
}

pub unsafe extern "system" fn register_class_w_impl(lpwndclass: *const WNDCLASSW) -> u16 {
    info!("RegisterClassW invoked");

    let mut wndclass = *lpwndclass;

    if wndclass.hInstance == GetModuleHandleW(PCWSTR::null()).unwrap() {
        wndclass.style |= CS_OWNDC;
    }

    return (*REGISTER_CLASS_W_TRAMPOLINE.get().expect("RegisterClassW unitialized")) as u16;
}

unsafe extern "system" fn register_class_ex_a_impl(lpwndclass: *const WNDCLASSW) -> u16 {
    info!("RegisterClassExA invoked");

    let mut wndclass = *lpwndclass;

    if wndclass.hInstance == GetModuleHandleA(PCSTR::null()).unwrap() {
        wndclass.style |= CS_OWNDC;
    }

    return (*REGISTER_CLASS_W_TRAMPOLINE.get().expect("RegisterClassExA unitialized")) as u16;
}

unsafe extern "system" fn register_class_ex_w_impl(lpwndclass: *const WNDCLASSW) -> u16 {
    info!("RegisterClassExW invoked");

    let mut wndclass = *lpwndclass;

    if wndclass.hInstance == GetModuleHandleW(PCWSTR::null()).unwrap() {
        wndclass.style |= CS_OWNDC;
    }

    return (*REGISTER_CLASS_W_TRAMPOLINE.get().expect("RegisterClassExW unitialized")) as u16;
}

pub unsafe extern "system" fn get_msg_proc(_code: i32, _wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let msg: *mut MSG = std::mem::transmute(lparam);
    if handle_window_message(msg) {
        TranslateMessage(msg);

        SetCursor(HCURSOR(0));

        (*msg).message = WM_NULL;
    }

    LRESULT(1)
}

pub fn setup_window_message_handling() {
    unsafe {
        let pid = GetCurrentProcessId();

        let mut ull_min_create_time = std::u64::MAX;
        let mut dw_main_thread_id = 0;

        let thread_snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0).unwrap();

        if thread_snap != INVALID_HANDLE_VALUE {
            let mut th32: THREADENTRY32 = std::mem::zeroed();
            th32.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            let mut thread_found = Thread32First(thread_snap, &mut th32).as_bool();

            while thread_found {
                if th32.th32OwnerProcessID == pid {
                    if let Ok(handle_thread) =
                        OpenThread(THREAD_QUERY_INFORMATION, true, th32.th32ThreadID)
                    {
                        if handle_thread != INVALID_HANDLE_VALUE {
                            let mut af_times: [FILETIME; 4] = [std::mem::zeroed(); 4];

                            if GetThreadTimes(
                                handle_thread,
                                &mut af_times[0],
                                &mut af_times[1],
                                &mut af_times[2],
                                &mut af_times[3],
                            )
                            .as_bool()
                            {
                                SetWindowsHookExW(
                                    WH_GETMESSAGE,
                                    Some(get_msg_proc),
                                    HINSTANCE::default(),
                                    th32.th32ThreadID,
                                );
                                let ull_test = af_times[0].dwLowDateTime as u64
                                    + ((af_times[0].dwHighDateTime as u64) << 32);
                                if ull_test != 0 && ull_test < ull_min_create_time {
                                    ull_min_create_time = ull_test;
                                    dw_main_thread_id = th32.th32ThreadID;
                                }
                            }
                            CloseHandle(handle_thread);
                        }
                    }
                }
                thread_found = Thread32Next(thread_snap, &mut th32).as_bool();
            }
        }

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

        let register_class_a_address: RegisterClassFn =
            std::mem::transmute(RegisterClassA as usize);
        let register_class_w_address: RegisterClassFn =
            std::mem::transmute(RegisterClassW as usize);
        let register_class_ex_a_address: RegisterClassFn =
            std::mem::transmute(RegisterClassExA as usize);
        let register_class_ex_w_address: RegisterClassFn =
            std::mem::transmute(RegisterClassExW as usize);

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
                "couldn't create GetCursorPos
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

        let register_class_a =
            MhHook::new(register_class_a_address as *mut _, register_class_a_impl as *mut _)
                .expect(
                    "couldn't
    create RegisterClassA hook",
                );

        let register_class_w =
            MhHook::new(register_class_w_address as *mut _, register_class_w_impl as *mut _)
                .expect(
                    "couldn't
    create RegisterClassW hook",
                );

        let register_class_ex_a =
            MhHook::new(register_class_ex_a_address as *mut _, register_class_ex_a_impl as *mut _)
                .expect(
                    "couldn't
    create RegisterClassExA hook",
                );

        let register_class_ex_w =
            MhHook::new(register_class_ex_w_address as *mut _, register_class_ex_w_impl as *mut _)
                .expect(
                    "couldn't
    create RegisterClassExW hook",
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

        REGISTER_CLASS_A_TRAMPOLINE
            .get_or_init(|| std::mem::transmute(register_class_a.trampoline()));
        REGISTER_CLASS_W_TRAMPOLINE
            .get_or_init(|| std::mem::transmute(register_class_w.trampoline()));
        REGISTER_CLASS_EX_A_TRAMPOLINE
            .get_or_init(|| std::mem::transmute(register_class_ex_a.trampoline()));
        REGISTER_CLASS_EX_W_TRAMPOLINE
            .get_or_init(|| std::mem::transmute(register_class_ex_w.trampoline()));

        // let status = MH_QueueEnableHook(set_cursor_pos.addr);
        // debug!("MH_QueueEnable SetCursorPos: {:?}", status);

        // let status = MH_QueueEnableHook(get_cursor_pos.addr);
        // debug!("MH_QueueEnable GetCursorPos: {:?}", status);

        // let status = MH_QueueEnableHook(clip_cursor.addr);
        // debug!("MH_QueueEnable ClipCursor: {:?}", status);

        let status = MH_QueueEnableHook(post_message_a.addr);
        debug!("MH_QueueEnable PostMessageA: {:?}", status);

        let status = MH_QueueEnableHook(post_message_w.addr);
        debug!("MH_QueueEnable PostMessageW: {:?}", status);

        let status = MH_QueueEnableHook(peek_message_a.addr);
        debug!("MH_QueueEnable PeekMessageA: {:?}", status);

        let status = MH_QueueEnableHook(peek_message_w.addr);
        debug!("MH_QueueEnable PeekMessageW: {:?}", status);

        let status = MH_QueueEnableHook(get_message_a.addr);
        debug!("MH_QueueEnable GetMessageA: {:?}", status);

        let status = MH_QueueEnableHook(get_message_w.addr);
        debug!("MH_QueueEnable GetMessageW: {:?}", status);

        let status = MH_QueueEnableHook(register_raw_input_devices.addr);
        debug!("MH_QueueEnable RegisterRawInputDevices: {:?}", status);

        let status = MH_QueueEnableHook(register_class_a.addr);
        debug!("MH_QueueEnable RegisterClassA: {:?}", status);

        let status = MH_QueueEnableHook(register_class_w.addr);
        debug!("MH_QueueEnable RegisterClassW: {:?}", status);

        let status = MH_QueueEnableHook(register_class_ex_a.addr);
        debug!("MH_QueueEnable RegisterClassExA: {:?}", status);

        let status = MH_QueueEnableHook(register_class_ex_w.addr);
        debug!("MH_QueueEnable RegisterClassExW: {:?}", status);

        let status = MH_ApplyQueued();
        debug!("MH_ApplyQueued: {:?}", status);
    }
}
