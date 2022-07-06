#![feature(once_cell)]
//! # hudhook
//!
//! This library implements a mechanism for hooking into the render loop of
//! DirectX11 applications, perform memory manipulation and draw things on
//! screen via [`imgui`](https://docs.rs/imgui/0.4.0/imgui/). It has been
//! largely inspired by [CheatEngine](https://www.cheatengine.org/).
//!
//! It's been extracted out of the [`darksoulsiii-practice-tool`](https://github.com/veeenu/darksoulsiii-practice-tool)
//! for generalized usage as a stand-alone framework. It is also a complete,
//! fully-fledged example of usage; it is a good idea to refer to that for
//! any doubts about the API which aren't clarified by this documentation.
//!
//! Refer to [this post](https://veeenu.github.io/blog/sekiro-practice-tool-architecture/)
//! for in-depth information about the architecture of the library.
//!
//! ## Fair warning
//!
//! `hudhook` provides essential, crash-safe features for memory manipulation
//! and UI rendering. It does, alas, contain a hefty amount of FFI and `unsafe`
//! code which still has to be thoroughly tested, validated and audited for
//! soundness. It should be OK for small projects such as videogame mods, but
//! it may crash your application at this stage.
//!
//! ## Examples
//!
//! ### Hooking the render loop and drawing things with `imgui`:
//!
//! Compile your crate with both a `cdylib` and an executable target. The
//! executable will be very minimal and used to inject the DLL into the
//! target process.
//!
//! #### Building the render loop
//!
//! Implement the [`RenderLoop`] trait
//!
//! ```no_run
//! // lib.rs
//! use hudhook::hooks::dx11;
//! use hudhook::*;
//!
//! pub struct MyRenderLoop;
//!
//! impl dx11::ImguiRenderLoop for MyRenderLoop {
//!     fn render(&self, ctx: hudhook::RenderContext) {
//!         imgui::Window::new(im_str!("My first render loop"))
//!             .position([0., 0.], imgui::Condition::FirstUseEver)
//!             .size([320., 200.], imgui::Condition::FirstUseEver)
//!             .build(ctx.frame, || {
//!                 ctx.frame.text(imgui::im_str!("Hello, hello!"));
//!             });
//!     }
//!
//!     fn is_visible(&self) -> bool {
//!         true
//!     }
//!
//!     fn is_capturing(&self) -> bool {
//!         true
//!     }
//! }
//!
//! hudhook!(MyRenderLoop.into_hook())
//! ```
//!
//! ```no_run
//! // main.rs
//! use hudhook::inject;
//!
//! fn main() {
//!     let mut cur_exe = std::env::current_exe().unwrap();
//!     cur_exe.push("..");
//!     cur_exe.push("libmyhook.dll");
//!
//!     let cur_dll = cur_exe.canonicalize().unwrap();
//!
//!     inject("MyTargetApplication.exe", cur_dll.as_path().to_str().unwrap()).unwrap();
//! }
//! ```
//!
//! ### Memory manipulation
//!
//! In an initialization step:
//!
//! ```no_run
//! let x = PointerChain::<f32>::new(&[base_address, 0x40, 0x28, 0x80]);
//! let y = PointerChain::<f32>::new(&[base_address, 0x40, 0x28, 0x88]);
//! let z = PointerChain::<f32>::new(&[base_address, 0x40, 0x28, 0x84]);
//! ```
//!
//! In the render loop:
//!
//! ```no_run
//! x.read().map(|val| x.write(val + 1.));
//! y.read().map(|val| y.write(val + 1.));
//! z.read().map(|val| z.write(val + 1.));
//! ```
#![allow(clippy::needless_doctest_main)]

pub mod hooks;
pub mod inject;

pub mod utils {
    use std::sync::atomic::{AtomicBool, Ordering};

    static CONSOLE_ALLOCATED: AtomicBool = AtomicBool::new(false);

    /// Allocate a Windows console.
    pub fn alloc_console() {
        if !CONSOLE_ALLOCATED.swap(true, Ordering::SeqCst) {
            unsafe {
                crate::reexports::AllocConsole();
            }
        }
    }

    /// Initialize `simplelog` with sane defaults.
    pub fn simplelog() {
        use log::*;
        use simplelog::*;

        TermLogger::init(
            LevelFilter::Trace,
            Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        )
        .ok();
    }

    /// Free the previously allocated Windows console.
    pub fn free_console() {
        if CONSOLE_ALLOCATED.swap(false, Ordering::SeqCst) {
            unsafe {
                crate::reexports::FreeConsole();
            }
        }
    }
}

pub use log;

pub mod reexports {
    pub use detour::RawDetour;
    pub use windows::Win32::Foundation::HINSTANCE;
    pub use windows::Win32::System::Console::{AllocConsole, FreeConsole};
    pub use windows::Win32::System::SystemServices::{DLL_PROCESS_ATTACH, DLL_PROCESS_DETACH};
}

pub mod global_state {
    use std::cell::OnceCell;
    use std::thread;

    use windows::Win32::Foundation::HINSTANCE;
    use windows::Win32::System::LibraryLoader::FreeLibraryAndExitThread;

    use crate::{hooks, utils};

    static mut MODULE: OnceCell<HINSTANCE> = OnceCell::new();
    static mut HOOKS: OnceCell<Box<dyn hooks::Hooks>> = OnceCell::new();

    pub fn set_module(module: HINSTANCE) {
        unsafe {
            MODULE.set(module).unwrap();
        }
    }

    pub fn get_module() -> HINSTANCE {
        unsafe { MODULE.get().unwrap().clone() }
    }

    pub fn set_hooks(hooks: Box<dyn hooks::Hooks>) {
        unsafe { HOOKS.set(hooks).ok() };
    }

    pub fn eject() {
        thread::spawn(|| unsafe {
            utils::free_console();

            if let Some(mut hooks) = HOOKS.take() {
                hooks.unhook();
            }

            if let Some(module) = MODULE.take() {
                FreeLibraryAndExitThread(module, 0);
            }
        });
    }
}

/// Entry point for the library.
///
/// Example usage:
/// ```no_run
/// pub struct MyRenderLoop;
///
/// impl RenderLoop for MyRenderLoop {
///   fn render(&self, frame: imgui::Ui, flags: &ImguiRenderLoopFlags) { ... }
/// }
///
/// hudhook!(MyRenderLoop.into_hook());
/// ```
#[macro_export]
macro_rules! hudhook {
    ($hooks:expr) => {
        use hudhook::log::*;
        use hudhook::reexports::*;
        use hudhook::*;

        /// Entry point created by the `hudhook` library.
        #[no_mangle]
        pub unsafe extern "stdcall" fn DllMain(
            hmodule: HINSTANCE,
            reason: u32,
            _: *mut std::ffi::c_void,
        ) {
            if reason == DLL_PROCESS_ATTACH {
                hudhook::global_state::set_module(hmodule);

                trace!("DllMain()");
                std::thread::spawn(move || {
                    let hooks: Box<dyn hooks::Hooks> = { $hooks };
                    hooks.hook();
                    hudhook::global_state::set_hooks(hooks);
                    // let hooks: Vec<RawDetour> = { $hooks };
                    // for hook in &hooks {
                    //     if let Err(e) = hook.enable() {
                    //         error!("Couldn't enable hook: {e}");
                    //     }
                    // }
                });
            } else if reason == DLL_PROCESS_DETACH {
                // TODO trigger drops on exit:
                // - Store _hmodule in a static OnceCell
                // - Wait for a render loop to be complete
                // - Call FreeLibraryAndExitThread from a utility function
                // This branch will then get called.
                // trace!("Unapplying hooks");
                // if let Some(mut hooks) = HOOKS.take() {
                //     hooks.unhook();
                //     // hooks.iter().for_each(|hook| {
                //     //     if let Err(e) = hook.disable() {
                //     //         error!("Error disabling hook: {e}");
                //     //     }
                //     // });
                // }
            }
        }
    };
}
