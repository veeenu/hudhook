# Roll your own DLL entry point

Some times, you may want to perform your own logic inside of the library's entry
point. To do that, you can manually expand the output of the `hudhook::hudhook`
macro, and thus write your own `DllMain` function. It should look similar to this:


```rust
use hudhook::log::*;
use hudhook::reexports::*;
use hudhook::*;

#[no_mangle]
pub unsafe extern "stdcall" fn DllMain(
    hmodule: HINSTANCE,
    reason: u32,
    _: *mut std::ffi::c_void,
) {
    // You can add your own logic anywhere in here, provided
    // hudhook is initialized properly as follows.

    if reason == DLL_PROCESS_ATTACH {
        hudhook::lifecycle::global_state::set_module(hmodule);

        std::thread::spawn(move || {
            let hooks: Box<dyn hooks::Hooks> = 
                HelloHud::new().into_hook::<ImguiDx12Hooks>();

            hooks.hook();
            hudhook::lifecycle::global_state::set_hooks(hooks);
        });
    }
}
```
