use std::mem;
use std::ptr::{self, null_mut};
use std::sync::OnceLock;

use tracing::trace;
use windows::core::{w, ComInterface, Result};
use windows::Win32::Foundation::{GENERIC_ALL, HWND};
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_1;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Resource, ID3D11Texture2D};
use windows::Win32::Graphics::Direct3D11on12::{D3D11On12CreateDevice, ID3D11On12Device};
use windows::Win32::Graphics::Direct3D12::{
    ID3D12Resource, D3D12_CLEAR_VALUE, D3D12_CLEAR_VALUE_0, D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
    D3D12_HEAP_FLAG_NONE, D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_READBACK,
    D3D12_MEMORY_POOL_UNKNOWN, D3D12_RESOURCE_DESC, D3D12_RESOURCE_DIMENSION_TEXTURE2D,
    D3D12_RESOURCE_FLAG_NONE, D3D12_RESOURCE_STATE_COPY_DEST, D3D12_TEXTURE_LAYOUT_UNKNOWN,
};
use windows::Win32::Graphics::Direct3D9::{
    Direct3DCreate9Ex, IDirect3DDevice9, IDirect3DTexture9, IDirect3DVertexBuffer9,
    D3DBLEND_INVSRCALPHA, D3DBLEND_SRCALPHA, D3DFMT_A8B8G8R8, D3DFVF_DIFFUSE, D3DFVF_TEX1,
    D3DFVF_XYZRHW, D3DPOOL_DEFAULT, D3DPT_TRIANGLESTRIP, D3DRS_ALPHABLENDENABLE, D3DRS_DESTBLEND,
    D3DRS_SRCBLEND, D3DUSAGE_RENDERTARGET, D3DVIEWPORT9, D3D_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIResource;

use crate::renderer::RenderedSurface;
use crate::util::{self, try_out_param};

#[repr(C)]
struct Vertex {
    x: f32,
    y: f32,
    z: f32,
    rhw: f32,
    tu: f32,
    tv: f32,
}

const D3DFVF_CUSTOMVERTEX: u32 = D3DFVF_XYZRHW | D3DFVF_TEX1;

const VERTICES: [Vertex; 4] = [
    Vertex { x: 0.0, y: -1.0, z: 0.5, rhw: 1.0, tu: 0.0, tv: 1.0 },
    Vertex { x: 1.0, y: -1.0, z: 0.5, rhw: 1.0, tu: 1.0, tv: 1.0 },
    Vertex { x: -1.0, y: 1.0, z: 0.5, rhw: 1.0, tu: 0.0, tv: 0.0 },
    Vertex { x: 1.0, y: 1.0, z: 0.5, rhw: 1.0, tu: 1.0, tv: 0.0 },
];

pub struct Compositor {
    device: IDirect3DDevice9,
    target_hwnd: HWND,
    vertex_buffer: IDirect3DVertexBuffer9,
    viewport: D3DVIEWPORT9,
    width: i32,
    height: i32,
}

impl Compositor {
    pub fn new(device: &IDirect3DDevice9, target_hwnd: HWND) -> Result<Self> {
        let (width, height) = util::win_size(target_hwnd);

        let vertex_buffer = try_out_param(|v| unsafe {
            device.CreateVertexBuffer(
                mem::size_of_val(&VERTICES) as u32,
                0,
                D3DFVF_CUSTOMVERTEX,
                D3DPOOL_DEFAULT,
                v,
                null_mut(),
            )
        })?;
        let vertex_buffer = vertex_buffer.unwrap();

        unsafe {
            let mut p_vertices: *mut u8 = null_mut();
            vertex_buffer.Lock(0, 0, &mut p_vertices as *mut *mut u8 as _, 0)?;
            ptr::copy_nonoverlapping(
                VERTICES.as_ptr() as *const u8,
                p_vertices,
                mem::size_of_val(&VERTICES),
            );
            vertex_buffer.Unlock()?;
        }

        let viewport = D3DVIEWPORT9 {
            X: 0,
            Y: 0,
            Width: width as _,
            Height: height as _,
            MinZ: 0.0,
            MaxZ: 1.0,
        };

        Ok(Self { device: device.clone(), vertex_buffer, target_hwnd, width, height, viewport })
    }

    pub fn composite(&mut self, surface: RenderedSurface) -> Result<()> {
        unsafe {
            // self.device.SetTexture(0, &texture)?;
            self.device.SetViewport(&self.viewport)?;
            self.device.SetRenderState(D3DRS_ALPHABLENDENABLE, true.into())?;
            self.device.SetRenderState(D3DRS_SRCBLEND, D3DBLEND_SRCALPHA.0)?;
            self.device.SetRenderState(D3DRS_DESTBLEND, D3DBLEND_INVSRCALPHA.0)?;
            self.device.SetFVF(D3DFVF_CUSTOMVERTEX)?;
            self.device.SetStreamSource(
                0,
                &self.vertex_buffer,
                0,
                mem::size_of::<Vertex>() as u32,
            )?;
            self.device.DrawPrimitive(D3DPT_TRIANGLESTRIP, 0, 2)?;
        }

        Ok(())
    }
}
