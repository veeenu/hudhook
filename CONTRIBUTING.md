# Contributing

I want to turn this into a suitable guide for new contributors, but we're not there yet. Sorry :(

## Testing on Linux

On Linux, the default cross compilation target is `x86_64-pc-windows-gnu`. Cross compilation
is necessary due to DirectX being Windows only. It is recommended to also install the 32-bit
target.

```
rustup target add x86_64-pc-windows-gnu
rustup target add i686-pc-windows-gnu
```

To run tests in Wine, it is necessary to install your distribution's mingw32 package, and add
the DLL paths to `$WINEPATH`:

```
export WINEPATH=/usr/x86_64-w64-mingw32/bin
```

Your path could be different, depending on where your distribution installs the packages.
The one above is valid on Manjaro; the `mingw-w64-*` packages are the ones to install.
