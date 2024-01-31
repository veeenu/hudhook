# Writing the entry point

In order for our code to be executed on [injection](/injecting-library/01-inject.md), we need an
entry point. In Windows, that entails implementing the `DllMain` method. We could
[roll our own][roll-your-own], but `hudhook` provides facilities to simplify that to a single
line of code.

The `hudhook!` macro takes in the type of the hook we are targeting, and an instance of a struct
that implements the `ImguiRenderLoop` trait.
We are targeting a DirectX 12 host application, so the target hook type is `ImguiDx12Hooks`.

Our `HelloHud` struct already implements `ImguiRenderLoop`, so we can
instantiate it and use it as-is:

```rust
use hudhook::hooks::dx12::ImguiDX12Hooks;

hudhook::hudhook!(ImguiDx12Hooks, HelloHud::new());
```

We are finally ready to build our library.

```
cargo build --release
```

This will generate a `target/release/hello_hud.dll`. We can [inject][inject] this library directly.

# Roll your own DLL entry point

Some times, you may want to perform your own logic inside of the library's entry
point, or use more than one kind of hook. Instead of relying on the entry point
generation macro, you can write your own `DllMain` function and use the `Hudhook`
builder object to build your hooks pipeline:

```rust
use hudhook::tracing::*;
use hudhook::*;

#[no_mangle]
pub unsafe extern "stdcall" fn DllMain(
    hmodule: HINSTANCE,
    reason: u32,
    _: *mut std::ffi::c_void,
) {
    if reason == DLL_PROCESS_ATTACH {
        trace!("DllMain()");
        std::thread::spawn(move || {
            if let Err(e) = Hudhook::builder()
                .with::<ImguiDx12Hooks>(HelloHud::new())
                .with_hmodule(hmodule)
                .build()
                .apply()
            {
                error!("Couldn't apply hooks: {e:?}");
                eject();
            }
        });
    }
}
```

[roll-your-own]: #roll-your-own-dll-entry-point
[inject]: /injecting-library/01-injecting.md
