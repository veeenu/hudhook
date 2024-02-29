//! Implementations of render engine hooks.

use std::mem;
use std::sync::OnceLock;

use tracing::{debug, error};
use windows::core::w;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, EnumWindows, GetWindowThreadProcessId,
    RegisterClassExW, UnregisterClassW, CS_HREDRAW, CS_VREDRAW, WNDCLASSEXW,
    WS_EX_OVERLAPPEDWINDOW, WS_OVERLAPPEDWINDOW,
};

#[cfg(feature = "dx11")]
pub mod dx11;
#[cfg(feature = "dx12")]
pub mod dx12;
#[cfg(feature = "dx9")]
pub mod dx9;
#[cfg(feature = "opengl3")]
pub mod opengl3;

/// A utility function to retrieve the top level [`HWND`] belonging to this
/// process.
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
    /// Construct the dummy [`HWND`].
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
            lpszClassName: w!("HUDHOOK"),
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

    /// Retrieve the window handle.
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
