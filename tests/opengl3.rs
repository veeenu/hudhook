mod harness;
mod hook;

use std::thread;
use std::time::Duration;

use harness::opengl3::Opengl3Harness;
use hook::HookExample;
use hudhook::hooks::opengl3::ImguiOpenGl3Hooks;
use hudhook::hooks::{Hooks, ImguiRenderLoop};
use tracing::metadata::LevelFilter;

#[test]
fn test_imgui_opengl3() {
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::TRACE)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .with_thread_names(true)
        .init();

    let opengl3_harness = Opengl3Harness::new("OpenGL3 hook example");
    thread::sleep(Duration::from_millis(500));

    unsafe {
        let hooks: Box<dyn Hooks> = { HookExample::new().into_hook::<ImguiOpenGl3Hooks>() };
        hooks.hook();
        hudhook::lifecycle::global_state::set_hooks(hooks);
    }

    thread::sleep(Duration::from_millis(5000));
    drop(opengl3_harness);
}
