# hudhook

![GitHub Actions Workflow Status](https://img.shields.io/github/actions/workflow/status/veeenu/hudhook/lint.yml)
[![GitHub Release](https://img.shields.io/github/v/release/veeenu/hudhook)](https://github.com/veeenu/hudhook/releases)
[![Crates.io Version](https://img.shields.io/crates/v/hudhook)](https://crates.io/crates/hudhook)
[![GitHub License](https://img.shields.io/github/license/veeenu/hudhook)](https://github.com/veeenu/hudhook/blob/main/LICENSE)
[![Discord](https://img.shields.io/discord/267623298647457802)](https://discord.gg/CVHbN7eF)
[![book](https://img.shields.io/badge/docs-book-brightgreen)](https://veeenu.github.io/hudhook)
[![rustdoc](https://img.shields.io/badge/docs-rustdoc-brightgreen)](https://veeenu.github.io/hudhook/rustdoc/hudhook)
[![Patreon](https://img.shields.io/badge/Support_me-Patreon-orange)](https://www.patreon.com/johndisandonato)

A Rust renderer hook library for building [Dear ImGui](https://github.com/ocornut/imgui) overlays.

Currently supports DirectX 9, DirectX 11, DirectX 12 and OpenGL 3. Runs on Windows and Wine/Proton.

![hello](tests/hello.jpg)

## Resources

- [Tutorial book](https://veeenu.github.io/hudhook).
- [API reference](https://veeenu.github.io/hudhook/rustdoc/hudhook).
- [Architecture blog post](https://veeenu.github.io/blog/sekiro-practice-tool-architecture/) (a bit outdated).

## Supporting the project

If you like `hudhook` and would like to support the project, you can do so via my [Patreon](https://www.patreon.com/johndisandonato).

I'm glad the project works for you and I'm grateful for your support. Thank you!

## Example

```rust
// src/lib.rs
use hudhook::*;

pub struct MyRenderLoop;

impl ImguiRenderLoop for MyRenderLoop {
    fn render(&mut self, ui: &mut imgui::Ui) {
        ui.window("My first render loop")
            .position([0., 0.], imgui::Condition::FirstUseEver)
            .size([320., 200.], imgui::Condition::FirstUseEver)
            .build(|| {
                ui.text("Hello, hello!");
            });
    }
}

{
    // Use this if hooking into a DirectX 9 application.
    use hudhook::hooks::dx9::ImguiDx9Hooks;
    hudhook!(ImguiDx9Hooks, MyRenderLoop);
}

{
    // Use this if hooking into a DirectX 11 application.
    use hudhook::hooks::dx11::ImguiDx11Hooks;
    hudhook!(ImguiDx11Hooks, MyRenderLoop);
}

{
    // Use this if hooking into a DirectX 12 application.
    use hudhook::hooks::dx12::ImguiDx12Hooks;
    hudhook!(ImguiDx12Hooks, MyRenderLoop);
}

{
    // Use this if hooking into an OpenGL 3 application.
    use hudhook::hooks::opengl3::ImguiOpenGl3Hooks;
    hudhook!(ImguiOpenGl3Hooks, MyRenderLoop);
}
```

```rust
// src/main.rs
use hudhook::inject::Process;

fn main() {
    let mut cur_exe = std::env::current_exe().unwrap();
    cur_exe.push("..");
    cur_exe.push("libmyhook.dll");

    let cur_dll = cur_exe.canonicalize().unwrap();

    Process::by_name("MyTargetApplication.exe").unwrap().inject(cur_dll).unwrap();
}
```
