use std::time::Instant;

use hudhook::memory::*;
use hudhook::prelude::*;
use imgui::im_str;

pub struct HelloWorld {
  start: Instant,
  counter: f64,
}

impl RenderLoop for HelloWorld {
  fn render(&mut self, ui: &mut imgui::Ui) {
    self.counter += 0.001;

    let baddr: isize = base_address();
    let ptr = PointerChain::<f64>::new(&[baddr + 0x1BAF0, 0x18]);
    ptr.write(self.counter);

    imgui::Window::new(im_str!("Hello"))
      .size([320.0, 256.0], imgui::Condition::FirstUseEver)
      .build(ui, || {
        ui.text(im_str!("Hello world!"));
        ui.text(format!("Time elapsed: {:?}", self.start.elapsed()));
        ui.text(format!("Counter: {}", self.counter));
        ui.separator();
      });
  }
}

hook!(Box::new(HelloWorld {
  start: Instant::now(),
  counter: 1000.
}));
