# Build a proxy DLL

Another injection technique that doesn't leverage an external injector is DLL
proxying. This works by placing a properly named DLL on the DLL search path of your
executable, so that when it invokes `LoadLibrary`, our own DLL is loaded instead.

For example, Dark Souls III loads `dinput8.dll`, and if we build a library with
that file name and copy it next to `DarkSoulsIII.exe`, it will be loaded instead of
the original `dinput8.dll` which usually sits in `C:\Windows\System32`.

The proxy DLL needs some attention as to the way it is compiled and linked.

Let's analyze the example of the [Dark Souls III no-logo mod][ds3-nologo] that is
bundled with the practice tool.

Similarly to other `hudhook` libraries, we need to specify the type of library
we are building in `Cargo.toml`:

```toml
[lib]
name = "dinput8"
crate-type = ["cdylib"]
```

We then need to specify the functions that will be exported from our library.
Create a file named `exports.def` with the following contents:

```
EXPORTS
  DirectInput8Create
```

...and make this file visible to the linker via a `build.rs` build script:

```rust
fn main() {
    println!("cargo:rustc-cdylib-link-arg=/DEF:exports.def");
}
```

Our library will define its own `DirectInput8Create` export function, as
specified above. In order to be invisible to the host application, we also
need to make sure that when that function is invoked, we also invoke the
original one.

```rust
// We define our exported function's signature as a type.
type FDirectInput8Create = unsafe extern "stdcall" fn(
    hinst: HINSTANCE,
    dwversion: u32,
    riidltf: *const GUID,
    ppvout: *mut *mut c_void,
    punkouter: HINSTANCE,
) -> HRESULT;

// We create a structure to hold a pointer to the original function.
struct State {
    directinput8create: FDirectInput8Create,
}

// These impls are safe because the pointer to the function will be constant
// across the entire execution.
unsafe impl Send for State {}
unsafe impl Sync for State {}

// We lazily initialize and statically store our `State` structure. The first
// time this is invoked, it will load the actual `dinput8.dll` and get the
// pointer to the `DirectInput8Create` function inside of it.
static STATE: LazyLock<State> = LazyLock::new(|| unsafe {
    let dinput8 = LoadLibraryA(PCSTR(b"C:\\Windows\\System32\\dinput8.dll\0".as_ptr())).unwrap();
    let directinput8create =
        std::mem::transmute(GetProcAddress(dinput8, PCSTR(b"DirectInput8Create\0".as_ptr())));
    println!("Called!");

    State { directinput8create }
});
```

We then need to define our exported function, paying attention that the calling
convention is appropriate and making sure to invoke the original function with
the same parameters, and return its return value.


```rust
#[no_mangle]
unsafe extern "stdcall" fn DirectInput8Create(
    hinst: HINSTANCE,
    dwversion: u32,
    riidltf: *const GUID,
    ppvout: *mut *mut c_void,
    punkouter: HINSTANCE,
) -> HRESULT {
    patch();  // Perform our custom logic, like setup hudhook or whatever.

    (STATE.directinput8create)(hinst, dwversion, riidltf, ppvout, punkouter)
}
```

Finally, define our entry point and make it so that the lazy lock we defined
above is evaluated.

```rust
#[no_mangle]
#[allow(non_snake_case, unused_variables)]
extern "system" fn DllMain(dll_module: HINSTANCE, call_reason: u32, reserved: *mut c_void) -> BOOL {
    match call_reason {
        DLL_PROCESS_ATTACH => LazyLock::force(&STATE),
        DLL_PROCESS_DETACH => (),
        _ => (),
    }

    BOOL::from(true)
}
```

We can now compile our DLL:

```
cargo build --release
```

The result in `target/release/dinput8.dll` can be copied and pasted as-is and is
going to execute whatever code is in the `patch()` function in the context of the
process once the DLL gets automatically loaded at application startup.

[ds3-nologo]: https://github.com/veeenu/darksoulsiii-practice-tool/tree/master/lib/no-logo
