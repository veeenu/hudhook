use std::mem;
use std::ptr::null;

use imgui::Key;
use tracing::debug;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, UnregisterClassW, CS_HREDRAW,
    CS_VREDRAW, HCURSOR, HICON, HWND_MESSAGE, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
};

pub use crate::hooks::common::wnd_proc::*;
use crate::hooks::ImguiRenderLoop;
use crate::mh::MhHook;

pub mod wnd_proc;

pub type WndProcType =
    unsafe extern "system" fn(hwnd: HWND, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT;

/// Generic trait for platform-specific hooks.
///
/// Implement this if you are building a custom renderer.
///
/// Check out first party implementations ([`crate::hooks::dx9`],
/// [`crate::hooks::dx11`], [`crate::hooks::dx12`], [`crate::hooks::opengl3`])
/// for guidance on how to implement the methods.
pub trait Hooks {
    fn from_render_loop<T>(t: T) -> Box<Self>
    where
        Self: Sized,
        T: ImguiRenderLoop + Send + Sync + 'static;

    /// Return the list of hooks to be enabled, in order.
    fn hooks(&self) -> &[MhHook];

    /// Cleanup global data and disable the hooks.
    ///
    /// # Safety
    ///
    /// Is most definitely UB.
    unsafe fn unhook(&mut self);
}

/// Implement this if you are building a custom renderer.
///
/// Check out first party implementations ([`crate::hooks::dx9`],
/// [`crate::hooks::dx11`], [`crate::hooks::dx12`], [`crate::hooks::opengl3`])
/// for guidance on how to implement the methods.
pub trait ImguiWindowsEventHandler {
    fn io(&self) -> &imgui::Io;
    fn io_mut(&mut self) -> &mut imgui::Io;

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

impl Default for DummyHwnd {
    fn default() -> Self {
        Self::new()
    }
}

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
            hInstance: unsafe { GetModuleHandleW(None).unwrap().into() },
            hIcon: HICON(0),
            hCursor: HCURSOR(0),
            hbrBackground: HBRUSH(0),
            lpszMenuName: PCWSTR(null()),
            lpszClassName: w!("HUDHOOK"),
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
                None,
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
            DestroyWindow(self.0).expect("DestroyWindow");
            UnregisterClassW(self.1.lpszClassName, self.1.hInstance).expect("DestroyWindow");
        }
    }
}
