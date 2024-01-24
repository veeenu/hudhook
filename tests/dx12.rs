mod harness;
mod hook;

use std::thread;
use std::time::Duration;

use harness::dx12::Dx12Harness;
use hook::HookExample;
use hudhook::hooks::dx12::ImguiDx12Hooks;
use hudhook::*;
use tracing::metadata::LevelFilter;

#[test]
fn test_imgui_dx12() {
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::TRACE)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .with_thread_names(true)
        .init();

    let dx12_harness = Dx12Harness::new("DX12 hook example");
    thread::sleep(Duration::from_millis(500));

    if let Err(e) = Hudhook::builder().with::<ImguiDx12Hooks>(HookExample::new()).build().apply() {
        eprintln!("Couldn't apply hooks: {e:?}");
    }

    thread::sleep(Duration::from_millis(5000));
    drop(dx12_harness);
}
