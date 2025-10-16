# Contributing

## Dependencies

You will need:

- Latest [Rust stable](https://rustup.rs/) with a `+nightly` toolchain installed
- The [MSVC toolchain](https://visualstudio.microsoft.com/vs/features/cplusplus/)

### Linux

On Linux, cross compilation is required due to DirectX being Windows only.
You have a couple options:
1. MSVC and cargo-xwin:
  ```
  cargo install cargo-xwin
  rustup target add x86_64-pc-windows-msvc
  rustup target add +nightly x86_64-pc-windows-msvc
  ```
2. Toolchains for cross compilation:
  ```
  rustup target add x86_64-pc-windows-gnu i686-pc-windows-gnu
  rustup target add +nightly x86_64-pc-windows-gnu i686-pc-windows-gnu
  sudo pacman -S mingw-w64-gcc   # or equivalent for your distro
  ```

## Testing on Linux

To run tests in Wine, it is necessary to install your distribution's `mingw32` package, and add
the DLL paths to `$WINEPATH`:

```
export WINEPATH=/usr/x86_64-w64-mingw32/bin
```

Your path could be different, depending on where your distribution installs the packages.
The one above is valid on Manjaro; the `mingw-w64-*` packages are the ones to install.

To run the tests:

```
cargo t
# which is an alias of:
cargo xwin test --target x86_64-pc-windows-msvc
```

```
cargo tg
# which is an alias of:
cargo test --target x86_64-pc-windows-gnu
```

TODO: Sometimes this doesn't work on dx11 and dx12 (dx9 and opengl are fine). Investigate.

## Preparing a pull request

Correct formatting and Clippy lints are enforced via CI. For both, the `+nightly` channel is
required so we can have more up-to-date rules. Please run both to ensure compliance when submitting
a pull request.

Format:

```
cargo +nightly fmt --all
```

Lints:

- On Windows:
  ```
  cargo +nightly clippy
  ```
- On Linux with cargo-xwin:
  ```
  cargo +nightly c
  # which is an alias of:
  cargo +nightly xwin clippy --target x86_64-pc-windows-msvc --all
  ```
- On Linux with the GNU toolchain:
  ```
  cargo +nightly cg
  # which is an alias of:
  cargo +nightly --target x86_64-pc-windows-gnu
  ```
