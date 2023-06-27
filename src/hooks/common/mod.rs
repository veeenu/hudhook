use std::hint;
use std::sync::atomic::{AtomicBool, Ordering};

use imgui::{Context, Io, Key, Ui};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

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
