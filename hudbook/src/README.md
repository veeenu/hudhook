# Introduction

`hudhook` is a Rust library for creating in-game overlays, similar to the Steam
Overlay.

`hudhook` allows you to create injectable DLLs that seamlessly hook into the
rendering loop of applications and draw a UI on top. This way, you can control
the application's state, display relevant information, or do whatever it is you
would like to do every time a frame is displayed by the rendering engine.

At the moment, the only UI toolkit supported is `dear imgui`, but there are
plans to support `egui` in the future for a 100% Rust development experience.

`hudhook` currently supports rendering on top of DirectX 9, DirectX 11, DirectX
12, and OpenGL 3. If the application you chose to target uses one of these
engines, `hudhook` can get in there and draw stuff for you!

The way this is done is by [detouring][detouring] calls to rendering functions,
such as `IDXGISwapChain::Present`, and introduce custom logic and draw calls
before yielding the control back to the host application.

In this book, you will be guided through creating a hookable library and
injecting it into a [sample DirectX 12 application][samples].

Refer to the [crate's documentation][rustdocs] for the API.

[detouring]: https://en.wikipedia.org/wiki/Microsoft_Detours
[samples]: https://github.com/microsoft/DirectX-Graphics-Samples
[rustdocs]: https://veeenu.github.io/hudhook/rustdoc/hudhook
