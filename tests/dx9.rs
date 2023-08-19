mod harness;
mod hook;

use std::thread;
use std::time::Duration;

use harness::dx9::Dx9Harness;
use hook::HookExample;
use hudhook::hooks::dx9::ImguiDx9Hooks;
use hudhook::hooks::ImguiRenderLoop;
use hudhook::Hudhook;
use tracing::metadata::LevelFilter;

#[test]
fn test_imgui_dx9() {
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::TRACE)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .with_thread_names(true)
        .init();

    let dx9_harness = Dx9Harness::new("DX9 hook example");
    thread::sleep(Duration::from_millis(500));

    if let Err(e) =
        Hudhook::builder().with(HookExample::new().into_hook::<ImguiDx9Hooks>()).build().apply()
    {
        eprintln!("Couldn't apply hooks: {e:?}");
    }

    thread::sleep(Duration::from_millis(5000));
    drop(dx9_harness);
}
