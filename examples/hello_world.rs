use std::time::Instant;

use hudhook::memory::{get_base_address, PointerChain};
use hudhook::{hook, RenderLoop};
use imgui::im_str;

pub struct HelloWorld {
  start: Instant,
  counter: f64,
}

impl RenderLoop for HelloWorld {
  fn render(&mut self, ui: &mut imgui::Ui) {
    self.counter += 0.001;

    let baddr: isize = unsafe { std::mem::transmute(get_base_address::<std::ffi::c_void>()) };
    let ptr = PointerChain::<f64>::new(vec![baddr + 0x1BAF0, 0x18]);
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
