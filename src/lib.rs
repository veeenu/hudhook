#[macro_use]
extern crate imgui;
extern crate simplelog;

pub mod inject;

pub mod mh;
pub mod hook;
pub mod util;
pub mod imgui_impl {
  pub mod dx11;
}

use log::*;
use simplelog::*;
use log_panics;
use std::thread;

#[no_mangle]
pub extern "stdcall" fn DllMain(_: winapi::shared::minwindef::HINSTANCE, reason: u32, _: *mut winapi::ctypes::c_void) {
  if reason == 1 {
    log_panics::init();

    unsafe { winapi::um::consoleapi::AllocConsole(); }
    // SimpleLogger::init(LevelFilter::Trace, Config::default());
    CombinedLogger::init(
      vec![
        TermLogger::new(LevelFilter::Trace, Config::default(), TerminalMode::Mixed).unwrap(),
        WriteLogger::new(LevelFilter::Trace, Config::default(), std::fs::File::create("hudhook.log").unwrap()),
      ]
    ).unwrap();
    info!("DllMain()");
    thread::spawn(|| {
      info!("Started thread, enabling hook...");
      match hook::hook() {
        Ok(_) => {
          info!("Hook enabled");
        },
        Err(e) => {
          error!("Hook errored: {:?}", e);
        }
      }
    });
  }
}