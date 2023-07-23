use std::mem;
use std::ptr::null;

use imgui::{Context, Io, Key, Ui};
use tracing::debug;
use windows::core::PCWSTR;
use windows::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, UnregisterClassW, CS_HREDRAW,
    CS_VREDRAW, HCURSOR, HICON, HWND_MESSAGE, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
};

pub(crate) use self::wnd_proc::*;
use crate::hooks::Hooks;

mod wnd_proc;

/// Holds information useful to the render loop which can't be retrieved from
/// `imgui::Ui`.
pub struct ImguiRenderLoopFlags {
    /// Whether the hooked program's window is currently focused.
    pub focused: bool,
}

/// Implement your `imgui` rendering logic via this trait.
pub trait ImguiRenderLoop {
    /// Called once at the first occurrence of the hook. Implement this to
    /// initialize your data.
    fn initialize(&mut self, _ctx: &mut Context) {}

    /// Called every frame. Use the provided `ui` object to build your UI.
    fn render(&mut self, ui: &mut Ui, flags: &ImguiRenderLoopFlags);

    /// Called during the window procedure.
    fn on_wnd_proc(&self, _hwnd: HWND, _umsg: u32, _wparam: WPARAM, _lparam: LPARAM) {}

    /// If this function returns true, the WndProc function will not call the
    /// procedure of the parent window.
    fn should_block_messages(&self, _io: &Io) -> bool {
        false
    }

    fn into_hook<T>(self) -> Box<T>
    where
        T: Hooks,
        Self: Send + Sync + Sized + 'static,
    {
        T::from_render_loop(self)
    }
}

pub(crate) type WndProcType =
    unsafe extern "system" fn(hwnd: HWND, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;

pub(crate) trait ImguiWindowsEventHandler {
    fn io(&self) -> &imgui::Io;
    fn io_mut(&mut self) -> &mut imgui::Io;

    fn focus(&self) -> bool;
    fn focus_mut(&mut self) -> &mut bool;

    fn wnd_proc(&self) -> WndProcType;

    fn setup_io(&mut self) {
        let io = ImguiWindowsEventHandler::io_mut(self);

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

/// A RAII dummy window.
///
/// Registers a class and creates a window on instantiation.
/// Destroys the window and unregisters the class on drop.
pub struct DummyHwnd(HWND, WNDCLASSEXW);

impl DummyHwnd {
    pub fn new() -> Self {
        // The window procedure for the class just calls `DefWindowProcW`.
        unsafe extern "system" fn wnd_proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        // Create and register the class.
        let wndclass = WNDCLASSEXW {
            cbSize: mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: unsafe { GetModuleHandleW(None).unwrap() },
            hIcon: HICON(0),
            hCursor: HCURSOR(0),
            hbrBackground: HBRUSH(0),
            lpszMenuName: PCWSTR(null()),
            lpszClassName: w!("HUDHOOK").into(),
            hIconSm: HICON(0),
        };
        debug!("{:?}", wndclass);
        unsafe { RegisterClassExW(&wndclass) };

        // Create the window.
        let hwnd = unsafe {
            CreateWindowExW(
                Default::default(),
                wndclass.lpszClassName,
                w!("HUDHOOK"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                100,
                100,
                HWND_MESSAGE,
                None,
                wndclass.hInstance,
                null(),
            )
        };
        debug!("{:?}", hwnd);

        Self(hwnd, wndclass)
    }

    // Retrieve the window handle.
    pub fn hwnd(&self) -> HWND {
        self.0
    }
}

impl Drop for DummyHwnd {
    fn drop(&mut self) {
        // Destroy the window and unregister the class.
        unsafe {
            DestroyWindow(self.0);
            UnregisterClassW(self.1.lpszClassName, self.1.hInstance);
        }
    }
}
