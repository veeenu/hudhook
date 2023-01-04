# hudhook

[![book](https://img.shields.io/badge/docs-book-brightgreen)](https://veeenu.github.io/hudhook)
[![rustdoc](https://img.shields.io/badge/docs-rustdoc-brightgreen)](https://veeenu.github.io/hudhook/rustdoc/hudhook)

A render loop hook library with [Dear ImGui](https://github.com/ocornut/imgui)
overlays, largely inspired by [CheatEngine](https://cheatengine.org/).

Currently supports DirectX 9, DirectX 11, DirectX 12 and OpenGL 3.

Compiles on Rust nightly only.

Read the tutorial book [here](https://veeenu.github.io/hudhook).

Read the API reference [here](https://veeenu.github.io/hudhook/rustdoc/hudhook).

Read up on the underlying architecture [here](https://veeenu.github.io/blog/sekiro-practice-tool-architecture/).

## Example

```rust
// src/lib.rs
use hudhook::hooks::dx11::ImguiDX11Hooks;
use hudhook::hooks::{ImguiRenderLoop, ImguiRenderLoopFlags};
use imgui::{Condition, Window};
struct Dx11HookExample;

impl Dx11HookExample {
    fn new() -> Self {
        println!("Initializing");
        hudhook::utils::alloc_console();
        hudhook::utils::simplelog();

        Dx11HookExample
    }
}

impl ImguiRenderLoop for Dx11HookExample {
    fn render(&mut self, ui: &mut imgui::Ui, _: &ImguiRenderLoopFlags) {
        ui.window("Hello world").size([300.0, 110.0], Condition::FirstUseEver).build(|| {
            ui.text("Hello world!");
            ui.text("こんにちは世界！");
            ui.text("This...is...imgui-rs!");
            ui.separator();
            let mouse_pos = ui.io().mouse_pos;
            ui.text(format!("Mouse Position: ({:.1},{:.1})", mouse_pos[0], mouse_pos[1]));
        });
    }
}

hudhook::hudhook!(Dx11HookExample::new().into_hook::<ImguiDX11Hooks>());
```

```rust
// src/main.rs
use hudhook::inject;
use std::process::Command;

#[test]
fn test_run_against_sample() {
    let mut child = Command::new("my_dx11_application.exe")
        .spawn()
        .expect("Failed to run child process");
    std::thread::sleep(std::time::Duration::from_millis(250));

    inject::inject("my_dx11_application.exe", "target/release/libmycrate.dll").ok();

    child.wait().expect("Child process error");
}
```
