use crate::common::check_hresult;

use std::ptr::{null_mut, NonNull};

use winapi::shared::dxgi::*;
use winapi::shared::dxgiformat::*;
use winapi::shared::dxgitype::{
    DXGI_MODE_DESC, DXGI_RATIONAL, DXGI_SAMPLE_DESC, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use winapi::shared::windef::*;
use winapi::um::d3d11::*;
use winapi::um::d3dcommon::D3D_DRIVER_TYPE_HARDWARE;
use winapi::um::winuser::GetWindowRect;
use winapi::Interface;

const DEVICE_FLAGS: u32 = D3D11_CREATE_DEVICE_DEBUG;

pub(crate) struct DeviceAndSwapChain {
    dev: NonNull<ID3D11Device>,
    dev_ctx: NonNull<ID3D11DeviceContext>,
    swap_chain: NonNull<IDXGISwapChain>,
    back_buffer: NonNull<ID3D11RenderTargetView>,
}

impl DeviceAndSwapChain {
    pub(crate) fn new(hwnd: HWND) -> Self {
        let mut swap_chain: *mut IDXGISwapChain = null_mut();
        let mut dev: *mut ID3D11Device = null_mut();
        let mut dev_ctx: *mut ID3D11DeviceContext = null_mut();

        unsafe {
            check_hresult(D3D11CreateDeviceAndSwapChain(
                null_mut(),
                D3D_DRIVER_TYPE_HARDWARE,
                null_mut(),
                DEVICE_FLAGS,
                null_mut(),
                0,
                D3D11_SDK_VERSION,
                &DXGI_SWAP_CHAIN_DESC {
                    BufferCount: 1,
                    BufferDesc: DXGI_MODE_DESC {
                        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                        Width: 0,
                        Height: 0,
                        RefreshRate: DXGI_RATIONAL {
                            Numerator: 0,
                            Denominator: 0,
                        },
                        ScanlineOrdering: 0,
                        Scaling: 0,
                    },
                    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                    OutputWindow: hwnd,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 4,
                        Quality: 0,
                    },
                    Windowed: 1,
                    SwapEffect: 0,
                    Flags: 0,
                } as *const _,
                &mut swap_chain as *mut *mut _,
                &mut dev as *mut *mut _,
                null_mut(),
                &mut dev_ctx as *mut *mut _,
            ))
        };

        DeviceAndSwapChain::new_with_ptrs(dev, dev_ctx, swap_chain)
    }

    pub(crate) fn new_with_ptrs(
        dev: *mut ID3D11Device,
        dev_ctx: *mut ID3D11DeviceContext,
        swap_chain: *mut IDXGISwapChain,
    ) -> Self {
        let dev = NonNull::new(dev).expect("Null device");
        let dev_ctx = NonNull::new(dev_ctx).expect("Null device context");
        let swap_chain = NonNull::new(swap_chain).expect("Null swap chain");

        let back_buffer = NonNull::new(unsafe {
            let mut p_back_buffer = null_mut();
            let mut back_buffer = null_mut();

            check_hresult(swap_chain.as_ref().GetBuffer(
                0,
                &ID3D11Texture2D::uuidof(),
                &mut p_back_buffer as *mut _ as _,
            ));

            check_hresult(dev.as_ref().CreateRenderTargetView(
                p_back_buffer,
                null_mut(),
                &mut back_buffer as *mut _ as _,
            ));

            dev_ctx
                .as_ref()
                .OMSetRenderTargets(1, &back_buffer, null_mut());

            back_buffer
        })
        .expect("Null back buffer");

        unsafe {
            dev_ctx.as_ref().RSSetViewports(
                1,
                &D3D11_VIEWPORT {
                    TopLeftX: 0.,
                    TopLeftY: 0.,
                    Width: 640.,
                    Height: 480.,
                    MinDepth: 0.,
                    MaxDepth: 1.,
                },
            )
        };

        DeviceAndSwapChain {
            dev,
            dev_ctx,
            swap_chain,
            back_buffer,
        }
    }

    pub(crate) fn setup_state(&self, draw_data: &imgui::DrawData) {
        let [_x, _y] = draw_data.display_pos;
        let [_w, _h] = draw_data.display_size;

        self.set_render_target();
    }

    pub(crate) fn set_shader_resources(&self, srv: *mut ID3D11ShaderResourceView) {
        unsafe { self.dev_ctx().PSSetShaderResources(0, 1, &srv as *const _) }
    }

    pub(crate) fn set_viewport(&self, rect: RECT) {
        unsafe {
            self.dev_ctx().RSSetViewports(
                1,
                &D3D11_VIEWPORT {
                    TopLeftX: 0.,
                    TopLeftY: 0.,
                    Width: (rect.right - rect.left) as f32,
                    Height: (rect.bottom - rect.top) as f32,
                    MinDepth: 0.,
                    MaxDepth: 1.,
                },
            )
        };
    }

    pub(crate) fn set_render_target(&self) {
        unsafe {
            self.dev_ctx().OMSetRenderTargets(
                1,
                &mut self.back_buffer.as_ptr() as *mut _,
                null_mut(),
            );
        }
    }

    pub(crate) fn get_window_rect(&self) -> Option<RECT> {
        let mut sd: DXGI_SWAP_CHAIN_DESC = unsafe { std::mem::zeroed() };
        let mut rect: RECT = unsafe { std::mem::zeroed() };

        unsafe {
            check_hresult(self.swap_chain().GetDesc(&mut sd as _));
            if GetWindowRect(sd.OutputWindow, &mut rect as _) != 0 {
                Some(rect)
            } else {
                None
            }
        }
    }

    pub(crate) fn dev(&self) -> &ID3D11Device {
        unsafe { self.dev.as_ref() }
    }

    pub(crate) fn dev_ctx(&self) -> &ID3D11DeviceContext {
        unsafe { self.dev_ctx.as_ref() }
    }

    pub(crate) fn swap_chain(&self) -> &IDXGISwapChain {
        unsafe { self.swap_chain.as_ref() }
    }

    pub(crate) fn with_mapped<T, F>(&self, ptr: NonNull<T>, f: F)
    where
        F: FnOnce(&D3D11_MAPPED_SUBRESOURCE),
    {
        unsafe {
            let mut ms: D3D11_MAPPED_SUBRESOURCE = std::mem::zeroed();
            check_hresult(self.dev_ctx.as_ref().Map(
                ptr.as_ptr() as _,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                &mut ms as *mut _,
            ));

            f(&ms);

            self.dev_ctx.as_ref().Unmap(ptr.as_ptr() as _, 0);
        }
    }
}

impl Drop for DeviceAndSwapChain {
    fn drop(&mut self) {
        unsafe {
            self.back_buffer.as_ref().Release();
            self.swap_chain.as_ref().Release();
            self.dev_ctx.as_ref().Release();
            self.dev.as_ref().Release();
        }
    }
}
