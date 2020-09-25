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
//! ```
//! // lib.rs
//! use hudhook::*;
//!
//! pub struct MyRenderLoop;
//! impl RenderLoop for MyRenderLoop {
//!   fn render(&self, ctx: hudhook::RenderContext) {
//!    imgui::Window::new(im_str!("My first render loop"))
//!     .position([0., 0.], imgui::Condition::FirstUseEver)
//!     .size([320., 200.], imgui::Condition::FirstUseEver)
//!     .build(ctx.frame, || {
//!       ctx.frame.text(imgui::im_str!("Hello, hello!"));
//!     });
//!   }
//!
//!   fn is_visible(&self) -> bool { true }
//!   fn is_capturing(&self) -> bool { true }
//! }
//!
//! hudhook!(Box::new(MyRenderLoop::new()))
//! ```
//!
//! ```
//! // main.rs
//! use hudhook::inject;
//!
//! fn main() {
//!   let mut cur_exe = std::env::current_exe().unwrap();
//!   cur_exe.push("..");
//!   cur_exe.push("libmyhook.dll");
//!
//!   let cur_dll = cur_exe.canonicalize().unwrap();
//!
//!   inject("MyTargetApplication.exe", cur_dll.as_path().to_str().unwrap()).unwrap();
//! }
//! ```
//!
//! ### Memory manipulation
//!
//! In an initialization step:
//!
//! ```
//! let x = PointerChain::<f32>::new(&[base_address, 0x40, 0x28, 0x80]);
//! let y = PointerChain::<f32>::new(&[base_address, 0x40, 0x28, 0x88]);
//! let z = PointerChain::<f32>::new(&[base_address, 0x40, 0x28, 0x84]);
//! ```
//!
//! In the render loop:
//!
//! ```
//! x.read().map(|val| x.write(val + 1.));
//! y.read().map(|val| y.write(val + 1.));
//! z.read().map(|val| z.write(val + 1.));
//! ```

mod hook;
mod inject;
pub mod memory;
mod mh;
mod util;
mod imgui_impl {
  pub mod dx11;
}

//
// Reexports
//
pub use crate::hook::{apply_hook, RenderContext, RenderLoop};
pub use crate::inject::inject;
pub use crate::util::{get_dll_path, Error};

//
// Library reexports
//
pub use imgui;
pub use winapi;

/// Entry point for the library.
///
/// Example usage:
/// ```
/// pub struct MyRenderLoop;
/// impl RenderLoop for MyRenderLoop {
///   fn render(&self, frame: imgui::Ui) { ... }
/// }
///
/// hook!(Box::new(MyRenderLoop::new(...)))
/// ```
#[macro_export]
macro_rules! hudhook {
  ($e:expr) => {
    use log::*;
    use log_panics;
    use std::thread;

    /// Entry point created by the `hudhook` library.
    #[no_mangle]
    pub extern "stdcall" fn DllMain(
      _: winapi::shared::minwindef::HINSTANCE,
      reason: u32,
      _: *mut winapi::ctypes::c_void,
    ) {
      if reason == 1 {
        trace!("DllMain()");
        thread::spawn(move || {
          debug!("Started thread, enabling hook...");
          match apply_hook($e) {
            Ok(_) => {
              debug!("Hook enabled");
            }
            Err(e) => {
              error!("Hook errored: {:?}", e);
            }
          }
        });
      }
    }
  };
}
