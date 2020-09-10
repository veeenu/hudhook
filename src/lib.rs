// pub mod common;
mod hook;
mod inject;
pub mod memory;
mod mh;
mod util;
mod imgui_impl {
  pub mod dx11;
}

pub mod prelude {
  //
  // Reexports
  //
  pub use crate::hook::{apply_hook, RenderContext, RenderLoop};
  pub use crate::inject::inject;
  pub use crate::memory;
  pub use crate::util::{get_dll_path, Error};

  //
  // Library reexports
  //
  pub use imgui;
  pub use winapi;

  pub use crate::hook;
}

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
macro_rules! hook {
  ($e:expr) => {
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
        // TODO leave this for the client
        /*log_panics::init();

        unsafe {
          winapi::um::consoleapi::AllocConsole();
        }

        CombinedLogger::init(vec![
          TermLogger::new(LevelFilter::Trace, Config::default(), TerminalMode::Mixed),
          WriteLogger::new(
            LevelFilter::Trace,
            Config::default(),
            std::fs::File::create("hudhook.log").unwrap(),
          ),
        ])
        .unwrap();*/

        trace!("DllMain()");
        thread::spawn(|| {
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
