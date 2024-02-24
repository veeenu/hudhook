use std::mem;
use std::ptr::{self, null_mut};

use windows::core::Result;
use windows::Foundation::Numerics::Matrix4x4;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Direct3D9::*;

use crate::renderer::RenderEngine;
use crate::util::{self, try_out_param};

const D3DTS_WORLDMATRIX: D3DTRANSFORMSTATETYPE = D3DTRANSFORMSTATETYPE(256);
const MAT_IDENTITY: Matrix4x4 = Matrix4x4 {
    M11: 1.0,
    M22: 1.0,
    M33: 1.0,
    M44: 1.0,
    M12: 0.0,
    M13: 0.0,
    M14: 0.0,
    M21: 0.0,
    M23: 0.0,
    M24: 0.0,
    M31: 0.0,
    M32: 0.0,
    M34: 0.0,
    M41: 0.0,
    M42: 0.0,
    M43: 0.0,
};

const D3DFVF_CUSTOMVERTEX: u32 = D3DFVF_XYZ | D3DFVF_TEX1;

#[repr(C)]
struct Vertex {
    pos: [f32; 3],
    uv: [f32; 2],
}

const VERTICES: [Vertex; 6] = [
    Vertex { pos: [-1.0, 1.0, 0.0], uv: [0.0, 0.0] },
    Vertex { pos: [1.0, 1.0, 0.0], uv: [1.0, 0.0] },
    Vertex { pos: [-1.0, -1.0, 0.0], uv: [0.0, 1.0] },
    Vertex { pos: [1.0, 1.0, 0.0], uv: [1.0, 0.0] },
    Vertex { pos: [1.0, -1.0, 0.0], uv: [1.0, 1.0] },
    Vertex { pos: [-1.0, -1.0, 0.0], uv: [0.0, 1.0] },
];

pub struct Compositor {
    device: IDirect3DDevice9,
    vertex_buffer: IDirect3DVertexBuffer9,
    texture: IDirect3DTexture9,
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

        let texture = util::try_out_ptr(|v| unsafe {
            device.CreateTexture(
                width as u32,
                height as u32,
                1,
                D3DUSAGE_DYNAMIC as _,
                D3DFMT_A8R8G8B8,
                D3DPOOL_DEFAULT,
                v,
                ptr::null_mut(),
            )
        })?;

        Ok(Self { device: device.clone(), vertex_buffer, texture })
    }

    pub fn composite(&self, engine: &RenderEngine, resource: ID3D12Resource) -> Result<()> {
        unsafe {
            self.device.BeginScene()?;
            let rect = util::try_out_param(|v| self.texture.LockRect(0, v, ptr::null_mut(), 0))?;

            engine.copy_texture(resource, rect.pBits as *mut u8)?;

            self.texture.UnlockRect(0)?;

            self.device
                .SetRenderTarget(0, &self.device.GetBackBuffer(0, 0, D3DBACKBUFFER_TYPE_MONO)?)?;
            self.device.SetPixelShader(None)?;
            self.device.SetVertexShader(None)?;
            self.device.SetTexture(0, &self.texture)?;
            self.device.SetRenderState(D3DRS_ALPHABLENDENABLE, true.into())?;
            self.device.SetRenderState(D3DRS_SRCBLEND, D3DBLEND_SRCALPHA.0)?;
            self.device.SetRenderState(D3DRS_DESTBLEND, D3DBLEND_INVSRCALPHA.0)?;
            self.device.SetRenderState(D3DRS_LIGHTING, false.into())?;
            self.device.SetSamplerState(0, D3DSAMP_MINFILTER, D3DTEXF_NONE.0 as u32)?;
            self.device.SetSamplerState(0, D3DSAMP_MAGFILTER, D3DTEXF_NONE.0 as u32)?;

            self.device.SetTransform(D3DTS_WORLDMATRIX, &MAT_IDENTITY).unwrap();
            self.device.SetTransform(D3DTS_VIEW, &MAT_IDENTITY).unwrap();
            self.device.SetTransform(D3DTS_PROJECTION, &MAT_IDENTITY).unwrap();

            self.device.SetFVF(D3DFVF_CUSTOMVERTEX)?;
            self.device.SetStreamSource(
                0,
                &self.vertex_buffer,
                0,
                mem::size_of::<Vertex>() as u32,
            )?;
            self.device.DrawPrimitive(D3DPT_TRIANGLELIST, 0, 2)?;
            self.device.EndScene()?;
        }

        Ok(())
    }
}
