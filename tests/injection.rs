use hudhook::inject;
use std::process::Command;

#[test]
fn test_run_against_sample() {
  let mut child = Command::new("tests/test_sample.exe")
    .spawn()
    .expect("Failed to run child process");
  std::thread::sleep(std::time::Duration::from_millis(250));

  // let pid = inject::find_process("sample.exe").expect("Process not found");
  inject("test_sample.exe", "target/release/examples/hello_world.dll").unwrap();

  child.wait().expect("Child process error");
}
