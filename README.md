# hudhook

A DirectX 11 and 12 render loop hook library with memory manipulation API and
[Dear ImGui](https://github.com/ocornut/imgui) overlays, largely inspired by
[CheatEngine](https://cheatengine.org/).

Read up on the underlying architecture [here](https://veeenu.github.io/blog/sekiro-practice-tool-architecture/).

## Example

```rust
// src/lib.rs
use std::time::Instant;

use hudhook::*;
use hudhook::memory::*;
use hudhook::hooks::dx11;
use imgui::im_str;

pub struct HelloWorld {
    start: Instant,
    counter: f64,
}

impl RenderLoop for HelloWorld {
    fn render(&mut self, ctx: hudhook::RenderContext) {
        self.counter += 0.001;

        let baddr: usize = base_address();
        let ptr = PointerChain::<f64>::new(&[baddr + 0x1BAF0, 0x18]);
        ptr.write(self.counter);

        imgui::Window::new(im_str!("Hello"))
            .size([320.0, 256.0], imgui::Condition::FirstUseEver)
            .build(ctx.frame, || {
                ctx.frame.text(im_str!("Hello world!"));
                ctx
                    .frame
                    .text(format!("Time elapsed: {:?}", self.start.elapsed()));
                ctx.frame.text(format!("Counter: {}", self.counter));
                ctx.frame.separator();
            });
    }
    fn is_visible(&self) -> bool {
        true
    }
    fn is_capturing(&self) -> bool {
        true
    }
}

hudhook!(|| [
    dx11::hook_imgui(HelloWorld {
        start: Instant::now(),
        counter: 1000.
    }),
]);
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
