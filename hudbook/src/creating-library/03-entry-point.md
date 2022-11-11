# Writing the entry point

In order for our code to be executed on [injection](/injecting-library/01-inject.md), we need an
entry point. In Windows, that entails implementing the `DllMain` method. We could
[roll our own](roll-your-own), but `hudhook` provides facilities to simplify that to a single
line of code.

The `hudhook!` macro takes in an object whose type is one of the hook structures.
The one we care about is `ImguiDX12Hooks` as we are targeting a DirectX 12 host application.

We don't have to implement this trait manually: all the objects implementing `ImguiRenderLoop` also
have an `into_hook` method which is designed for that purpose. We only need to invoke it, also
specifying which hook type we want with a generic parameter:

```rust
use hudhook::hooks::dx12::ImguiDX12Hooks;

hudhook::hudhook!(HelloHud::new().into_hook::<ImguiDX12Hooks>());
```

We are finally ready to build our library:

```
cargo build --release
```

This will generate a `target/release/hello_hud.dll`. We can [inject](inject) this library directly.

[roll-your-own]: /
[inject]: /injecting-library/01-injecting.md
