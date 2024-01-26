//! # How to implement a hook
//!
//! The code structure of a hook should follow this scheme:
//!
//! 1. `use` statements
//! 2. Type aliases for hooked functions
//! 3. A `struct Trampolines` that should hold all needed trampolines
//! 4. `static mut` objects, favoring `OnceLock`s and keeping heavy sync
//!    primitives like mutexes to the minimum absolute necessary
//! 5. Hook function implementations, one for each trampoline
//! 6. An `imgui_wnd_proc` implementation
//! 7. A `render` function that locks the render engine and uses it to draw and
//!    swap. Possibly implement a critical section with an `AtomicBool` to prevent
//!    double invocations
//! 8. A `get_target_addrs` function that retrieves hooked function addresses
//! 9. A `Imgui<something>Hooks` type that holds necessary hook state (generally
//!    just a static array of `MhHook` objects); implement a `new` function and all
//!    necessary methods/associated functions, then implement the `Hooks` trait
//!    below it
use std::{mem, sync::OnceLock};

use tracing::{debug, error};
use windows::{
    core::w,
    Win32::{
        Foundation::{BOOL, HWND, LPARAM, LRESULT, WPARAM},
        System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentProcessId},
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, EnumWindows, GetWindowThreadProcessId,
            RegisterClassExW, UnregisterClassW, CS_HREDRAW, CS_VREDRAW, WNDCLASSEXW,
            WS_EX_OVERLAPPEDWINDOW, WS_OVERLAPPEDWINDOW,
        },
    },
};

pub mod dx11;
pub mod dx12;
pub mod dx9;
mod input;
pub mod opengl3;
mod render;

pub fn find_process_hwnd() -> Option<HWND> {
    static mut FOUND_HWND: OnceLock<HWND> = OnceLock::new();

    unsafe extern "system" fn enum_callback(hwnd: HWND, _: LPARAM) -> BOOL {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        tracing::debug!("hwnd {hwnd:?} has pid {pid} vs {}", GetCurrentProcessId());
        if pid == GetCurrentProcessId() {
            FOUND_HWND.get_or_init(|| hwnd);
            BOOL::from(false)
        } else {
            BOOL::from(true)
        }
    }

    unsafe {
        FOUND_HWND.take();
        EnumWindows(Some(enum_callback), LPARAM(0)).ok();
    }

    unsafe { FOUND_HWND.get().copied() }
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
            // hIcon: HICON(0),
            // hCursor: HCURSOR(0),
            // hbrBackground: HBRUSH(0),
            // lpszMenuName: PCWSTR(null()),
            lpszClassName: w!("HUDHOOK"),
            // hIconSm: HICON(0),
            ..Default::default()
        };
        debug!("{:?}", wndclass);
        unsafe { RegisterClassExW(&wndclass) };

        // Create the window.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_OVERLAPPEDWINDOW,
                wndclass.lpszClassName,
                w!("HUDHOOK"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                100,
                100,
                None,
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
            if let Err(e) = DestroyWindow(self.0) {
                error!("DestroyWindow: {e}");
            }
            if let Err(e) = UnregisterClassW(self.1.lpszClassName, self.1.hInstance) {
                error!("UnregisterClass: {e}");
            }
        }
    }
}
