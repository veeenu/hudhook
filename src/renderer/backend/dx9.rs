// Based on https://github.com/Veykril/imgui-dx9-renderer

use std::{mem, ptr};

use imgui::internal::RawWrapper;
use imgui::{BackendFlags, Context, DrawCmd, DrawData, DrawIdx, DrawVert, TextureId};
use tracing::error;
use windows::core::{Error, Result, HRESULT};
use windows::Foundation::Numerics::Matrix4x4;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D9::*;

use crate::renderer::RenderEngine;
use crate::{util, RenderContext};

const D3DFVF_CUSTOMVERTEX: u32 = D3DFVF_XYZ | D3DFVF_DIFFUSE | D3DFVF_TEX1;
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

#[repr(C)]
struct CustomVertex {
    pos: [f32; 3],
    col: [u8; 4],
    uv: [f32; 2],
}

pub struct D3D9RenderEngine {
    device: IDirect3DDevice9,

    texture_heap: TextureHeap,

    vertex_buffer: Buffer<IDirect3DVertexBuffer9, CustomVertex>,
    index_buffer: Buffer<IDirect3DIndexBuffer9, DrawIdx>,
    projection_buffer: Matrix4x4,
}

impl D3D9RenderEngine {
    pub fn new(device: &IDirect3DDevice9, ctx: &mut Context) -> Result<Self> {
        let device = device.clone();

        let texture_heap = TextureHeap::new(&device)?;

        let vertex_buffer = Buffer::new(&device, 5000)?;
        let index_buffer = Buffer::new(&device, 10000)?;
        let projection_buffer = Default::default();

        ctx.set_ini_filename(None);
        ctx.io_mut().backend_flags |= BackendFlags::RENDERER_HAS_VTX_OFFSET;
        ctx.set_renderer_name(String::from(concat!("hudhook-dx9@", env!("CARGO_PKG_VERSION"))));

        Ok(Self { device, texture_heap, vertex_buffer, index_buffer, projection_buffer })
    }
}

impl RenderContext for D3D9RenderEngine {
    fn load_texture(&mut self, data: &[u8], width: u32, height: u32) -> Result<TextureId> {
        unsafe {
            let texture_id = self.texture_heap.create_texture(width, height)?;
            self.texture_heap.upload_texture(texture_id, data, width, height)?;
            Ok(texture_id)
        }
    }

    fn replace_texture(
        &mut self,
        texture_id: TextureId,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<()> {
        unsafe { self.texture_heap.upload_texture(texture_id, data, width, height) }
    }
}

impl RenderEngine for D3D9RenderEngine {
    type RenderTarget = IDirect3DSurface9;

    fn render(
        &mut self,
        draw_data: &imgui::DrawData,
        render_target: Self::RenderTarget,
    ) -> Result<()> {
        unsafe {
            let state_backup = StateBackup::backup(&self.device)?;
            self.device.SetRenderTarget(0, &render_target)?;
            self.render_draw_data(draw_data)?;
            state_backup.restore(&self.device)?;
        }
        Ok(())
    }

    fn setup_fonts(&mut self, ctx: &mut Context) -> Result<()> {
        let fonts = ctx.fonts();
        let fonts_texture = fonts.build_rgba32_texture();
        fonts.tex_id =
            self.load_texture(fonts_texture.data, fonts_texture.width, fonts_texture.height)?;
        Ok(())
    }
}

impl D3D9RenderEngine {
    unsafe fn render_draw_data(&mut self, draw_data: &DrawData) -> Result<()> {
        self.vertex_buffer.clear();
        self.index_buffer.clear();

        draw_data
            .draw_lists()
            .map(|draw_list| {
                (draw_list.vtx_buffer().iter().copied(), draw_list.idx_buffer().iter().copied())
            })
            .for_each(|(vertices, indices)| {
                // CPU swizzle FTW
                self.vertex_buffer.extend(vertices.map(|draw_vert| CustomVertex {
                    pos: [draw_vert.pos[0], draw_vert.pos[1], 0.0],
                    col: [draw_vert.col[2], draw_vert.col[1], draw_vert.col[0], draw_vert.col[3]],
                    uv: draw_vert.uv,
                }));
                self.index_buffer.extend(indices);
            });

        self.vertex_buffer.upload(&self.device)?;
        self.index_buffer.upload(&self.device)?;

        self.projection_buffer = {
            let [l, t, r, b] = [
                draw_data.display_pos[0] + 0.5,
                draw_data.display_pos[1] + 0.5,
                draw_data.display_pos[0] + draw_data.display_size[0] + 0.5,
                draw_data.display_pos[1] + draw_data.display_size[1] + 0.5,
            ];

            Matrix4x4 {
                M11: 2. / (r - l),
                M22: 2. / (t - b),
                M33: 0.5,
                M41: (r + l) / (l - r),
                M42: (t + b) / (b - t),
                M43: 0.5,
                M44: 1.0,
                ..Default::default()
            }
        };

        self.setup_render_state(draw_data)?;

        let mut vtx_offset = 0usize;
        let mut idx_offset = 0usize;
        let mut last_texture = None;

        for cl in draw_data.draw_lists() {
            for cmd in cl.commands() {
                match cmd {
                    DrawCmd::Elements { count, cmd_params } => {
                        let [cx, cy, cw, ch] = cmd_params.clip_rect;
                        let [x, y] = draw_data.display_pos;
                        let r = RECT {
                            left: (cx - x) as i32,
                            top: (cy - y) as i32,
                            right: (cw - x) as i32,
                            bottom: (ch - y) as i32,
                        };

                        last_texture = match last_texture {
                            Some(t) if t == cmd_params.texture_id => Some(t),
                            None | Some(_) => {
                                let texture = self.texture_heap.get(cmd_params.texture_id);
                                self.device.SetTexture(0, texture)?;
                                Some(cmd_params.texture_id)
                            },
                        };

                        if r.right > r.left && r.bottom > r.top {
                            self.device.SetScissorRect(&r)?;
                            self.device.DrawIndexedPrimitive(
                                D3DPT_TRIANGLELIST,
                                (cmd_params.vtx_offset + vtx_offset) as i32,
                                0,
                                cl.vtx_buffer().len() as u32,
                                (cmd_params.idx_offset + idx_offset) as u32,
                                count as u32 / 3,
                            )?;
                        }
                    },
                    DrawCmd::ResetRenderState => {
                        self.setup_render_state(draw_data)?;
                    },
                    DrawCmd::RawCallback { callback, raw_cmd } => callback(cl.raw(), raw_cmd),
                }
            }
            idx_offset += cl.idx_buffer().len();
            vtx_offset += cl.vtx_buffer().len();
        }

        Ok(())
    }

    unsafe fn setup_render_state(&mut self, draw_data: &DrawData) -> Result<()> {
        self.device.SetViewport(&D3DVIEWPORT9 {
            X: 0,
            Y: 0,
            Width: draw_data.display_size[0] as u32,
            Height: draw_data.display_size[1] as u32,
            MinZ: 0.0,
            MaxZ: 1.0,
        })?;
        self.device.SetPixelShader(None)?;
        self.device.SetVertexShader(None)?;
        self.device.SetRenderState(D3DRS_CULLMODE, D3DCULL_NONE.0 as u32)?;
        self.device.SetRenderState(D3DRS_LIGHTING, false.into())?;
        self.device.SetRenderState(D3DRS_ZENABLE, false.into())?;
        self.device.SetRenderState(D3DRS_ALPHABLENDENABLE, true.into())?;
        self.device.SetRenderState(D3DRS_ALPHATESTENABLE, false.into())?;
        self.device.SetRenderState(D3DRS_BLENDOP, D3DBLENDOP_ADD.0 as u32)?;
        self.device.SetRenderState(D3DRS_SRCBLEND, D3DBLEND_SRCALPHA.0 as u32)?;
        self.device.SetRenderState(D3DRS_DESTBLEND, D3DBLEND_INVSRCALPHA.0 as u32)?;
        self.device.SetRenderState(D3DRS_SCISSORTESTENABLE, true.into())?;
        self.device.SetRenderState(D3DRS_SHADEMODE, D3DSHADE_GOURAUD.0 as u32)?;
        self.device.SetRenderState(D3DRS_FOGENABLE, false.into())?;
        self.device.SetTextureStageState(0, D3DTSS_COLOROP, D3DTOP_MODULATE.0 as u32)?;
        self.device.SetTextureStageState(0, D3DTSS_COLORARG1, D3DTA_TEXTURE)?;
        self.device.SetTextureStageState(0, D3DTSS_COLORARG2, D3DTA_DIFFUSE)?;
        self.device.SetTextureStageState(0, D3DTSS_ALPHAOP, D3DTOP_MODULATE.0 as u32)?;
        self.device.SetTextureStageState(0, D3DTSS_ALPHAARG1, D3DTA_TEXTURE)?;
        self.device.SetTextureStageState(0, D3DTSS_ALPHAARG2, D3DTA_DIFFUSE)?;
        self.device.SetSamplerState(0, D3DSAMP_MINFILTER, D3DTEXF_LINEAR.0 as u32)?;
        self.device.SetSamplerState(0, D3DSAMP_MAGFILTER, D3DTEXF_LINEAR.0 as u32)?;
        self.device.SetTransform(D3DTRANSFORMSTATETYPE(256), &MAT_IDENTITY)?;
        self.device.SetTransform(D3DTS_VIEW, &MAT_IDENTITY)?;
        self.device.SetTransform(D3DTS_PROJECTION, &self.projection_buffer)?;
        self.device.SetStreamSource(
            0,
            &self.vertex_buffer.resource,
            0,
            mem::size_of::<CustomVertex>() as u32,
        )?;
        self.device.SetIndices(&self.index_buffer.resource)?;
        self.device.SetFVF(D3DFVF_CUSTOMVERTEX)?;

        Ok(())
    }
}

trait BufferType: Sized {
    fn create_resource(device: &IDirect3DDevice9, resource_capacity: usize) -> Result<Self>;
    fn upload<T>(&mut self, data: &[T]) -> Result<()>;
}

struct Buffer<B: BufferType, T> {
    resource: B,
    resource_capacity: usize,
    data: Vec<T>,
}

impl<B: BufferType, T> Buffer<B, T> {
    fn new(device: &IDirect3DDevice9, resource_capacity: usize) -> Result<Self> {
        let resource = B::create_resource(device, resource_capacity)?;
        let data = Vec::with_capacity(resource_capacity);

        Ok(Self { resource, resource_capacity, data })
    }

    fn clear(&mut self) {
        self.data.clear();
    }

    fn extend<I: IntoIterator<Item = T>>(&mut self, it: I) {
        self.data.extend(it)
    }

    fn upload(&mut self, device: &IDirect3DDevice9) -> Result<()> {
        let capacity = self.data.capacity();
        if capacity > self.resource_capacity {
            drop(mem::replace(&mut self.resource, B::create_resource(device, capacity)?));
            self.resource_capacity = capacity;
        }

        self.resource.upload(&self.data)?;

        Ok(())
    }
}

impl BufferType for IDirect3DVertexBuffer9 {
    fn create_resource(
        device: &IDirect3DDevice9,
        resource_capacity: usize,
    ) -> Result<IDirect3DVertexBuffer9> {
        util::try_out_ptr(|v| unsafe {
            device.CreateVertexBuffer(
                (resource_capacity * mem::size_of::<DrawVert>()) as u32,
                (D3DUSAGE_DYNAMIC | D3DUSAGE_WRITEONLY) as u32,
                D3DFVF_CUSTOMVERTEX,
                D3DPOOL_DEFAULT,
                v,
                ptr::null_mut(),
            )
        })
    }

    fn upload<T>(&mut self, data: &[T]) -> Result<()> {
        unsafe {
            let mut resource_ptr = ptr::null_mut();

            self.Lock(0, mem::size_of_val(data) as u32, &mut resource_ptr, D3DLOCK_DISCARD as u32)?;

            ptr::copy_nonoverlapping(data.as_ptr(), resource_ptr as *mut T, data.len());

            self.Unlock()?;
        }

        Ok(())
    }
}

impl BufferType for IDirect3DIndexBuffer9 {
    fn create_resource(
        device: &IDirect3DDevice9,
        resource_capacity: usize,
    ) -> Result<IDirect3DIndexBuffer9> {
        util::try_out_ptr(|v| unsafe {
            device.CreateIndexBuffer(
                (resource_capacity * mem::size_of::<DrawIdx>()) as u32,
                (D3DUSAGE_DYNAMIC | D3DUSAGE_WRITEONLY) as u32,
                if mem::size_of::<DrawIdx>() == 2 { D3DFMT_INDEX16 } else { D3DFMT_INDEX32 },
                D3DPOOL_DEFAULT,
                v,
                ptr::null_mut(),
            )
        })
    }

    fn upload<T>(&mut self, data: &[T]) -> Result<()> {
        unsafe {
            let mut resource_ptr = ptr::null_mut();

            self.Lock(0, mem::size_of_val(data) as u32, &mut resource_ptr, D3DLOCK_DISCARD as u32)?;

            ptr::copy_nonoverlapping(data.as_ptr(), resource_ptr as *mut T, data.len());

            self.Unlock()?;
        }

        Ok(())
    }
}

#[derive(Debug)]
#[allow(unused)]
struct Texture {
    resource: IDirect3DTexture9,
    id: TextureId,
    width: u32,
    height: u32,
}

struct TextureHeap {
    device: IDirect3DDevice9,
    textures: Vec<Texture>,
}

impl TextureHeap {
    fn new(device: &IDirect3DDevice9) -> Result<Self> {
        Ok(Self { device: device.clone(), textures: Vec::new() })
    }

    fn get(&self, texture_id: TextureId) -> &IDirect3DTexture9 {
        &self.textures[texture_id.id()].resource
    }

    unsafe fn create_texture(&mut self, width: u32, height: u32) -> Result<TextureId> {
        let resource = util::try_out_ptr(|v| {
            self.device.CreateTexture(
                width,
                height,
                1,
                D3DUSAGE_DYNAMIC as u32,
                D3DFMT_A8R8G8B8,
                D3DPOOL_DEFAULT,
                v,
                ptr::null_mut(),
            )
        })?;

        let id = TextureId::from(self.textures.len());
        self.textures.push(Texture { resource, id, width, height });

        Ok(id)
    }

    unsafe fn upload_texture(
        &mut self,
        texture_id: TextureId,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<()> {
        let texture = &self.textures[texture_id.id()];
        if texture.width != width || texture.height != height {
            error!(
                "image size {width}x{height} do not match expected {}x{}",
                texture.width, texture.height
            );
            return Err(Error::from_hresult(HRESULT(-1)));
        }

        let mut r: D3DLOCKED_RECT = Default::default();
        texture.resource.LockRect(0, &mut r, ptr::null_mut(), 0)?;

        let bits = r.pBits as *mut u8;
        let pitch = r.Pitch as usize;
        let height = height as usize;
        let width = width as usize;

        // CPU swizzle FTW
        for y in 0..height {
            for x in 0..width {
                let offset_dest = pitch * y + x * 4;
                let offset_src = width * 4 * y + x * 4;
                *bits.add(offset_dest) = data[offset_src + 2];
                *bits.add(offset_dest + 1) = data[offset_src + 1];
                *bits.add(offset_dest + 2) = data[offset_src];
                *bits.add(offset_dest + 3) = data[offset_src + 3];
            }
        }

        texture.resource.UnlockRect(0)?;

        Ok(())
    }
}

struct StateBackup {
    state_block: IDirect3DStateBlock9,
    mat_world: Matrix4x4,
    mat_view: Matrix4x4,
    mat_projection: Matrix4x4,
    viewport: D3DVIEWPORT9,
    surface: IDirect3DSurface9,
}

impl StateBackup {
    unsafe fn backup(device: &IDirect3DDevice9) -> Result<Self> {
        match device.CreateStateBlock(D3DSBT_ALL) {
            Ok(state_block) => {
                let mut mat_world: Matrix4x4 = Default::default();
                let mut mat_view: Matrix4x4 = Default::default();
                let mut mat_projection: Matrix4x4 = Default::default();
                let mut viewport: D3DVIEWPORT9 = core::mem::zeroed();

                device.GetTransform(D3DTRANSFORMSTATETYPE(256), &mut mat_world)?;
                device.GetTransform(D3DTS_VIEW, &mut mat_view)?;
                device.GetTransform(D3DTS_PROJECTION, &mut mat_projection)?;
                device.GetViewport(&mut viewport)?;
                let surface = device.GetRenderTarget(0)?;

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
        self.state_block.Apply()?;
        device.SetTransform(D3DTRANSFORMSTATETYPE(256), &self.mat_world)?;
        device.SetTransform(D3DTS_VIEW, &self.mat_view)?;
        device.SetTransform(D3DTS_PROJECTION, &self.mat_projection)?;
        device.SetViewport(&self.viewport)?;
        device.SetRenderTarget(0, &self.surface)?;
        Ok(())
    }
}
