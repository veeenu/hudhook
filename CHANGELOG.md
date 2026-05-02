# Changelog

All notable changes to this project will be documented in this file.

## [0.9.0] - 2026-03-31

### What's Changed

- fix: prevent panic on D3D backends when rendering empty draw data by @Cdm2883 in [#243](https://github.com/veeenu/hudhook/pull/243)
- Add `MessageFilter::InputMouseMove` by @vars1ty in [#244](https://github.com/veeenu/hudhook/pull/244)
- DX12: deferred fence signaling and per-frame command contexts by @updawg in [#245](https://github.com/veeenu/hudhook/pull/245)
- Eject fixes by @henrikwiller in [#242](https://github.com/veeenu/hudhook/pull/242)
- Split `on_wnd_proc` into `before_wnd_proc` and `after_wnd_proc` by @veeenu in [#246](https://github.com/veeenu/hudhook/pull/246)
- Bump `windows-rs` to `0.62` by @veeenu in [#247](https://github.com/veeenu/hudhook/pull/247)

### New Contributors

- @Cdm2883 made their first contribution in [#243](https://github.com/veeenu/hudhook/pull/243)
- @updawg made their first contribution in [#245](https://github.com/veeenu/hudhook/pull/245)
- @henrikwiller made their first contribution in [#242](https://github.com/veeenu/hudhook/pull/242)

**Full Changelog**: [0.8.3...0.9.0](https://github.com/veeenu/hudhook/compare/0.8.3...0.9.0)

## [0.8.3] - 2026-01-28

### What's Changed

- DX9: `CreateVertexBuffer` with the correct size by @veeenu in [#239](https://github.com/veeenu/hudhook/pull/239)

**Full Changelog**: [0.8.2...0.8.3](https://github.com/veeenu/hudhook/compare/0.8.2...0.8.3)

## [0.8.2] - 2025-10-26

### What's Changed

- Set correct version for actions/upload-pages-artifact by @veeenu in [#223](https://github.com/veeenu/hudhook/pull/223)
- Bump deploy-pages to v4 by @veeenu in [#224](https://github.com/veeenu/hudhook/pull/224)
- fix: missing size_of import by @jacobtread in [#227](https://github.com/veeenu/hudhook/pull/227)
- Improve `eject()` crashes on DX12 by @zed0 in [#231](https://github.com/veeenu/hudhook/pull/231)

### New Contributors

- @zed0 made their first contribution in [#231](https://github.com/veeenu/hudhook/pull/231)

**Full Changelog**: [0.8.1...0.8.2](https://github.com/veeenu/hudhook/compare/0.8.1...0.8.2)

## [0.8.1] - 2025-03-24

### What's Changed

- Update discord invite by @veeenu in [#215](https://github.com/veeenu/hudhook/pull/215)
- Fix D3DERR_INVALIDCALL returned from Present by @kkots in [#221](https://github.com/veeenu/hudhook/pull/221)

### New Contributors

- @kkots made their first contribution in [#221](https://github.com/veeenu/hudhook/pull/221)

**Full Changelog**: [0.8.0...0.8.1](https://github.com/veeenu/hudhook/compare/0.8.0...0.8.1)

## [0.8.0] - 2024-11-06

### What's Changed

- feat: add imgui features to feature table by @MeguminSama in [#197](https://github.com/veeenu/hudhook/pull/197)
- Use stabilized offset_of by @Jakobzs in [#202](https://github.com/veeenu/hudhook/pull/202)
- Address new lints by @veeenu in [#206](https://github.com/veeenu/hudhook/pull/206)
- Fix DirectX12 heap resize bug by @veeenu in [#200](https://github.com/veeenu/hudhook/pull/200)
- Choose command queue associated to swap chain in DirectX 12 hook by @veeenu in [#209](https://github.com/veeenu/hudhook/pull/209)

### New Contributors

- @MeguminSama made their first contribution in [#197](https://github.com/veeenu/hudhook/pull/197)
- @cryeprecision made their first contribution in [#210](https://github.com/veeenu/hudhook/pull/210)

**Full Changelog**: [0.7.1...0.8.0](https://github.com/veeenu/hudhook/compare/0.7.1...0.8.0)

## [0.7.1] - 2024-07-11

### What's Changed

- Use unicode WinApi functions everywhere to support non-ASCII inputs by @soarqin in [#193](https://github.com/veeenu/hudhook/pull/193)

### New Contributors

- @soarqin made their first contribution in [#193](https://github.com/veeenu/hudhook/pull/193)

**Full Changelog**: [0.7.0...0.7.1](https://github.com/veeenu/hudhook/compare/0.7.0...0.7.1)

## [0.7.0] - 2024-05-24

### What's Changed

- Fix the crash caused by the key press by @vSylva in [#175](https://github.com/veeenu/hudhook/pull/175)
- Align upload_pitch by @misdake in [#174](https://github.com/veeenu/hudhook/pull/174)
- Replace texture image data by @misdake in [#176](https://github.com/veeenu/hudhook/pull/176)
- Explicit transmute types by @veeenu in [#185](https://github.com/veeenu/hudhook/pull/185)
- Bugfix panic in D3D11RenderEngine->restore by @FrankvdStam in [#184](https://github.com/veeenu/hudhook/pull/184)
- Add MessageFilter to selectively block window message by @ruby3141 in [#183](https://github.com/veeenu/hudhook/pull/183)
- Make pipeline reports `delta_time` to imgui by @ruby3141 in [#187](https://github.com/veeenu/hudhook/pull/187)
- Bump imgui by @veeenu in [#189](https://github.com/veeenu/hudhook/pull/189)

### New Contributors

- @misdake made their first contribution in [#174](https://github.com/veeenu/hudhook/pull/174)
- @FrankvdStam made their first contribution in [#184](https://github.com/veeenu/hudhook/pull/184)
- @ruby3141 made their first contribution in [#183](https://github.com/veeenu/hudhook/pull/183)

**Full Changelog**: [0.6.5...0.7.0](https://github.com/veeenu/hudhook/compare/0.6.5...0.7.0)

## [0.6.5] - 2024-03-12

### What's Changed

- Reorder console allocation and enable color calls in example support by @vSylva in [#170](https://github.com/veeenu/hudhook/pull/170)
- Refactor input to use non-deprecated imgui key i/o system by @veeenu in [#171](https://github.com/veeenu/hudhook/pull/171)

**Full Changelog**: [0.6.4...0.6.5](https://github.com/veeenu/hudhook/compare/0.6.4...0.6.5)

## [0.6.4] - 2024-03-11

### What's Changed

- Add proper cleanup to Pipeline by @veeenu in [#169](https://github.com/veeenu/hudhook/pull/169)

**Full Changelog**: [0.6.3...0.6.4](https://github.com/veeenu/hudhook/compare/0.6.3...0.6.4)

## [0.6.3] - 2024-03-11

### What's Changed

- Remove platform block in build.rs by @veeenu in [#168](https://github.com/veeenu/hudhook/pull/168)

**Full Changelog**: [0.6.2...0.6.3](https://github.com/veeenu/hudhook/compare/0.6.2...0.6.3)

## [0.6.2] - 2024-03-07

### What's Changed

- Add markdown link labels for crates.io by @vSylva in [#166](https://github.com/veeenu/hudhook/pull/166)
- Avoid calls to `RoOriginateErrorW` by @veeenu in [#167](https://github.com/veeenu/hudhook/pull/167)

**Full Changelog**: [0.6.1...0.6.2](https://github.com/veeenu/hudhook/compare/0.6.1...0.6.2)

## [0.6.1] - 2024-03-02

This release applies a bugfix.

### What's Changed

- Initialize fonts texture after render loop setup by @veeenu in [#165](https://github.com/veeenu/hudhook/pull/165)

**Full Changelog**: [0.6.0...0.6.1](https://github.com/veeenu/hudhook/compare/0.6.0...0.6.1)

## [0.6.0] - 2024-03-02

This release contains big, sweeping changes as of #158. The breaking changes to the public API are very minor and shouldn't require more than a couple lines' worth of changes to adapt.

### What's Changed

- Update README.md by @veeenu in [#134](https://github.com/veeenu/hudhook/pull/134)
- Fix OpenGL3 comment in README by @Jakobzs in [#136](https://github.com/veeenu/hudhook/pull/136)
- Make UnregisterClassW not panic by @veeenu in [#135](https://github.com/veeenu/hudhook/pull/135)
- Fixed hooking comment by @jacobtread in [#140](https://github.com/veeenu/hudhook/pull/140)
- Unified renderer by @veeenu in [#143](https://github.com/veeenu/hudhook/pull/143)
- Image support by @veeenu in [#146](https://github.com/veeenu/hudhook/pull/146)
- Update documentation by @veeenu in [#148](https://github.com/veeenu/hudhook/pull/148)
- Update documentation and hudbook by @veeenu in [#152](https://github.com/veeenu/hudhook/pull/152)
- Make string usage in `inject` more idiomatic by @veeenu in [#153](https://github.com/veeenu/hudhook/pull/153)
- Revamped per-engine renderers by @veeenu in [#158](https://github.com/veeenu/hudhook/pull/158)
- Bump `windows-rs` to 0.53 by @veeenu in [#162](https://github.com/veeenu/hudhook/pull/162)
- Bump `windows-rs` to 0.54 by @veeenu in [#163](https://github.com/veeenu/hudhook/pull/163)
- Update readme by @veeenu in [#164](https://github.com/veeenu/hudhook/pull/164)

### New Contributors

- @jacobtread made their first contribution in [#140](https://github.com/veeenu/hudhook/pull/140)

**Full Changelog**: [0.5.0...0.6.0](https://github.com/veeenu/hudhook/compare/0.5.0...0.6.0)

## [0.5.0] - 2023-09-28

Not everything planned made it in this release, but I think it would be a good idea to just release breaking versions more often and plan for smaller releases.

### What's Changed

- Fix DX9 early inject crash by @Godnoken in [#90](https://github.com/veeenu/hudhook/pull/90)
- Reexport imgui by @veeenu in [#91](https://github.com/veeenu/hudhook/pull/91)
- Switch to use null driver in DX11 by @veeenu in [#92](https://github.com/veeenu/hudhook/pull/92)
- Add tracing feature 'log' by @Godnoken in [#93](https://github.com/veeenu/hudhook/pull/93)
- Fix OpenGL resizing by @Godnoken in [#95](https://github.com/veeenu/hudhook/pull/95)
- Fix #[warn(unused_mut)] by @vSylva in [#99](https://github.com/veeenu/hudhook/pull/99)
- Remove `HookableBackend` trait by @veeenu in [#100](https://github.com/veeenu/hudhook/pull/100)
- Simplify dependencies by @vSylva in [#102](https://github.com/veeenu/hudhook/pull/102)
- Simplify dependencies by @vSylva in [#105](https://github.com/veeenu/hudhook/pull/105)
- Add raw input handling by @veeenu in [#104](https://github.com/veeenu/hudhook/pull/104)
- Bump dependencies and add feature crates by @veeenu in [#109](https://github.com/veeenu/hudhook/pull/109)
- Move from GetWindowRect to GetClientRect by @veeenu in [#110](https://github.com/veeenu/hudhook/pull/110)
- Refactor DX12 renderer by @veeenu in [#112](https://github.com/veeenu/hudhook/pull/112)
- Gate renderers behind feature flags by @veeenu in [#113](https://github.com/veeenu/hudhook/pull/113)
- Add wnd proc hook to ImguiRenderLoop by @veeenu in [#114](https://github.com/veeenu/hudhook/pull/114)
- Add RAII dummy hwnd to all hooks by @veeenu in [#115](https://github.com/veeenu/hudhook/pull/115)
- Lock hooks and injection behind feature flags by @vars1ty in [#116](https://github.com/veeenu/hudhook/pull/116)
- Expose `hooks::common` by @veeenu in [#122](https://github.com/veeenu/hudhook/pull/122)
- Remove focus flag and `ImguiRenderLoopFlags` by @veeenu in [#123](https://github.com/veeenu/hudhook/pull/123)
- DirectX 9 integration tests by @veeenu in [#125](https://github.com/veeenu/hudhook/pull/125)
- Change top-level API by @veeenu in [#126](https://github.com/veeenu/hudhook/pull/126)
- Bump `windows` to 0.51 by @veeenu in [#127](https://github.com/veeenu/hudhook/pull/127)
- Add i686 check workflow by @veeenu in [#129](https://github.com/veeenu/hudhook/pull/129)

### New Contributors

- @vSylva made their first contribution in [#99](https://github.com/veeenu/hudhook/pull/99)
- @vars1ty made their first contribution in [#116](https://github.com/veeenu/hudhook/pull/116)

**Full Changelog**: [0.4.0...0.5.0](https://github.com/veeenu/hudhook/compare/0.4.0...0.5.0)

## [0.4.0] - 2023-04-21

### What's Changed

- feat: Set the special keys by @etra0 in [#64](https://github.com/veeenu/hudhook/pull/64)
- Bump imgui to 0.9.0 by @veeenu in [#66](https://github.com/veeenu/hudhook/pull/66)
- Fixed OpenGL3 by @Jakobzs in [#68](https://github.com/veeenu/hudhook/pull/68)
- Change rust action by @veeenu in [#67](https://github.com/veeenu/hudhook/pull/67)
- Added OpenGL3 integration test by @Jakobzs in [#69](https://github.com/veeenu/hudhook/pull/69)
- Reintroduced dummy HWND for DX12 by @veeenu in [#70](https://github.com/veeenu/hudhook/pull/70)
- Fix resizing crashes & potentially eject crashes by @Godnoken in [#74](https://github.com/veeenu/hudhook/pull/74)
- Remap shift/ctrl/menu virtual keys by @veeenu in [#77](https://github.com/veeenu/hudhook/pull/77)
- Add internal OpenGL loader by @Godnoken in [#75](https://github.com/veeenu/hudhook/pull/75)
- Switch once_cell feature to lazy_cell by @veeenu in [#83](https://github.com/veeenu/hudhook/pull/83)
- Clippy lints by @veeenu in [#84](https://github.com/veeenu/hudhook/pull/84)
- Added tracing by @Jakobzs in [#81](https://github.com/veeenu/hudhook/pull/81)
- DX11 feature level 10 support by @riyuzenn in [#87](https://github.com/veeenu/hudhook/pull/87)
- Bump imgui to 0.11.0 by @veeenu in [#88](https://github.com/veeenu/hudhook/pull/88)

### New Contributors

- @Godnoken made their first contribution in [#74](https://github.com/veeenu/hudhook/pull/74)
- @riyuzenn made their first contribution in [#87](https://github.com/veeenu/hudhook/pull/87)

**Full Changelog**: [0.3.0...0.4.0](https://github.com/veeenu/hudhook/compare/0.3.0...0.4.0)

## [0.3.0] - 2022-12-03

### What's Changed

- DX11 and DX12 UnregisterClass by @veeenu in [#31](https://github.com/veeenu/hudhook/pull/31)
- Unhook concurrency synchronization by @veeenu in [#37](https://github.com/veeenu/hudhook/pull/37)
- Integration tests for DX11/DX12 by @veeenu in [#40](https://github.com/veeenu/hudhook/pull/40)
- Pruned unused dependencies and fixed doctests by @veeenu in [#44](https://github.com/veeenu/hudhook/pull/44)
- Improved injection, added 32 bit support to injector by @veeenu in [#39](https://github.com/veeenu/hudhook/pull/39)
- Inject overhaul by @veeenu in [#46](https://github.com/veeenu/hudhook/pull/46)
- Clippy lints by @veeenu in [#47](https://github.com/veeenu/hudhook/pull/47)
- Hudbook by @veeenu in [#50](https://github.com/veeenu/hudhook/pull/50)
- Changed book/doc build stuff by @veeenu in [#51](https://github.com/veeenu/hudhook/pull/51)
- Rename structs by @veeenu in [#52](https://github.com/veeenu/hudhook/pull/52)
- Additions to Hudbook by @veeenu in [#53](https://github.com/veeenu/hudhook/pull/53)
- Desktop swapchain by @veeenu in [#55](https://github.com/veeenu/hudhook/pull/55)
- Fix for drawing more than 64k draw commands by @joffreybesos in [#59](https://github.com/veeenu/hudhook/pull/59)
- Back to minhook by @veeenu in [#61](https://github.com/veeenu/hudhook/pull/61)

### New Contributors

- @joffreybesos made their first contribution in [#59](https://github.com/veeenu/hudhook/pull/59)

**Full Changelog**: [0.2.0...0.3.0](https://github.com/veeenu/hudhook/compare/0.2.0...0.3.0)

## [0.2.0] - 2022-09-02

Initial tracked release.

[0.9.0]: https://github.com/veeenu/hudhook/releases/tag/0.9.0
[0.8.3]: https://github.com/veeenu/hudhook/releases/tag/0.8.3
[0.8.2]: https://github.com/veeenu/hudhook/releases/tag/0.8.2
[0.8.1]: https://github.com/veeenu/hudhook/releases/tag/0.8.1
[0.8.0]: https://github.com/veeenu/hudhook/releases/tag/0.8.0
[0.7.1]: https://github.com/veeenu/hudhook/releases/tag/0.7.1
[0.7.0]: https://github.com/veeenu/hudhook/releases/tag/0.7.0
[0.6.5]: https://github.com/veeenu/hudhook/releases/tag/0.6.5
[0.6.4]: https://github.com/veeenu/hudhook/releases/tag/0.6.4
[0.6.3]: https://github.com/veeenu/hudhook/releases/tag/0.6.3
[0.6.2]: https://github.com/veeenu/hudhook/releases/tag/0.6.2
[0.6.1]: https://github.com/veeenu/hudhook/releases/tag/0.6.1
[0.6.0]: https://github.com/veeenu/hudhook/releases/tag/0.6.0
[0.5.0]: https://github.com/veeenu/hudhook/releases/tag/0.5.0
[0.4.0]: https://github.com/veeenu/hudhook/releases/tag/0.4.0
[0.3.0]: https://github.com/veeenu/hudhook/releases/tag/0.3.0
[0.2.0]: https://github.com/veeenu/hudhook/releases/tag/0.2.0
