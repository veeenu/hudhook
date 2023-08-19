# Writing the DLL code

We will build a DLL which tracks how much time has passed since it has been injected, and displays
that in a simple `dear imgui` window.

We need a structure to store our state data. In our case, we only need to keep track of the moment
the DLL was injected.

```rust
use std::time::Instant;

struct HelloHud {
    start_time: Instant,
}

impl HelloHud {
    fn new() -> Self {
        Self { start_time: Instant::now() }
    }
}
```

We then need to supply `hudhook` with the rendering code it is supposed to run at every frame.
To do that, we import the `ImguiRenderLoop` trait, and implement that on our structure.

The trait consists of only one method, `render`. `hudhook` will supply the `imgui::Ui` object we
need to use to render our UI, we are only tasked with actually implementing our rendering code.

```rust
use hudhook::hooks::ImguiRenderLoop;
use imgui::*;


impl ImguiRenderLoop for HelloHud {
    fn render(&mut self, ui: &mut Ui) {
        ui.window("##hello")
            .size([320., 200.], Condition::Always)
            .build(|| {
                ui.text("Hello, world!");
                ui.text(format!("Elapsed: {:?}", self.start_time.elapsed()));
            });
    }
}
```

That's it! Inside of the `render` method, we can deploy whatever logic and UI rendering we want.
