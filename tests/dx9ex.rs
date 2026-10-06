mod harness;
mod hook;

use std::thread;
use std::time::Duration;

use harness::dx9ex::Dx9ExHarness;
use hook::HookExample;
use hudhook::hooks::dx9::ex::ImguiDx9ExHooks;
use hudhook::*;

#[test]
fn test_imgui_dx9ex() {
    hook::setup_tracing();

    let dx9ex_harness = Dx9ExHarness::new("DX9Ex hook example");
    thread::sleep(Duration::from_millis(500));

    if let Err(e) = Hudhook::builder().with::<ImguiDx9ExHooks>(HookExample::new()).build().apply() {
        eprintln!("Couldn't apply hooks: {e:?}");
    }

    thread::sleep(Duration::from_millis(7000));
    drop(dx9ex_harness);
}
