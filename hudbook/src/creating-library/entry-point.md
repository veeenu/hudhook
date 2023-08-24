# Writing the entry point

In order for our code to be executed on [injection](/injecting-library/01-inject.md), we need an
entry point. In Windows, that entails implementing the `DllMain` method. We could
[roll our own][roll-your-own], but `hudhook` provides facilities to simplify that to a single
line of code.

The `hudhook!` macro takes in an object whose type is one of the hook structures.
The one we care about is `ImguiDx12Hooks` as we are targeting a DirectX 12 host application.

We don't have to implement this trait manually: all the objects implementing `ImguiRenderLoop` also
have an `into_hook` method which is designed for that purpose. We only need to invoke it, also
specifying which hook type we want with a generic parameter:

```rust
use hudhook::hooks::dx12::ImguiDX12Hooks;

hudhook::hudhook!(HelloHud::new().into_hook::<ImguiDx12Hooks>());
```

We are finally ready to build our library:

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
                .with(HelloHud::new().into_hook::<ImguiDx12Hooks>())
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
