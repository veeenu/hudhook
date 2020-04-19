use std::process::Command;
use hudhook::inject;

#[test]
fn test_run_against_sample() {
  let mut child = Command::new("tests/test_sample.exe")
    .spawn()
    .expect("Failed to run child process");
  std::thread::sleep(std::time::Duration::from_millis(250));

  // let pid = inject::find_process("sample.exe").expect("Process not found");
  inject::inject(child.id(), "target/release/hudhook.dll");

  child.wait().expect("Child process error");
}