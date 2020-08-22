#[macro_use]
extern crate imgui;
extern crate simplelog;

// pub mod common;
pub mod hook;
pub mod inject;
pub mod mh;
pub mod util;
pub mod imgui_impl {
  pub mod dx11;
}
pub mod memory;

//
// Reexports
//
pub use hook::RenderLoop;
pub use inject::inject;
pub use util::Error;

/// Entry point for the library.
///
/// Example usage:
/// ```
/// pub struct MyRenderLoop;
/// impl MyRenderLoop {
///   fn new() -> MyRenderLoop { ... }
/// }
/// impl RenderLoop for MyRenderLoop {
///   fn render(&self, frame: imgui::Ui) { ... }
/// }
///
/// hook!(MyRenderLoop::new())
/// ```
///
#[macro_export]
macro_rules! hook {
  ($e:expr) => {
    use log::*;
    use log_panics;
    use simplelog::*;
    use std::thread;

    #[no_mangle]
    pub extern "stdcall" fn DllMain(
      _: winapi::shared::minwindef::HINSTANCE,
      reason: u32,
      _: *mut winapi::ctypes::c_void,
    ) {
      if reason == 1 {
        log_panics::init();

        unsafe {
          winapi::um::consoleapi::AllocConsole();
        }

        // TODO leave this for the client
        CombinedLogger::init(vec![
          TermLogger::new(LevelFilter::Trace, Config::default(), TerminalMode::Mixed).unwrap(),
          WriteLogger::new(
            LevelFilter::Trace,
            Config::default(),
            std::fs::File::create("hudhook.log").unwrap(),
          ),
        ])
        .unwrap();

        debug!("DllMain()");
        thread::spawn(|| {
          debug!("Started thread, enabling hook...");
          match hook::hook($e) {
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
