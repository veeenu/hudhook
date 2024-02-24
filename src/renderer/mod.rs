//! The [`hudhook`](crate) overlay rendering engine.
mod engine;
pub(crate) mod input;
mod keys;

pub use engine::RenderEngine;
use tracing::{debug, error};
use windows::Win32::Graphics::Direct3D12::{D3D12GetDebugInterface, ID3D12Debug};
use windows::Win32::Graphics::Dxgi::{
    DXGIGetDebugInterface1, IDXGIInfoQueue, DXGI_DEBUG_ALL, DXGI_INFO_QUEUE_MESSAGE,
};

use crate::util;

pub fn enable_debug_interface() {
    let debug_interface: Result<ID3D12Debug, _> =
        util::try_out_ptr(|v| unsafe { D3D12GetDebugInterface(v) });

    match debug_interface {
        Ok(debug_interface) => unsafe { debug_interface.EnableDebugLayer() },
        Err(e) => {
            error!("Could not create debug interface: {e:?}")
        },
    }
}

pub fn print_dxgi_debug_messages() {
    let Ok(diq): Result<IDXGIInfoQueue, _> = (unsafe { DXGIGetDebugInterface1(0) }) else {
        return;
    };

    let n = unsafe { diq.GetNumStoredMessages(DXGI_DEBUG_ALL) };
    for i in 0..n {
        let mut msg_len: usize = 0;
        unsafe { diq.GetMessage(DXGI_DEBUG_ALL, i, None, &mut msg_len as _).unwrap() };
        let diqm = vec![0u8; msg_len];
        let pdiqm = diqm.as_ptr() as *mut DXGI_INFO_QUEUE_MESSAGE;
        unsafe { diq.GetMessage(DXGI_DEBUG_ALL, i, Some(pdiqm), &mut msg_len as _).unwrap() };
        let diqm = unsafe { pdiqm.as_ref().unwrap() };
        debug!(
            "[DIQ] {}",
            String::from_utf8_lossy(unsafe {
                std::slice::from_raw_parts(diqm.pDescription, diqm.DescriptionByteLength - 1)
            })
        );
    }
    unsafe { diq.ClearStoredMessages(DXGI_DEBUG_ALL) };
}
