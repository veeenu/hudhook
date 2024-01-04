//! # hudhook
//!
//! This library implements a mechanism for hooking into the
//! render loop of applications and drawing things on screen via
//! [`imgui`](https://docs.rs/imgui/0.11.0/imgui/). It has been largely inspired
//! by [CheatEngine](https://www.cheatengine.org/).
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
//! Implement the [`hooks::ImguiRenderLoop`] trait:
//!
//! ```no_run
//! // lib.rs
//! use hudhook::hooks::ImguiRenderLoop;
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
//!     hudhook!(MyRenderLoop.into_hook::<ImguiDx9Hooks>());
//! }
//!
//! {
//!     // Use this if hooking into a DirectX 11 application.
//!     use hudhook::hooks::dx11::ImguiDx11Hooks;
//!     hudhook!(MyRenderLoop.into_hook::<ImguiDx11Hooks>());
//! }
//!
//! {
//!     // Use this if hooking into a DirectX 12 application.
//!     use hudhook::hooks::dx12::ImguiDx12Hooks;
//!     hudhook!(MyRenderLoop.into_hook::<ImguiDx12Hooks>());
//! }
//!
//! {
//!     // Use this if hooking into a OpenGL 3 application.
//!     use hudhook::hooks::opengl3::ImguiOpenGl3Hooks;
//!     hudhook!(MyRenderLoop.into_hook::<ImguiOpenGl3Hooks>());
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

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use once_cell::sync::OnceCell;
use tracing::error;
use windows::core::Error;
pub use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::System::Console::{
    AllocConsole, FreeConsole, GetConsoleMode, GetStdHandle, SetConsoleMode, CONSOLE_MODE,
    ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_OUTPUT_HANDLE,
};
use windows::Win32::System::LibraryLoader::FreeLibraryAndExitThread;
pub use windows::Win32::System::SystemServices::{DLL_PROCESS_ATTACH, DLL_PROCESS_DETACH};
pub use {imgui, tracing};

use crate::hooks::Hooks;
use crate::mh::{MH_ApplyQueued, MH_Initialize, MH_Uninitialize, MhHook, MH_STATUS};
pub use crate::render_loop::ImguiRenderLoop;

pub mod hooks;
#[cfg(feature = "inject")]
pub mod inject;
pub mod mh;
pub mod render_loop;
pub mod renderer;

mod util;

// Global state objects.
static mut MODULE: OnceCell<HINSTANCE> = OnceCell::new();
static mut HUDHOOK: OnceCell<Hudhook> = OnceCell::new();
static CONSOLE_ALLOCATED: AtomicBool = AtomicBool::new(false);

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

            // call GetConsoleMode to get the current mode of the console
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
/// been created before) and invoke `FreeLibraryAndExitThread`.
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
///                 .with(MyRenderLoop.into_hook::<ImguiDx12Hooks>())
///                 .with_hmodule(hmodule)
///                 .build();
///             hooks.apply();
///         });
///     }
/// }

pub struct HudhookBuilder(Hudhook);

impl HudhookBuilder {
    /// Add a hook object.
    pub fn with(mut self, hook: Box<dyn Hooks>) -> Self {
        self.0 .0.push(hook);
        self
    }

    /// Save the DLL instance (for the eject method).
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
    ($hooks:expr) => {
        use hudhook::tracing::*;
        use hudhook::*;

        /// Entry point created by the `hudhook` library.
        #[no_mangle]
        pub unsafe extern "stdcall" fn DllMain(
            hmodule: ::hudhook::HINSTANCE,
            reason: u32,
            _: *mut ::std::ffi::c_void,
        ) {
            if reason == DLL_PROCESS_ATTACH {
                trace!("DllMain()");
                ::std::thread::spawn(move || {
                    if let Err(e) =
                        Hudhook::builder().with({ $hooks }).with_hmodule(hmodule).build().apply()
                    {
                        error!("Couldn't apply hooks: {e:?}");
                        eject();
                    }
                });
            }
        }
    };
}
