mod harness;
mod hook;

use std::thread;
use std::time::Duration;

use harness::dx12::Dx12Harness;
use hook::HookExample;
use hudhook::hooks::dx12::ImguiDx12Hooks;
use hudhook::*;

#[test]
fn test_imgui_dx12() {
    hook::setup_tracing();

    let dx12_harness = Dx12Harness::new();
    thread::sleep(Duration::from_millis(1000));

    if let Err(e) = Hudhook::builder().with::<ImguiDx12Hooks>(HookExample::new()).build().apply() {
        eprintln!("Couldn't apply hooks: {e:?}");
    }

    thread::sleep(Duration::from_millis(25000));
    drop(dx12_harness);
}
