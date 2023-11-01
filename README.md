# hudhook

[![book](https://img.shields.io/badge/docs-book-brightgreen)](https://veeenu.github.io/hudhook)
[![rustdoc](https://img.shields.io/badge/docs-rustdoc-brightgreen)](https://veeenu.github.io/hudhook/rustdoc/hudhook)

A render loop hook library with [Dear ImGui](https://github.com/ocornut/imgui)
overlays, largely inspired by [CheatEngine](https://cheatengine.org/).

Currently supports DirectX 9, DirectX 11, DirectX 12 and OpenGL 3.

Read the tutorial book [here](https://veeenu.github.io/hudhook).

Read the API reference [here](https://veeenu.github.io/hudhook/rustdoc/hudhook).

Read up on the underlying architecture [here](https://veeenu.github.io/blog/sekiro-practice-tool-architecture/).

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
    hudhook!(MyRenderLoop.into_hook::<ImguiDx9Hooks>());
}

{
    // Use this if hooking into a DirectX 11 application.
    use hudhook::hooks::dx11::ImguiDx11Hooks;
    hudhook!(MyRenderLoop.into_hook::<ImguiDx11Hooks>());
}

{
    // Use this if hooking into a DirectX 12 application.
    use hudhook::hooks::dx12::ImguiDx12Hooks;
    hudhook!(MyRenderLoop.into_hook::<ImguiDx12Hooks>());
}

{
    // Use this if hooking into an OpenGL 3 application.
    use hudhook::hooks::opengl3::ImguiOpenGl3Hooks;
    hudhook!(MyRenderLoop.into_hook::<ImguiOpenGl3Hooks>());
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
