use hudhook::*;
use simplelog::*;
use std::process::Command;
use std::path::Path;

fn example_hello_world() {
  let mut child = Command::new("tests/test_sample.exe")
    .spawn()
    .expect("Failed to run child process");
  std::thread::sleep(std::time::Duration::from_millis(250));

  // Build test_sample.exe from `lib/test_sample/test_sample.cpp`:
  // > cl /std:c++17 .\test_sample.cpp; .\test_sample.exe
  // > cp .\test_sample.exe tests
  inject("test_sample.exe", Path::new("target/release/examples/hello_world.dll")).unwrap();

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
