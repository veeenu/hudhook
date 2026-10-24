use windows::core::{Interface, Result};
use windows::Win32::Graphics::Dxgi::{IDXGISwapChain, IDXGISwapChain2};

pub(super) fn display_size(swap_chain: &IDXGISwapChain) -> Result<(u32, u32)> {
    if let Ok(swap_chain2) = swap_chain.cast::<IDXGISwapChain2>() {
        let (mut width, mut height) = (0, 0);
        if unsafe { swap_chain2.GetSourceSize(&mut width, &mut height) }.is_ok() {
            return Ok((width, height));
        }
    }
    let desc = unsafe { swap_chain.GetDesc()? };
    Ok((desc.BufferDesc.Width, desc.BufferDesc.Height))
}
