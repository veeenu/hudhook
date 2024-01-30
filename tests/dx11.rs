mod harness;
mod hook;

use std::thread;
use std::time::Duration;

use harness::dx11::Dx11Harness;
use hook::HookExample;
use hudhook::hooks::dx11::ImguiDx11Hooks;
use hudhook::*;

#[test]
fn test_imgui_dx11() {
    hook::setup_tracing();

    let dx11_harness = Dx11Harness::new("DX11 hook example");
    thread::sleep(Duration::from_millis(500));

    if let Err(e) = Hudhook::builder().with::<ImguiDx11Hooks>(HookExample::new()).build().apply() {
        eprintln!("Couldn't apply hooks: {e:?}");
    }

    thread::sleep(Duration::from_millis(25000));
    drop(dx11_harness);
}
