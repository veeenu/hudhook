# imgui-opengl

[![Build Status](https://travis-ci.org/michaelfairley/rust-imgui-opengl-renderer.svg?branch=master)](https://travis-ci.org/michaelfairley/rust-imgui-opengl-renderer)
[![Documentation](https://docs.rs/imgui-opengl-renderer/badge.svg)](https://docs.rs/imgui-opengl-renderer)
[![Version](https://img.shields.io/crates/v/imgui-opengl-renderer.svg)](https://crates.io/crates/imgui-opengl-renderer)

OpenGL (3+) rendering for [imgui-rs](https://github.com/Gekkio/imgui-rs)

## Integration guide

1. Construct it (passing in an OpenGL function loader from [SDL2](https://github.com/Rust-SDL2/rust-sdl2) or [glutin](https://github.com/tomaka/glutin) or somesuch).
   ```rust
   let renderer = imgui_opengl_renderer::Renderer::new(&mut imgui, |s| video.gl_get_proc_address(s) as _);
   ```
2. Call `render` to draw the UI.
   ```rust
   renderer.render(ui);
   ```

Take a look at the [example app](https://github.com/michaelfairley/rust-imgui-sdl2/blob/master/examples/demo.rs) to see it all in context.
