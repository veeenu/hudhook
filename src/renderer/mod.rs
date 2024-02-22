//! The [`hudhook`](crate) overlay rendering engine.
mod engine;
pub(crate) mod input;
mod keys;

pub use engine::RenderEngine;
use tracing::debug;
use windows::Win32::Graphics::Dxgi::{
    DXGIGetDebugInterface1, IDXGIInfoQueue, DXGI_DEBUG_ALL, DXGI_INFO_QUEUE_MESSAGE,
};

pub(crate) unsafe fn print_dxgi_debug_messages() {
    let diq: IDXGIInfoQueue = DXGIGetDebugInterface1(0).unwrap();

    for i in 0..diq.GetNumStoredMessages(DXGI_DEBUG_ALL) {
        let mut msg_len: usize = 0;
        diq.GetMessage(DXGI_DEBUG_ALL, i, None, &mut msg_len as _).unwrap();
        let diqm = vec![0u8; msg_len];
        let pdiqm = diqm.as_ptr() as *mut DXGI_INFO_QUEUE_MESSAGE;
        diq.GetMessage(DXGI_DEBUG_ALL, i, Some(pdiqm), &mut msg_len as _).unwrap();
        let diqm = pdiqm.as_ref().unwrap();
        debug!(
            "[DIQ] {}",
            String::from_utf8_lossy(std::slice::from_raw_parts(
                diqm.pDescription,
                diqm.DescriptionByteLength - 1
            ))
        );
    }
    diq.ClearStoredMessages(DXGI_DEBUG_ALL);
}
