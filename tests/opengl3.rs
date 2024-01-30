mod harness;
mod hook;

use std::thread;
use std::time::Duration;

use harness::opengl3::Opengl3Harness;
use hook::HookExample;
use hudhook::hooks::opengl3::ImguiOpenGl3Hooks;
use hudhook::*;

#[test]
fn test_imgui_opengl3() {
    hook::setup_tracing();

    let opengl3_harness = Opengl3Harness::new("OpenGL3 hook example");
    thread::sleep(Duration::from_millis(500));

    if let Err(e) = Hudhook::builder().with::<ImguiOpenGl3Hooks>(HookExample::new()).build().apply()
    {
        eprintln!("Couldn't apply hooks: {e:?}");
    }

    thread::sleep(Duration::from_millis(5000));
    drop(opengl3_harness);
}
