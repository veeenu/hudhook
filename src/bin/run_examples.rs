use hudhook::prelude::*;
use simplelog::*;
use std::process::Command;

fn example_hello_world() {
  let mut child = Command::new("tests/test_sample.exe")
    .spawn()
    .expect("Failed to run child process");
  std::thread::sleep(std::time::Duration::from_millis(250));

  // let pid = inject::find_process("sample.exe").expect("Process not found");
  inject("test_sample.exe", "target/release/examples/hello_world.dll").unwrap();

  child.wait().expect("Child process error");
}

fn main() {
  CombinedLogger::init(vec![TermLogger::new(
    LevelFilter::Trace,
    Config::default(),
    TerminalMode::Mixed,
  )
  .unwrap()])
  .unwrap();
  example_hello_world();
}
