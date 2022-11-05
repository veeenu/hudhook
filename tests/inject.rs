use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use hudhook::inject::inject;

#[test]
fn test_inject() {
    let mut child = Command::new("notepad.exe").spawn().expect("Couldn't start notepad");
    std::thread::sleep(Duration::from_millis(500));
    println!("Should show a message box that says \"Hello\".");

    inject("Untitled - Notepad", examples_path().join("dummy_hook.dll")).unwrap();

    std::thread::sleep(Duration::from_millis(1000));
    child.kill().expect("Couldn't kill notepad");
}

fn examples_path() -> PathBuf {
    project_root().join("target").join("debug").join("examples")
}

fn project_root() -> PathBuf {
    Path::new(&env!("CARGO_MANIFEST_DIR")).to_path_buf()
}
