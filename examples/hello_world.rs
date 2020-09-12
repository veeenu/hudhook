use std::time::Instant;

use hudhook::memory::*;
use hudhook::*;
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

hudhook!(Box::new(HelloWorld {
  start: Instant::now(),
  counter: 1000.
}));
