# Injecting a library

Let's now build an application that will inject the DLL into our target process.

First of all, let's download and compile the [DirectX 12 samples](samples).
We will target the `HelloTexture` sample.

Let's add a binary target to our project's `Cargo.toml`:

```toml
[[bin]]
name = "hello_injector"
path = "src/main.rs"
```

What our injector needs to do is find the process and inject the DLL. `hudhook`
provides the facilities to do this in the `hudhook::inject` module.

The `Process` struct has two constructor methods: `by_name` and `by_title`.
The former retrieves the process' ID by its name, the one you can see in the Task Manager, and
that usually corresponds to the executable name. The latter finds the PID via matching against a
window title. We will try both methods.

Injecting the DLL by process name:

```rust
use hudhook::inject::Process;

fn main() {
    Process::by_name("D3D12HelloTexture.exe").unwrap().inject("hello_hud.dll".into()).unwrap();
}
```

Injecting the DLL by window title:

```rust
use hudhook::inject::Process;

fn main() {
    Process::by_title("D3D12 Hello Texture").unwrap().inject("hello_hud.dll".into()).unwrap();
}
```

We can now compile the whole project. First, start up `D3D12HelloTexture.exe`, then run:

```
cargo build --release
cd target/release
./hello_injector.exe
```

Our `dear imgui` window will now show up inside the application's window.

[samples]: https://github.com/microsoft/DirectX-Graphics-Samples 
