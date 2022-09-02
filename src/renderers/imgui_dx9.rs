// Based on https://github.com/Veykril/imgui-dx9-renderer
//
// Copyright (c) 2019 Lukas Wirth
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

#![cfg(windows)]
#![deny(missing_docs)]
//! This crate offers a DirectX 9 renderer for the [imgui-rs](https://docs.rs/imgui/) rust bindings.

use std::{mem, ptr, slice};

use imgui::internal::RawWrapper;
use imgui::{
    BackendFlags, Context, DrawCmd, DrawCmdParams, DrawData, DrawIdx, TextureId, Textures,
};
use windows::Win32::Foundation::{BOOL, HWND, RECT};
use windows::Win32::Graphics::Direct3D::{D3DMATRIX, D3DMATRIX_0};
use windows::Win32::Graphics::Direct3D9::{
    IDirect3DBaseTexture9, IDirect3DDevice9, IDirect3DIndexBuffer9, IDirect3DStateBlock9,
    IDirect3DSurface9, IDirect3DTexture9, IDirect3DVertexBuffer9, D3DBACKBUFFER_TYPE_MONO,
    D3DBLENDOP_ADD, D3DBLEND_INVSRCALPHA, D3DBLEND_SRCALPHA, D3DCULL_NONE,
    D3DDEVICE_CREATION_PARAMETERS, D3DFMT_A8R8G8B8, D3DFMT_INDEX16, D3DFMT_INDEX32, D3DLOCKED_RECT,
    D3DLOCK_DISCARD, D3DPOOL_DEFAULT, D3DPT_TRIANGLELIST, D3DRS_ALPHABLENDENABLE,
    D3DRS_ALPHATESTENABLE, D3DRS_BLENDOP, D3DRS_CULLMODE, D3DRS_DESTBLEND, D3DRS_FOGENABLE,
    D3DRS_LIGHTING, D3DRS_SCISSORTESTENABLE, D3DRS_SHADEMODE, D3DRS_SRCBLEND, D3DRS_ZENABLE,
    D3DSAMP_MAGFILTER, D3DSAMP_MINFILTER, D3DSBT_ALL, D3DSHADE_GOURAUD, D3DTEXF_LINEAR,
    D3DTOP_MODULATE, D3DTRANSFORMSTATETYPE, D3DTSS_ALPHAARG1, D3DTSS_ALPHAARG2, D3DTSS_ALPHAOP,
    D3DTSS_COLORARG1, D3DTSS_COLORARG2, D3DTSS_COLOROP, D3DTS_PROJECTION, D3DTS_VIEW,
    D3DUSAGE_DYNAMIC, D3DUSAGE_WRITEONLY, D3DVIEWPORT9,
};
use windows::Win32::Graphics::Dxgi::DXGI_ERROR_INVALID_CALL;
use windows::Win32::System::SystemServices::{
    D3DFVF_DIFFUSE, D3DFVF_TEX1, D3DFVF_XYZ, D3DTA_DIFFUSE, D3DTA_TEXTURE,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

const FONT_TEX_ID: usize = !0;
const D3DFVF_CUSTOMVERTEX: u32 = D3DFVF_XYZ | D3DFVF_DIFFUSE | D3DFVF_TEX1;

const FALSE: u32 = 0;
const TRUE: u32 = 1;

const VERTEX_BUF_ADD_CAPACITY: usize = 5000;
const INDEX_BUF_ADD_CAPACITY: usize = 10000;

const D3DTS_WORLDMATRIX: D3DTRANSFORMSTATETYPE = D3DTRANSFORMSTATETYPE(256);

/// Reexport of windows::core::Result<T>
pub type Result<T> = windows::core::Result<T>;

static MAT_IDENTITY: D3DMATRIX = D3DMATRIX {
    Anonymous: D3DMATRIX_0 {
        m: [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
    },
};

#[repr(C)]
struct CustomVertex {
    pos: [f32; 3],
    col: [u8; 4],
    uv: [f32; 2],
}

/// A DirectX 9 renderer for <https://docs.rs/imgui/>.
pub struct Renderer {
    device_creation_parameters: D3DDEVICE_CREATION_PARAMETERS,
    device: IDirect3DDevice9,
    font_tex: IDirect3DBaseTexture9,
    vertex_buffer: (IDirect3DVertexBuffer9, usize),
    index_buffer: (IDirect3DIndexBuffer9, usize),
    textures: Textures<IDirect3DBaseTexture9>,
    surface: IDirect3DSurface9,
}

impl Renderer {
    /// Creates a new renderer for the given [`IDirect3DDevice9`].
    ///
    /// # Safety
    ///
    /// `device` must be a valid [`IDirect3DDevice9`] pointer.
    ///
    /// [`IDirect3DDevice9`]: https://docs.rs/winapi/0.3/x86_64-pc-windows-msvc/winapi/shared/d3d9/struct.IDirect3DDevice9.html
    pub unsafe fn new(ctx: &mut Context, device: IDirect3DDevice9) -> Result<Self> {
        let font_tex =
            IDirect3DBaseTexture9::from(Self::create_font_texture(ctx.fonts(), &device)?);

        ctx.io_mut().backend_flags |= BackendFlags::RENDERER_HAS_VTX_OFFSET;
        ctx.set_renderer_name(String::from(concat!("imgui-dx9@", env!("CARGO_PKG_VERSION"))));

        let mut device_creation_parameters =
            D3DDEVICE_CREATION_PARAMETERS { ..core::mem::zeroed() };
        device.GetCreationParameters(&mut device_creation_parameters).unwrap();

        let surface = device.GetBackBuffer(0, 0, D3DBACKBUFFER_TYPE_MONO).unwrap();

        Ok(Renderer {
            vertex_buffer: Self::create_vertex_buffer(&device, 0)?,
            index_buffer: Self::create_index_buffer(&device, 0)?,
            device_creation_parameters,
            device,
            font_tex,
            textures: Textures::new(),
            surface,
        })
    }

    /// Gets the window width & height via the backbuffer
    pub fn get_window_rect(&self) -> Option<RECT> {
        unsafe {
            let mut rect: RECT = RECT { ..core::mem::zeroed() };
            if GetWindowRect(self.device_creation_parameters.hFocusWindow, &mut rect) != BOOL(0) {
                Some(rect)
            } else {
                None
            }
        }
    }

    /// Returns the hFocuswindow HWND from the device creation parameters
    pub fn get_hwnd(&self) -> HWND {
        self.device_creation_parameters.hFocusWindow
    }

    /// Creates a new renderer for the given [`IDirect3DDevice9`].
    ///
    /// # Safety
    ///
    /// `device` must be a valid [`IDirect3DDevice9`] pointer.
    ///
    /// [`IDirect3DDevice9`]: https://docs.rs/winapi/0.3/x86_64-pc-windows-msvc/winapi/shared/d3d9/struct.IDirect3DDevice9.html
    pub unsafe fn new_raw(im_ctx: &mut imgui::Context, device: IDirect3DDevice9) -> Result<Self> {
        Self::new(im_ctx, device)
    }

    /// The textures registry of this renderer.
    ///
    /// The texture slot at !0 is reserved for the font texture, therefore the
    /// renderer will ignore any texture inserted into said slot.
    #[inline]
    pub fn textures_mut(&mut self) -> &mut Textures<IDirect3DBaseTexture9> {
        &mut self.textures
    }

    /// The textures registry of this renderer.
    #[inline]
    pub fn textures(&self) -> &Textures<IDirect3DBaseTexture9> {
        &self.textures
    }

    /// Renders the given [`Ui`] with this renderer.
    ///
    /// Should the [`DrawData`] contain an invalid texture index the renderer
    /// will return `DXGI_ERROR_INVALID_CALL` and immediately stop rendering.
    ///
    /// [`Ui`]: https://docs.rs/imgui/*/imgui/struct.Ui.html
    pub fn render(&mut self, draw_data: &DrawData) -> Result<()> {
        if draw_data.display_size[0] < 0.0 || draw_data.display_size[1] < 0.0 {
            return Ok(());
        }
        unsafe {
            if self.vertex_buffer.1 < draw_data.total_vtx_count as usize {
                self.vertex_buffer =
                    Self::create_vertex_buffer(&self.device, draw_data.total_vtx_count as usize)?;
            }
            if self.index_buffer.1 < draw_data.total_idx_count as usize {
                self.index_buffer =
                    Self::create_index_buffer(&self.device, draw_data.total_idx_count as usize)?;
            }

            let state = StateBackup::backup(&self.device)?;

            self.set_render_state(draw_data);
            self.write_buffers(draw_data)?;
            self.render_impl(draw_data)?;

            state.restore(&self.device)?;
            Ok(())
        }
    }

    unsafe fn render_impl(&mut self, draw_data: &DrawData) -> Result<()> {
        let clip_off = draw_data.display_pos;
        let clip_scale = draw_data.framebuffer_scale;
        let mut vertex_offset = 0;
        let mut index_offset = 0;
        let mut last_tex = TextureId::from(FONT_TEX_ID);
        self.device.SetTexture(0, &self.font_tex).unwrap();
        for draw_list in draw_data.draw_lists() {
            for cmd in draw_list.commands() {
                match cmd {
                    DrawCmd::Elements {
                        count,
                        cmd_params: DrawCmdParams { clip_rect, texture_id, .. },
                    } => {
                        if texture_id != last_tex {
                            let texture = if texture_id.id() == FONT_TEX_ID {
                                &self.font_tex
                            } else {
                                self.textures.get(texture_id).ok_or(DXGI_ERROR_INVALID_CALL)?
                            };
                            self.device.SetTexture(0, texture).unwrap();
                            last_tex = texture_id;
                        }

                        let r: RECT = RECT {
                            left: ((clip_rect[0] - clip_off[0]) * clip_scale[0]) as i32,
                            top: ((clip_rect[1] - clip_off[1]) * clip_scale[1]) as i32,
                            right: ((clip_rect[2] - clip_off[0]) * clip_scale[0]) as i32,
                            bottom: ((clip_rect[3] - clip_off[1]) * clip_scale[1]) as i32,
                        };
                        self.device.SetScissorRect(&r).unwrap();
                        self.device
                            .DrawIndexedPrimitive(
                                D3DPT_TRIANGLELIST,
                                vertex_offset as i32,
                                0,
                                draw_list.vtx_buffer().len() as u32,
                                index_offset as u32,
                                count as u32 / 3,
                            )
                            .unwrap();
                        index_offset += count;
                    },
                    DrawCmd::ResetRenderState => self.set_render_state(draw_data),
                    DrawCmd::RawCallback { callback, raw_cmd } => {
                        callback(draw_list.raw(), raw_cmd)
                    },
                }
            }
            vertex_offset += draw_list.vtx_buffer().len();
        }
        Ok(())
    }

    unsafe fn set_render_state(&mut self, draw_data: &DrawData) {
        let vp = D3DVIEWPORT9 {
            X: 0,
            Y: 0,
            Width: draw_data.display_size[0] as u32,
            Height: draw_data.display_size[1] as u32,
            MinZ: 0.0,
            MaxZ: 1.0,
        };

        let device = &self.device;
        device.SetRenderTarget(0, &self.surface).unwrap();
        device.SetViewport(&vp).unwrap();
        device.SetPixelShader(None).unwrap();
        device.SetVertexShader(None).unwrap();
        device.SetRenderState(D3DRS_CULLMODE, D3DCULL_NONE.0 as u32).unwrap();
        device.SetRenderState(D3DRS_LIGHTING, FALSE).unwrap();
        device.SetRenderState(D3DRS_ZENABLE, FALSE).unwrap();
        device.SetRenderState(D3DRS_ALPHABLENDENABLE, TRUE).unwrap();
        device.SetRenderState(D3DRS_ALPHATESTENABLE, FALSE).unwrap();
        device.SetRenderState(D3DRS_BLENDOP, D3DBLENDOP_ADD.0 as u32).unwrap();
        device.SetRenderState(D3DRS_SRCBLEND, D3DBLEND_SRCALPHA.0 as u32).unwrap();
        device.SetRenderState(D3DRS_DESTBLEND, D3DBLEND_INVSRCALPHA.0 as u32).unwrap();
        device.SetRenderState(D3DRS_SCISSORTESTENABLE, TRUE).unwrap();
        device.SetRenderState(D3DRS_SHADEMODE, D3DSHADE_GOURAUD.0 as u32).unwrap();
        device.SetRenderState(D3DRS_FOGENABLE, FALSE).unwrap();
        device.SetTextureStageState(0, D3DTSS_COLOROP, D3DTOP_MODULATE.0 as u32).unwrap();
        device.SetTextureStageState(0, D3DTSS_COLORARG1, D3DTA_TEXTURE).unwrap();
        device.SetTextureStageState(0, D3DTSS_COLORARG2, D3DTA_DIFFUSE).unwrap();
        device.SetTextureStageState(0, D3DTSS_ALPHAOP, D3DTOP_MODULATE.0 as u32).unwrap();
        device.SetTextureStageState(0, D3DTSS_ALPHAARG1, D3DTA_TEXTURE).unwrap();
        device.SetTextureStageState(0, D3DTSS_ALPHAARG2, D3DTA_DIFFUSE).unwrap();
        device.SetSamplerState(0, D3DSAMP_MINFILTER, D3DTEXF_LINEAR.0 as u32).unwrap();
        device.SetSamplerState(0, D3DSAMP_MAGFILTER, D3DTEXF_LINEAR.0 as u32).unwrap();

        let l = draw_data.display_pos[0] + 0.5;
        let r = draw_data.display_pos[0] + draw_data.display_size[0] + 0.5;
        let t = draw_data.display_pos[1] + 0.5;
        let b = draw_data.display_pos[1] + draw_data.display_size[1] + 0.5;

        let mat_projection = D3DMATRIX {
            Anonymous: D3DMATRIX_0 {
                m: [
                    2.0 / (r - l),
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    2.0 / (t - b),
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.5,
                    0.0,
                    (l + r) / (l - r),
                    (t + b) / (b - t),
                    0.5,
                    1.0,
                ],
            },
        };

        device.SetTransform(D3DTS_WORLDMATRIX, &MAT_IDENTITY).unwrap();
        device.SetTransform(D3DTS_VIEW, &MAT_IDENTITY).unwrap();
        device.SetTransform(D3DTS_PROJECTION, &mat_projection).unwrap();
    }

    unsafe fn lock_buffers<'v, 'i>(
        vb: &'v mut IDirect3DVertexBuffer9,
        ib: &'i mut IDirect3DIndexBuffer9,
        vtx_count: usize,
        idx_count: usize,
    ) -> Result<(&'v mut [CustomVertex], &'i mut [DrawIdx])> {
        let mut vtx_dst: *mut CustomVertex = ptr::null_mut();
        let mut idx_dst: *mut DrawIdx = ptr::null_mut();

        vb.Lock(
            0,
            (vtx_count * mem::size_of::<CustomVertex>()) as u32,
            &mut vtx_dst as *mut _ as _,
            D3DLOCK_DISCARD as u32,
        )?;

        match ib.Lock(
            0,
            (idx_count * mem::size_of::<DrawIdx>()) as u32,
            &mut idx_dst as *mut _ as _,
            D3DLOCK_DISCARD as u32,
        ) {
            Ok(_) => Ok((
                slice::from_raw_parts_mut(vtx_dst, vtx_count),
                slice::from_raw_parts_mut(idx_dst, idx_count),
            )),
            Err(e) => {
                vb.Unlock().unwrap();
                Err(e)
            },
        }
    }

    unsafe fn write_buffers(&mut self, draw_data: &DrawData) -> Result<()> {
        let (mut vtx_dst, mut idx_dst) = Self::lock_buffers(
            &mut self.vertex_buffer.0,
            &mut self.index_buffer.0,
            draw_data.total_vtx_count as usize,
            draw_data.total_idx_count as usize,
        )?;

        for (vbuf, ibuf) in
            draw_data.draw_lists().map(|draw_list| (draw_list.vtx_buffer(), draw_list.idx_buffer()))
        {
            for (vertex, vtx_dst) in vbuf.iter().zip(vtx_dst.iter_mut()) {
                *vtx_dst = CustomVertex {
                    pos: [vertex.pos[0], vertex.pos[1], 0.0],
                    col: [vertex.col[2], vertex.col[1], vertex.col[0], vertex.col[3]],
                    uv: [vertex.uv[0], vertex.uv[1]],
                };
            }
            idx_dst[..ibuf.len()].copy_from_slice(ibuf);
            vtx_dst = &mut vtx_dst[vbuf.len()..];
            idx_dst = &mut idx_dst[ibuf.len()..];
        }
        self.vertex_buffer.0.Unlock().unwrap();
        self.index_buffer.0.Unlock().unwrap();
        self.device
            .SetStreamSource(0, &self.vertex_buffer.0, 0, mem::size_of::<CustomVertex>() as u32)
            .unwrap();
        self.device.SetIndices(&self.index_buffer.0).unwrap();
        self.device.SetFVF(D3DFVF_CUSTOMVERTEX).unwrap();
        Ok(())
    }

    unsafe fn create_vertex_buffer(
        device: &IDirect3DDevice9,
        vtx_count: usize,
    ) -> Result<(IDirect3DVertexBuffer9, usize)> {
        let len = vtx_count + VERTEX_BUF_ADD_CAPACITY;
        let mut vertex_buffer: Option<IDirect3DVertexBuffer9> = None;
        device.CreateVertexBuffer(
            (len * mem::size_of::<CustomVertex>()) as u32,
            (D3DUSAGE_DYNAMIC | D3DUSAGE_WRITEONLY) as u32,
            D3DFVF_CUSTOMVERTEX,
            D3DPOOL_DEFAULT,
            &mut vertex_buffer,
            ptr::null_mut(),
        )?;
        Ok((vertex_buffer.unwrap(), len))
    }

    unsafe fn create_index_buffer(
        device: &IDirect3DDevice9,
        idx_count: usize,
    ) -> Result<(IDirect3DIndexBuffer9, usize)> {
        let len = idx_count + INDEX_BUF_ADD_CAPACITY;
        let mut index_buffer: Option<IDirect3DIndexBuffer9> = None;

        device.CreateIndexBuffer(
            (len * mem::size_of::<DrawIdx>()) as u32,
            (D3DUSAGE_DYNAMIC | D3DUSAGE_WRITEONLY) as u32,
            if mem::size_of::<DrawIdx>() == 2 { D3DFMT_INDEX16 } else { D3DFMT_INDEX32 },
            D3DPOOL_DEFAULT,
            &mut index_buffer,
            ptr::null_mut(),
        )?;
        Ok((index_buffer.unwrap(), len))
    }

    // FIXME, imgui hands us an rgba texture while we make dx9 think it receives an
    // argb texture
    unsafe fn create_font_texture(
        mut fonts: imgui::FontAtlasRefMut<'_>,
        device: &IDirect3DDevice9,
    ) -> Result<IDirect3DTexture9> {
        let texture = fonts.build_rgba32_texture();
        let mut texture_handle: Option<IDirect3DTexture9> = None;

        device.CreateTexture(
            texture.width,
            texture.height,
            1,
            D3DUSAGE_DYNAMIC as u32,
            D3DFMT_A8R8G8B8,
            D3DPOOL_DEFAULT,
            &mut texture_handle,
            ptr::null_mut(),
        )?;

        let mut locked_rect: D3DLOCKED_RECT = D3DLOCKED_RECT { Pitch: 0, pBits: ptr::null_mut() };
        let result_texture = texture_handle.unwrap();

        result_texture.LockRect(0, &mut locked_rect, ptr::null_mut(), 0)?;

        let bits = locked_rect.pBits as *mut u8;
        let pitch = locked_rect.Pitch as usize;
        let height = texture.height as usize;
        let width = texture.width as usize;

        for y in 0..height {
            let d3d9_memory = bits.add(pitch * y);
            let pixels = texture.data.as_ptr();
            let pixels = pixels.add((width * 4) * y);
            std::ptr::copy(pixels, d3d9_memory, width * 4);
        }

        result_texture.UnlockRect(0).unwrap();
        fonts.tex_id = TextureId::from(FONT_TEX_ID);
        Ok(result_texture)
    }
}

struct StateBackup {
    state_block: IDirect3DStateBlock9,
    mat_world: D3DMATRIX,
    mat_view: D3DMATRIX,
    mat_projection: D3DMATRIX,
    viewport: D3DVIEWPORT9,
    surface: IDirect3DSurface9,
}

impl StateBackup {
    unsafe fn backup(device: &IDirect3DDevice9) -> Result<Self> {
        match device.CreateStateBlock(D3DSBT_ALL) {
            Ok(state_block) => {
                let mut mat_world: D3DMATRIX = D3DMATRIX { ..core::mem::zeroed() };
                let mut mat_view: D3DMATRIX = D3DMATRIX { ..core::mem::zeroed() };
                let mut mat_projection: D3DMATRIX = D3DMATRIX { ..core::mem::zeroed() };
                let mut viewport: D3DVIEWPORT9 = D3DVIEWPORT9 { ..core::mem::zeroed() };

                device.GetTransform(D3DTS_WORLDMATRIX, &mut mat_world)?;
                device.GetTransform(D3DTS_VIEW, &mut mat_view)?;
                device.GetTransform(D3DTS_PROJECTION, &mut mat_projection)?;
                device.GetViewport(&mut viewport)?;
                let surface = device.GetRenderTarget(0).unwrap();

                Ok(StateBackup {
                    state_block,
                    mat_world,
                    mat_view,
                    mat_projection,
                    viewport,
                    surface,
                })
            },
            Err(e) => Err(e),
        }
    }

    unsafe fn restore(&self, device: &IDirect3DDevice9) -> Result<()> {
        self.state_block.Apply().unwrap();
        device.SetTransform(D3DTS_WORLDMATRIX, &self.mat_world)?;
        device.SetTransform(D3DTS_VIEW, &self.mat_view)?;
        device.SetTransform(D3DTS_PROJECTION, &self.mat_projection)?;
        device.SetViewport(&self.viewport).unwrap();
        device.SetRenderTarget(0, &self.surface).unwrap();
        Ok(())
    }
}
