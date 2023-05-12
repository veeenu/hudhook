use windows::Win32::Foundation::{BOOL, HWND, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDeviceAndSwapChain, ID3D11Buffer, ID3D11Device, ID3D11DeviceContext,
    ID3D11RenderTargetView, ID3D11Resource, ID3D11ShaderResourceView, D3D11_CREATE_DEVICE_DEBUG,
    D3D11_CREATE_DEVICE_FLAG, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD, D3D11_SDK_VERSION,
    D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC, DXGI_MODE_SCALING, DXGI_MODE_SCANLINE_ORDER,
    DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGISwapChain, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

const DEVICE_FLAGS: D3D11_CREATE_DEVICE_FLAG = D3D11_CREATE_DEVICE_DEBUG;

pub(crate) struct DeviceAndSwapChain {
    dev: ID3D11Device,
    dev_ctx: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain,
    back_buffer: ID3D11RenderTargetView,
}

impl DeviceAndSwapChain {
    pub(crate) fn new(hwnd: HWND) -> Self {
        let mut swap_chain: Option<IDXGISwapChain> = None;
        let mut dev: Option<ID3D11Device> = None;
        let mut dev_ctx: Option<ID3D11DeviceContext> = None;

        unsafe {
            D3D11CreateDeviceAndSwapChain(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                None,
                DEVICE_FLAGS,
                None,
                D3D11_SDK_VERSION,
                Some(&DXGI_SWAP_CHAIN_DESC {
                    BufferCount: 1,
                    BufferDesc: DXGI_MODE_DESC {
                        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                        Width: 0,
                        Height: 0,
                        RefreshRate: DXGI_RATIONAL { Numerator: 0, Denominator: 0 },
                        ScanlineOrdering: DXGI_MODE_SCANLINE_ORDER(0),
                        Scaling: DXGI_MODE_SCALING(0),
                    },
                    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                    OutputWindow: hwnd,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 4, Quality: 0 },
                    Windowed: BOOL(1),
                    SwapEffect: DXGI_SWAP_EFFECT(0),
                    Flags: 0,
                } as *const _),
                Some(&mut swap_chain as *mut _),
                Some(&mut dev as *mut _),
                None,
                Some(&mut dev_ctx as *mut _),
            )
            .unwrap()
        };

        DeviceAndSwapChain::new_with_ptrs(
            dev.expect("Null device"),
            dev_ctx.expect("Null device context"),
            swap_chain.expect("Null swap chain"),
        )
    }

    pub(crate) fn new_with_ptrs(
        dev: ID3D11Device,
        dev_ctx: ID3D11DeviceContext,
        swap_chain: IDXGISwapChain,
    ) -> Self {
        let back_buffer = unsafe {
            let p_back_buffer: ID3D11Resource = swap_chain.GetBuffer(0).expect("GetBuffer");
            let mut back_buffer = None;

            dev.CreateRenderTargetView(&p_back_buffer, None, Some(&mut back_buffer))
                .expect("CreateRenderTargetView");

            dev_ctx.OMSetRenderTargets(Some(&[Some(back_buffer.unwrap().clone())]), None);

            back_buffer
        };

        unsafe {
            dev_ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.,
                TopLeftY: 0.,
                Width: 640.,
                Height: 480.,
                MinDepth: 0.,
                MaxDepth: 1.,
            }]))
        };

        DeviceAndSwapChain { dev, dev_ctx, swap_chain, back_buffer: back_buffer.unwrap() }
    }

    pub(crate) fn setup_state(&self, draw_data: &imgui::DrawData) {
        let [_x, _y] = draw_data.display_pos;
        let [_w, _h] = draw_data.display_size;

        self.set_render_target();
    }

    pub(crate) fn set_shader_resources(&self, srv: ID3D11ShaderResourceView) {
        unsafe { self.dev_ctx.PSSetShaderResources(0, Some(&[Some(srv)])) }
    }

    pub(crate) fn set_viewport(&self, rect: RECT) {
        unsafe {
            self.dev_ctx().RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.,
                TopLeftY: 0.,
                Width: (rect.right - rect.left) as f32,
                Height: (rect.bottom - rect.top) as f32,
                MinDepth: 0.,
                MaxDepth: 1.,
            }]))
        };
    }

    pub(crate) fn set_render_target(&self) {
        unsafe {
            self.dev_ctx.OMSetRenderTargets(Some(&[Some(self.back_buffer.clone())]), None);
        }
    }

    pub(crate) fn get_window_rect(&self) -> Option<RECT> {
        let mut sd = DXGI_SWAP_CHAIN_DESC::default();
        unsafe {
            self.swap_chain.GetDesc(&mut sd).expect("GetDesc");
            let mut rect: RECT = Default::default();
            if GetWindowRect(sd.OutputWindow, &mut rect as _) != BOOL(0) {
                Some(rect)
            } else {
                None
            }
        }
    }

    pub(crate) fn with_mapped<F>(&self, ptr: &ID3D11Buffer, f: F)
    where
        F: FnOnce(&D3D11_MAPPED_SUBRESOURCE),
    {
        let mut ms = D3D11_MAPPED_SUBRESOURCE::default();

        unsafe {
            self.dev_ctx.Map(ptr, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut ms)).expect("Map");

            f(&ms);

            self.dev_ctx.Unmap(ptr, 0);
        }
    }

    pub(crate) fn dev(&self) -> ID3D11Device {
        self.dev.clone()
    }

    pub(crate) fn dev_ctx(&self) -> ID3D11DeviceContext {
        self.dev_ctx.clone()
    }

    pub(crate) fn swap_chain(&self) -> IDXGISwapChain {
        self.swap_chain.clone()
    }
}
