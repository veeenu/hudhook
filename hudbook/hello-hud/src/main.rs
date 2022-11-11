use hudhook::inject::Process;

fn main() {
    // Process::by_name("D3D12HelloTexture.exe").unwrap().inject("hello_hud.dll".into()).unwrap();
    Process::by_title("D3D12 Hello Texture").unwrap().inject("hello_hud.dll".into()).unwrap();
}
