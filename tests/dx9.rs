mod harness;
mod hook;

use std::thread;
use std::time::Duration;

use harness::dx9::Dx9Harness;
use hook::HookExample;
use hudhook::hooks::dx9::ImguiDx9Hooks;
use hudhook::*;

#[test]
fn test_imgui_dx9() {
    hook::setup_tracing();

    let dx9_harness = Dx9Harness::new("DX9 hook example");
    thread::sleep(Duration::from_millis(500));

    if let Err(e) = Hudhook::builder().with::<ImguiDx9Hooks>(HookExample::new()).build().apply() {
        eprintln!("Couldn't apply hooks: {e:?}");
    }

    thread::sleep(Duration::from_millis(5000));
    drop(dx9_harness);
}
