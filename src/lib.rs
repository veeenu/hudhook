//! # hudhook
//!
//! This library implements a mechanism for hooking into the
//! render loop of applications and drawing things on screen via
//! [`dear imgui`](https://docs.rs/imgui/0.11.0/imgui/).
//!
//! Currently, DirectX9, DirectX 11, DirectX 12 and OpenGL 3 are supported.
//!
//! For complete, fully fledged examples of usage, check out the following
//! projects:
//!
//! - [`darksoulsiii-practice-tool`](https://github.com/veeenu/darksoulsiii-practice-tool)
//! - [`eldenring-practice-tool`](https://github.com/veeenu/eldenring-practice-tool)
//!
//! It is a good idea to refer to these projects for any doubts about the API
//! which aren't clarified by this documentation, as this project is directly
//! derived from them.
//!
//! Refer to [this post](https://veeenu.github.io/blog/sekiro-practice-tool-architecture/) for
//! in-depth information about the architecture of the library.
//!
//! A [tutorial book](https://veeenu.github.io/hudhook/) is also available, with end-to-end
//! examples.
//!
//! [`darksoulsiii-practice-tool`]: https://github.com/veeenu/darksoulsiii-practice-tool
//! [`eldenring-practice-tool`]: https://github.com/veeenu/eldenring-practice-tool
//!
//! ## Fair warning
//!
//! [`hudhook`](crate) provides essential, crash-safe features for memory
//! manipulation and UI rendering. It does, alas, contain a hefty amount of FFI
//! and `unsafe` code which still has to be thoroughly tested, validated and
//! audited for soundness. It should be OK for small projects such as videogame
//! mods, but it may crash your application at this stage.
//!
//! ## Examples
//!
//! ### Hooking the render loop and drawing things with `imgui`
//!
//! Compile your crate with both a `cdylib` and an executable target. The
//! executable will be very minimal and used to inject the DLL into the
//! target process.
//!
//! #### Building the render loop
//!
//! Implement the render loop trait for your hook target.
//!
//! ##### Example
//!
//! Implement the [`ImguiRenderLoop`] trait:
//!
//! ```no_run
//! // lib.rs
//! use hudhook::*;
//!
//! pub struct MyRenderLoop;
//!
//! impl ImguiRenderLoop for MyRenderLoop {
//!     fn render(&mut self, ui: &mut imgui::Ui) {
//!         ui.window("My first render loop")
//!             .position([0., 0.], imgui::Condition::FirstUseEver)
//!             .size([320., 200.], imgui::Condition::FirstUseEver)
//!             .build(|| {
//!                 ui.text("Hello, hello!");
//!             });
//!     }
//! }
//!
//! {
//!     // Use this if hooking into a DirectX 9 application.
//!     use hudhook::hooks::dx9::ImguiDx9Hooks;
//!     hudhook!(ImguiDx9Hooks, MyRenderLoop);
//! }
//!
//! {
//!     // Use this if hooking into a DirectX 11 application.
//!     use hudhook::hooks::dx11::ImguiDx11Hooks;
//!     hudhook!(ImguiDx11Hooks, MyRenderLoop);
//! }
//!
//! {
//!     // Use this if hooking into a DirectX 12 application.
//!     use hudhook::hooks::dx12::ImguiDx12Hooks;
//!     hudhook!(ImguiDx12Hooks, MyRenderLoop);
//! }
//!
//! {
//!     // Use this if hooking into a OpenGL 3 application.
//!     use hudhook::hooks::opengl3::ImguiOpenGl3Hooks;
//!     hudhook!(ImguiOpenGl3Hooks, MyRenderLoop);
//! }
//! ```
//!
//! #### Injecting the DLL
//!
//! You can use the facilities in [`inject`] in your binaries to inject
//! the DLL in your target process.
//!
//! ```no_run
//! // main.rs
//! use hudhook::inject::Process;
//!
//! fn main() {
//!     let mut cur_exe = std::env::current_exe().unwrap();
//!     cur_exe.push("..");
//!     cur_exe.push("libmyhook.dll");
//!
//!     let cur_dll = cur_exe.canonicalize().unwrap();
//!
//!     Process::by_name("MyTargetApplication.exe").unwrap().inject(cur_dll).unwrap();
//! }
//! ```
#![allow(clippy::needless_doctest_main)]
#![deny(missing_docs)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use imgui::{Context, Io, TextureId, Ui};
use once_cell::sync::OnceCell;
use tracing::error;
use windows::core::Error;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::System::Console::{
    AllocConsole, FreeConsole, GetConsoleMode, GetStdHandle, SetConsoleMode, CONSOLE_MODE,
    ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_OUTPUT_HANDLE,
};
use windows::Win32::System::LibraryLoader::FreeLibraryAndExitThread;
pub use {imgui, tracing, windows};

use crate::mh::{MH_ApplyQueued, MH_Initialize, MH_Uninitialize, MhHook, MH_STATUS};

pub mod hooks;
#[cfg(feature = "inject")]
pub mod inject;
pub mod mh;
pub(crate) mod renderer;

pub use renderer::msg_filter::MessageFilter;

pub mod util;

// Global state objects.
static mut MODULE: OnceCell<HINSTANCE> = OnceCell::new();
static mut HUDHOOK: OnceCell<Hudhook> = OnceCell::new();
static CONSOLE_ALLOCATED: AtomicBool = AtomicBool::new(false);

/// Texture Loader for ImguiRenderLoop callbacks to load and replace textures
pub trait RenderContext {
    /// Load texture and return TextureId to use. Invoke it in your
    /// [`crate::ImguiRenderLoop::initialize`] method for setting up textures.
    fn load_texture(&mut self, data: &[u8], width: u32, height: u32) -> Result<TextureId, Error>;

    /// Upload an image to an existing texture, replacing its content. Invoke it
    /// in your [`crate::ImguiRenderLoop::before_render`] method for
    /// updating textures.
    fn replace_texture(
        &mut self,
        texture_id: TextureId,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(), Error>;
}

/// Allocate a Windows console.
pub fn alloc_console() -> Result<(), Error> {
    if !CONSOLE_ALLOCATED.swap(true, Ordering::SeqCst) {
        unsafe { AllocConsole()? };
    }

    Ok(())
}

/// Enable console colors if the console is allocated.
pub fn enable_console_colors() {
    if CONSOLE_ALLOCATED.load(Ordering::SeqCst) {
        unsafe {
            // Get the stdout handle
            let stdout_handle = GetStdHandle(STD_OUTPUT_HANDLE).unwrap();

            // Call GetConsoleMode to get the current mode of the console
            let mut current_console_mode = CONSOLE_MODE(0);
            GetConsoleMode(stdout_handle, &mut current_console_mode).unwrap();

            // Set the new mode to include ENABLE_VIRTUAL_TERMINAL_PROCESSING for ANSI
            // escape sequences
            current_console_mode.0 |= ENABLE_VIRTUAL_TERMINAL_PROCESSING.0;

            // Call SetConsoleMode to set the new mode
            SetConsoleMode(stdout_handle, current_console_mode).unwrap();
        }
    }
}

/// Free the previously allocated Windows console.
pub fn free_console() -> Result<(), Error> {
    if CONSOLE_ALLOCATED.swap(false, Ordering::SeqCst) {
        unsafe { FreeConsole()? };
    }

    Ok(())
}

/// Disable hooks and eject the DLL.
///
/// ## Ejecting a DLL
///
/// To eject your DLL, invoke the [`eject`] method from anywhere in your
/// render loop. This will disable the hooks, free the console (if it has
/// been created before) and invoke
/// [`windows::Win32::System::LibraryLoader::FreeLibraryAndExitThread`].
///
/// Befor calling [`eject`], make sure to perform any manual cleanup (e.g.
/// dropping/resetting the contents of static mutable variables).
pub fn eject() {
    thread::spawn(|| unsafe {
        if let Err(e) = free_console() {
            error!("{e:?}");
        }

        if let Some(mut hudhook) = HUDHOOK.take() {
            if let Err(e) = hudhook.unapply() {
                error!("Couldn't unapply hooks: {e:?}");
            }
        }

        if let Some(module) = MODULE.take() {
            FreeLibraryAndExitThread(module, 0);
        }
    });
}

/// Implement your `imgui` rendering logic via this trait.
pub trait ImguiRenderLoop {
    /// Called once at the first occurrence of the hook. Implement this to
    /// initialize your data.
    /// `ctx` is the imgui context, and `render_context` is meant to access
    /// hudhook renderers' extensions such as texture management.
    fn initialize<'a>(
        &'a mut self,
        _ctx: &mut Context,
        _render_context: &'a mut dyn RenderContext,
    ) {
    }

    /// Called before rendering each frame. Use the provided `ctx` object to
    /// modify imgui settings before rendering the UI.
    /// `ctx` is the imgui context, and `render_context` is meant to access
    /// hudhook renderers' extensions such as texture management.
    fn before_render<'a>(
        &'a mut self,
        _ctx: &mut Context,
        _render_context: &'a mut dyn RenderContext,
    ) {
    }

    /// Called every frame. Use the provided `ui` object to build your UI.
    fn render(&mut self, ui: &mut Ui);

    /// Called during the window procedure.
    fn on_wnd_proc(&self, _hwnd: HWND, _umsg: u32, _wparam: WPARAM, _lparam: LPARAM) {}

    /// Returns the types of window message that
    /// you do not want to propagate to the main window
    fn message_filter(&self, _io: &Io) -> MessageFilter {
        MessageFilter::empty()
    }
}

/// Generic trait for platform-specific hooks.
///
/// Implement this if you are building a custom hook for a non-supported
/// renderer.
///
/// Check out first party implementations for guidance on how to implement the
/// methods:
/// - [`ImguiDx9Hooks`](crate::hooks::dx9::ImguiDx9Hooks)
/// - [`ImguiDx11Hooks`](crate::hooks::dx11::ImguiDx11Hooks)
/// - [`ImguiDx12Hooks`](crate::hooks::dx12::ImguiDx12Hooks)
/// - [`ImguiOpenGl3Hooks`](crate::hooks::opengl3::ImguiOpenGl3Hooks)
pub trait Hooks {
    /// Construct a boxed instance of the implementor, storing the provided
    /// render loop where appropriate.
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

/// Holds all the activated hooks and manages their lifetime.
pub struct Hudhook(Vec<Box<dyn Hooks>>);
unsafe impl Send for Hudhook {}
unsafe impl Sync for Hudhook {}

impl Hudhook {
    /// Create a builder object.
    pub fn builder() -> HudhookBuilder {
        HudhookBuilder(Hudhook::new())
    }

    fn new() -> Self {
        // Initialize minhook.
        match unsafe { MH_Initialize() } {
            MH_STATUS::MH_ERROR_ALREADY_INITIALIZED | MH_STATUS::MH_OK => {},
            status @ MH_STATUS::MH_ERROR_MEMORY_ALLOC => panic!("MH_Initialize: {status:?}"),
            _ => unreachable!(),
        }

        Hudhook(Vec::new())
    }

    /// Return an iterator of all the activated raw hooks.
    fn hooks(&self) -> impl IntoIterator<Item = &MhHook> {
        self.0.iter().flat_map(|h| h.hooks())
    }

    /// Apply the hooks.
    pub fn apply(self) -> Result<(), MH_STATUS> {
        // Queue enabling all the hooks.
        for hook in self.hooks() {
            unsafe { hook.queue_enable()? };
        }

        // Apply the queue of enable actions.
        unsafe { MH_ApplyQueued().ok_context("MH_ApplyQueued")? };

        unsafe { HUDHOOK.set(self).ok() };

        Ok(())
    }

    /// Disable and cleanup the hooks.
    pub fn unapply(&mut self) -> Result<(), MH_STATUS> {
        // Queue disabling all the hooks.
        for hook in self.hooks() {
            unsafe { hook.queue_disable()? };
        }

        // Apply the queue of disable actions.
        unsafe { MH_ApplyQueued().ok_context("MH_ApplyQueued")? };

        // Uninitialize minhook.
        unsafe { MH_Uninitialize().ok_context("MH_Uninitialize")? };

        // Invoke cleanup for all hooks.
        for hook in &mut self.0 {
            unsafe { hook.unhook() };
        }

        Ok(())
    }
}

/// Builder object for [`Hudhook`].
///
/// Example usage:
/// ```no_run
/// use hudhook::hooks::dx12::ImguiDx12Hooks;
/// use hudhook::hooks::ImguiRenderLoop;
/// use hudhook::*;
///
/// pub struct MyRenderLoop;
///
/// impl ImguiRenderLoop for MyRenderLoop {
///     fn render(&mut self, frame: &mut imgui::Ui) {
///         // ...
///     }
/// }
///
/// #[no_mangle]
/// pub unsafe extern "stdcall" fn DllMain(
///     hmodule: HINSTANCE,
///     reason: u32,
///     _: *mut std::ffi::c_void,
/// ) {
///     if reason == DLL_PROCESS_ATTACH {
///         std::thread::spawn(move || {
///             let hooks = Hudhook::builder()
///                 .with::<ImguiDx12Hooks>(MyRenderLoop())
///                 .with_hmodule(hmodule)
///                 .build();
///             hooks.apply();
///         });
///     }
/// }

pub struct HudhookBuilder(Hudhook);

impl HudhookBuilder {
    /// Add a hook object.
    pub fn with<T: Hooks + 'static>(
        mut self,
        render_loop: impl ImguiRenderLoop + Send + Sync + 'static,
    ) -> Self {
        self.0 .0.push(T::from_render_loop(render_loop));
        self
    }

    /// Save the DLL instance (for the [`eject`] method).
    pub fn with_hmodule(self, module: HINSTANCE) -> Self {
        unsafe { MODULE.set(module).unwrap() };
        self
    }

    /// Build the [`Hudhook`] object.
    pub fn build(self) -> Hudhook {
        self.0
    }
}

/// Entry point generator for the library.
///
/// After implementing your [render loop](crate::hooks) of choice, invoke
/// the macro to generate the `DllMain` function that will serve as entry point
/// for your hook.
///
/// Example usage:
/// ```no_run
/// use hudhook::hooks::dx12::ImguiDx12Hooks;
/// use hudhook::hooks::ImguiRenderLoop;
/// use hudhook::*;
///
/// pub struct MyRenderLoop;
///
/// impl ImguiRenderLoop for MyRenderLoop {
///     fn render(&mut self, frame: &mut imgui::Ui) {
///         // ...
///     }
/// }
///
/// hudhook::hudhook!(MyRenderLoop.into_hook::<ImguiDx12Hooks>());
/// ```
#[macro_export]
macro_rules! hudhook {
    ($t:ty, $hooks:expr) => {
        /// Entry point created by the `hudhook` library.
        #[no_mangle]
        pub unsafe extern "stdcall" fn DllMain(
            hmodule: ::hudhook::windows::Win32::Foundation::HINSTANCE,
            reason: u32,
            _: *mut ::std::ffi::c_void,
        ) {
            use ::hudhook::*;

            if reason == ::hudhook::windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH {
                ::hudhook::tracing::trace!("DllMain()");
                ::std::thread::spawn(move || {
                    if let Err(e) = ::hudhook::Hudhook::builder()
                        .with::<$t>({ $hooks })
                        .with_hmodule(hmodule)
                        .build()
                        .apply()
                    {
                        ::hudhook::tracing::error!("Couldn't apply hooks: {e:?}");
                        ::hudhook::eject();
                    }
                });
            }
        }
    };
}
