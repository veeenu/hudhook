# Setting up the project

To create a library with `hudhook`, we need to setup our project as a Windows dynamically-linked
library (DLL).

First of all, let's create a new Rust project and add `hudhook` as a dependency.

```
cargo init --lib hello-hud
cd hello-hud
cargo add hudhook@0.5 imgui@0.11
```

We need to specify that our library is a DLL, so let's add that to `Cargo.toml`:

```toml
[lib]
crate-type = ["cdylib", "rlib"]
name = "hello_hud"
```

`hudhook` only supports Rust nightly, so let's create a `rust-toolchain.toml` at the top level of
our crate, and paste the following content in it:

```toml
[toolchain]
channel = "nightly"
```

This will instruct `cargo` to always use the nightly toolchain in the context of this project.

We are now ready to start writing the code.
