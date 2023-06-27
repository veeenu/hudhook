use std::ptr::null_mut;

use imgui::internal::RawWrapper;
use imgui::{Context, DrawCmd, DrawData, DrawListIterator, DrawVert};
use tracing::{error, trace};
use windows::core::{Error, PCSTR};
use windows::Win32::Foundation::{BOOL, HWND, RECT};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D11_SRV_DIMENSION_TEXTURE2D,
    D3D_DRIVER_TYPE_HARDWARE, D3D_PRIMITIVE_TOPOLOGY,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDeviceAndSwapChain, ID3D11BlendState, ID3D11Buffer, ID3D11ClassInstance,
    ID3D11DepthStencilState, ID3D11Device, ID3D11DeviceContext, ID3D11InputLayout,
    ID3D11PixelShader, ID3D11RasterizerState, ID3D11RenderTargetView, ID3D11Resource,
    ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_INDEX_BUFFER, D3D11_BIND_SHADER_RESOURCE,
    D3D11_BIND_VERTEX_BUFFER, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_OP_ADD,
    D3D11_BLEND_SRC_ALPHA, D3D11_BLEND_ZERO, D3D11_BUFFER_DESC, D3D11_COLOR_WRITE_ENABLE_ALL,
    D3D11_COMPARISON_ALWAYS, D3D11_CPU_ACCESS_FLAG, D3D11_CPU_ACCESS_WRITE,
    D3D11_CREATE_DEVICE_DEBUG, D3D11_CREATE_DEVICE_FLAG, D3D11_CULL_NONE,
    D3D11_DEPTH_STENCILOP_DESC, D3D11_DEPTH_STENCIL_DESC, D3D11_DEPTH_WRITE_MASK_ALL,
    D3D11_FILL_SOLID, D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_INPUT_ELEMENT_DESC,
    D3D11_INPUT_PER_VERTEX_DATA, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_WRITE_DISCARD,
    D3D11_RASTERIZER_DESC, D3D11_RENDER_TARGET_BLEND_DESC, D3D11_RESOURCE_MISC_FLAG,
    D3D11_SAMPLER_DESC, D3D11_SDK_VERSION, D3D11_SHADER_RESOURCE_VIEW_DESC,
    D3D11_SHADER_RESOURCE_VIEW_DESC_0, D3D11_STENCIL_OP_KEEP, D3D11_SUBRESOURCE_DATA,
    D3D11_TEX2D_SRV, D3D11_TEXTURE2D_DESC, D3D11_TEXTURE_ADDRESS_WRAP, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_DYNAMIC, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_R16_UINT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R32_UINT,
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_MODE_DESC, DXGI_MODE_SCALING, DXGI_MODE_SCANLINE_ORDER,
    DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGISwapChain, DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_EFFECT, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

const DEVICE_FLAGS: D3D11_CREATE_DEVICE_FLAG = D3D11_CREATE_DEVICE_DEBUG;

/// The DirectX11 imgui render engine.
pub struct RenderEngine {
    dasc: DeviceAndSwapChain,
    shader_program: ShaderProgram,
    buffers: Buffers,
    texture: Texture,
}

impl RenderEngine {
    /// Initialize render engine from hwnd and imgui context.
    pub fn new(hwnd: HWND, ctx: &mut Context) -> Self {
        let dasc = DeviceAndSwapChain::new(hwnd);
        let shader_program = ShaderProgram::new(&dasc).expect("ShaderProgram");
        let buffers = Buffers::new(&dasc);
        let texture = Texture::new(&dasc, ctx.fonts()).expect("Texture");

        ctx.set_renderer_name(String::from(concat!("imgui-dx11@", env!("CARGO_PKG_VERSION"))));

        RenderEngine { dasc, shader_program, buffers, texture }
    }

    /// Initialize render engine from DirectX11 objects and imgui context.
    pub fn new_with_ptrs(
        dev: ID3D11Device,
        dev_ctx: ID3D11DeviceContext,
        swap_chain: IDXGISwapChain,
        ctx: &mut Context,
    ) -> Self {
        let dasc = DeviceAndSwapChain::new_with_ptrs(dev, dev_ctx, swap_chain);
        let shader_program = ShaderProgram::new(&dasc).expect("ShaderProgram");
        let buffers = Buffers::new(&dasc);
        let texture = Texture::new(&dasc, ctx.fonts()).expect("Texture");

        ctx.set_renderer_name(String::from(concat!("imgui-dx11@", env!("CARGO_PKG_VERSION"))));

        RenderEngine { dasc, shader_program, buffers, texture }
    }

    pub fn dev(&self) -> ID3D11Device {
        self.dasc.dev()
    }

    pub fn dev_ctx(&self) -> ID3D11DeviceContext {
        self.dasc.dev_ctx()
    }

    pub fn swap_chain(&self) -> IDXGISwapChain {
        self.dasc.swap_chain()
    }

    pub fn get_client_rect(&self) -> Option<RECT> {
        self.dasc.get_client_rect()
    }

    pub fn render_draw_data(&mut self, draw_data: &DrawData) -> Result<(), String> {
        trace!("Rendering started");
        let state_backup = StateBackup::backup(self.dasc.dev_ctx());

        let [x, y] = draw_data.display_pos;
        let [width, height] = draw_data.display_size;

        if width <= 0. && height <= 0. {
            return Err(format!("Insufficient display size {width} x {height}"));
        }

        let rect = RECT { left: 0, right: width as i32, top: 0, bottom: height as i32 };
        self.dasc.set_viewport(rect);
        self.dasc.set_render_target();
        unsafe { self.shader_program.set_state(&self.dasc) };

        unsafe {
            let dev_ctx = self.dasc.dev_ctx();

            trace!("Setting up buffers");
            self.buffers.set_constant_buffer(&self.dasc, [x, y, x + width, y + height]);
            self.buffers.set_buffers(&self.dasc, draw_data.draw_lists());

            dev_ctx.IASetVertexBuffers(
                0,
                1,
                &Some(self.buffers.vtx_buffer()),
                &(std::mem::size_of::<DrawVert>() as u32),
                &0,
            );
            dev_ctx.IASetIndexBuffer(
                &self.buffers.idx_buffer(),
                if std::mem::size_of::<imgui::DrawIdx>() == 2 {
                    DXGI_FORMAT_R16_UINT
                } else {
                    DXGI_FORMAT_R32_UINT
                },
                0,
            );
            dev_ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            dev_ctx.VSSetConstantBuffers(0, &[Some(self.buffers.mtx_buffer())]);
            dev_ctx.PSSetShaderResources(0, &[Some(self.texture.tex_view())]);

            let mut vtx_offset = 0usize;
            let mut idx_offset = 0usize;

            trace!("Rendering draw lists");
            for cl in draw_data.draw_lists() {
                for cmd in cl.commands() {
                    match cmd {
                        DrawCmd::Elements { count, cmd_params } => {
                            trace!("Rendering {count} elements");
                            let [cx, cy, cw, ch] = cmd_params.clip_rect;
                            dev_ctx.RSSetScissorRects(&[RECT {
                                left: (cx - x) as i32,
                                top: (cy - y) as i32,
                                right: (cw - x) as i32,
                                bottom: (ch - y) as i32,
                            }]);

                            // let srv = cmd_params.texture_id.id();
                            // We only load the font texture. This may not be correct.
                            self.dasc.set_shader_resources(self.texture.tex_view());

                            trace!("Drawing indexed {count}, {idx_offset}, {vtx_offset}");
                            dev_ctx.DrawIndexed(
                                count as u32,
                                (cmd_params.idx_offset + idx_offset) as _,
                                (cmd_params.vtx_offset + vtx_offset) as _,
                            );
                        },
                        DrawCmd::ResetRenderState => {
                            trace!("Resetting render state");
                            self.dasc.setup_state(draw_data);
                            self.shader_program.set_state(&self.dasc);
                        },
                        DrawCmd::RawCallback { callback, raw_cmd } => {
                            trace!("Executing raw callback");
                            callback(cl.raw(), raw_cmd)
                        },
                    }
                }
                idx_offset += cl.idx_buffer().len();
                vtx_offset += cl.vtx_buffer().len();
            }
        }

        trace!("Restoring state backup");
        state_backup.restore(self.dasc.dev_ctx());

        trace!("Rendering done");

        Ok(())
    }

    pub fn present(&self) {
        if let Err(e) = unsafe { self.dasc.swap_chain().Present(1, 0).ok() } {
            error!("Present: {e}");
        }
    }
}

struct DeviceAndSwapChain {
    dev: ID3D11Device,
    dev_ctx: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain,
    back_buffer: ID3D11RenderTargetView,
}

impl DeviceAndSwapChain {
    fn new(hwnd: HWND) -> Self {
        let mut swap_chain: Option<IDXGISwapChain> = None;
        let mut dev: Option<ID3D11Device> = None;
        let mut dev_ctx: Option<ID3D11DeviceContext> = None;

        unsafe {
            D3D11CreateDeviceAndSwapChain(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                None,
                DEVICE_FLAGS,
                &[],
                D3D11_SDK_VERSION,
                &DXGI_SWAP_CHAIN_DESC {
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
                } as *const _,
                &mut swap_chain as *mut _,
                &mut dev as *mut _,
                null_mut(),
                &mut dev_ctx as *mut _,
            )
            .unwrap()
        };

        DeviceAndSwapChain::new_with_ptrs(
            dev.expect("Null device"),
            dev_ctx.expect("Null device context"),
            swap_chain.expect("Null swap chain"),
        )
    }

    fn new_with_ptrs(
        dev: ID3D11Device,
        dev_ctx: ID3D11DeviceContext,
        swap_chain: IDXGISwapChain,
    ) -> Self {
        let back_buffer = unsafe {
            let p_back_buffer: ID3D11Resource = swap_chain.GetBuffer(0).expect("GetBuffer");

            let back_buffer = dev
                .CreateRenderTargetView(&p_back_buffer, null_mut())
                .expect("CreateRenderTargetView");

            dev_ctx.OMSetRenderTargets(&[Some(back_buffer.clone())], None);

            back_buffer
        };

        unsafe {
            dev_ctx.RSSetViewports(&[D3D11_VIEWPORT {
                TopLeftX: 0.,
                TopLeftY: 0.,
                Width: 640.,
                Height: 480.,
                MinDepth: 0.,
                MaxDepth: 1.,
            }])
        };

        DeviceAndSwapChain { dev, dev_ctx, swap_chain, back_buffer }
    }

    fn setup_state(&self, draw_data: &imgui::DrawData) {
        let [_x, _y] = draw_data.display_pos;
        let [_w, _h] = draw_data.display_size;

        self.set_render_target();
    }

    fn set_shader_resources(&self, srv: ID3D11ShaderResourceView) {
        unsafe { self.dev_ctx.PSSetShaderResources(0, &[Some(srv)]) }
    }

    fn set_viewport(&self, rect: RECT) {
        unsafe {
            self.dev_ctx().RSSetViewports(&[D3D11_VIEWPORT {
                TopLeftX: 0.,
                TopLeftY: 0.,
                Width: (rect.right - rect.left) as f32,
                Height: (rect.bottom - rect.top) as f32,
                MinDepth: 0.,
                MaxDepth: 1.,
            }])
        };
    }

    fn set_render_target(&self) {
        unsafe {
            self.dev_ctx.OMSetRenderTargets(&[Some(self.back_buffer.clone())], None);
        }
    }

    fn get_client_rect(&self) -> Option<RECT> {
        unsafe {
            let sd = self.swap_chain.GetDesc().expect("GetDesc");
            let mut rect: RECT = Default::default();
            if GetClientRect(sd.OutputWindow, &mut rect as _) != BOOL(0) {
                Some(rect)
            } else {
                None
            }
        }
    }

    fn with_mapped<F>(&self, ptr: &ID3D11Buffer, f: F)
    where
        F: FnOnce(&D3D11_MAPPED_SUBRESOURCE),
    {
        unsafe {
            let ms = self.dev_ctx.Map(ptr, 0, D3D11_MAP_WRITE_DISCARD, 0).expect("Map");

            f(&ms);

            self.dev_ctx.Unmap(ptr, 0);
        }
    }

    fn dev(&self) -> ID3D11Device {
        self.dev.clone()
    }

    fn dev_ctx(&self) -> ID3D11DeviceContext {
        self.dev_ctx.clone()
    }

    fn swap_chain(&self) -> IDXGISwapChain {
        self.swap_chain.clone()
    }
}

struct Buffers {
    vtx_buffer: ID3D11Buffer,
    idx_buffer: ID3D11Buffer,
    mtx_buffer: ID3D11Buffer,

    vtx_count: usize,
    idx_count: usize,
}

#[repr(transparent)]
struct VertexConstantBuffer([[f32; 4]; 4]);

impl Buffers {
    fn new(dasc: &DeviceAndSwapChain) -> Self {
        let mtx_buffer = unsafe {
            dasc.dev()
                .CreateBuffer(
                    &D3D11_BUFFER_DESC {
                        ByteWidth: std::mem::size_of::<VertexConstantBuffer>() as u32,
                        Usage: D3D11_USAGE_DYNAMIC,
                        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0,
                        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0,
                        MiscFlags: 0,
                        StructureByteStride: 0,
                    } as *const _,
                    null_mut(),
                )
                .expect("CreateBuffer")
        };

        let vtx_buffer = Buffers::create_vertex_buffer(dasc, 1);
        let idx_buffer = Buffers::create_index_buffer(dasc, 1);

        Buffers { vtx_buffer, idx_buffer, mtx_buffer, vtx_count: 1, idx_count: 1 }
    }

    fn set_constant_buffer(&mut self, dasc: &DeviceAndSwapChain, rect: [f32; 4]) {
        let [l, t, r, b] = rect;

        dasc.with_mapped(&self.mtx_buffer, |ms| unsafe {
            std::ptr::copy_nonoverlapping(
                &VertexConstantBuffer([
                    [2. / (r - l), 0., 0., 0.],
                    [0., 2. / (t - b), 0., 0.],
                    [0., 0., 0.5, 0.],
                    [(r + l) / (l - r), (t + b) / (b - t), 0.5, 1.0],
                ]),
                ms.pData as *mut _,
                1,
            );
        })
    }

    fn set_buffers(&mut self, dasc: &DeviceAndSwapChain, meshes: DrawListIterator) {
        let (vertices, indices): (Vec<DrawVert>, Vec<u16>) = meshes
            .map(|m| (m.vtx_buffer().iter(), m.idx_buffer().iter()))
            .fold((Vec::new(), Vec::new()), |(mut ov, mut oi), (v, i)| {
                ov.extend(v);
                oi.extend(i);
                (ov, oi)
            });

        self.resize(dasc, vertices.len(), indices.len());

        dasc.with_mapped(&self.vtx_buffer, |ms| unsafe {
            std::ptr::copy_nonoverlapping(vertices.as_ptr(), ms.pData as _, vertices.len());
        });

        dasc.with_mapped(&self.idx_buffer, |ms| unsafe {
            std::ptr::copy_nonoverlapping(indices.as_ptr(), ms.pData as _, indices.len());
        });
    }

    fn resize(&mut self, dasc: &DeviceAndSwapChain, vtx_count: usize, idx_count: usize) {
        if self.vtx_count <= vtx_count || (self.vtx_count == 0 && vtx_count == 0) {
            // unsafe { self.vtx_buffer.as_ref().Release() };
            self.vtx_count = vtx_count + 4096;
            self.vtx_buffer = Buffers::create_vertex_buffer(dasc, self.vtx_count);
        }

        if self.idx_count <= idx_count || (self.idx_count == 0 && idx_count == 0) {
            // unsafe { self.idx_buffer.as_ref().Release() };
            self.idx_count = idx_count + 4096;
            self.idx_buffer = Buffers::create_index_buffer(dasc, self.idx_count);
        }
    }

    fn create_vertex_buffer(dasc: &DeviceAndSwapChain, size: usize) -> ID3D11Buffer {
        unsafe {
            dasc.dev()
                .CreateBuffer(
                    &D3D11_BUFFER_DESC {
                        Usage: D3D11_USAGE_DYNAMIC,
                        ByteWidth: (size * std::mem::size_of::<imgui::DrawVert>()) as u32,
                        BindFlags: D3D11_BIND_VERTEX_BUFFER.0,
                        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0,
                        MiscFlags: 0,
                        StructureByteStride: 0,
                    },
                    null_mut(),
                )
                .expect("CreateBuffer")
        }
    }

    fn create_index_buffer(dasc: &DeviceAndSwapChain, size: usize) -> ID3D11Buffer {
        unsafe {
            dasc.dev()
                .CreateBuffer(
                    &D3D11_BUFFER_DESC {
                        Usage: D3D11_USAGE_DYNAMIC,
                        ByteWidth: (size * std::mem::size_of::<u32>()) as u32,
                        BindFlags: D3D11_BIND_INDEX_BUFFER.0,
                        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0,
                        MiscFlags: 0,
                        StructureByteStride: 0,
                    },
                    null_mut(),
                )
                .expect("CreateBuffer")
        }
    }

    fn vtx_buffer(&self) -> ID3D11Buffer {
        self.vtx_buffer.clone()
    }

    fn idx_buffer(&self) -> ID3D11Buffer {
        self.idx_buffer.clone()
    }

    fn mtx_buffer(&self) -> ID3D11Buffer {
        self.mtx_buffer.clone()
    }
}

struct Texture {
    _tex: ID3D11Texture2D,
    tex_view: ID3D11ShaderResourceView,
    _font_sampler: ID3D11SamplerState,
}

impl Texture {
    // TODO FontAtlasTexture may be too specific?
    fn new(dasc: &DeviceAndSwapChain, fonts: &mut imgui::FontAtlas) -> Result<Texture, Error> {
        let texture = fonts.build_rgba32_texture();
        let data = texture.data.to_vec();

        let tex = unsafe {
            dasc.dev().CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: texture.width as _,
                    Height: texture.height as _,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE,
                    CPUAccessFlags: D3D11_CPU_ACCESS_FLAG(0),
                    MiscFlags: D3D11_RESOURCE_MISC_FLAG(0),
                } as *const _,
                &D3D11_SUBRESOURCE_DATA {
                    pSysMem: data.as_ptr() as _,
                    SysMemPitch: texture.width * 4,
                    SysMemSlicePitch: 0,
                } as *const _,
            )?
        };

        let tex_view = unsafe {
            dasc.dev().CreateShaderResourceView(&tex, &D3D11_SHADER_RESOURCE_VIEW_DESC {
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                ViewDimension: D3D11_SRV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_SRV { MostDetailedMip: 0, MipLevels: 1 },
                },
            } as *const _)?
        };

        let _font_sampler = unsafe {
            dasc.dev().CreateSamplerState(&D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_WRAP,
                AddressV: D3D11_TEXTURE_ADDRESS_WRAP,
                AddressW: D3D11_TEXTURE_ADDRESS_WRAP,
                MipLODBias: 0.,
                ComparisonFunc: D3D11_COMPARISON_ALWAYS,
                MinLOD: 0.,
                MaxLOD: 0.,
                BorderColor: [0.; 4],
                MaxAnisotropy: 0,
            })?
        };

        fonts.tex_id = imgui::TextureId::from(&tex_view as *const _ as usize);
        trace!("Texture view: {:x} id: {:x}", &tex_view as *const _ as usize, fonts.tex_id.id());

        Ok(Texture { _tex: tex, tex_view, _font_sampler })
    }

    fn tex_view(&self) -> ID3D11ShaderResourceView {
        self.tex_view.clone()
    }
}

const D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE: usize = 16;

#[derive(Default)]
struct StateBackup {
    scissor_rects_count: u32,
    viewports_count: u32,
    scissor_rects: [RECT; D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE],
    viewports: [D3D11_VIEWPORT; D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE],
    rasterizer_state: Option<ID3D11RasterizerState>,
    blend_state: Option<ID3D11BlendState>,
    blend_factor: [f32; 4],
    sample_mask: u32,
    stencil_ref: u32,
    depth_stencil_state: Option<ID3D11DepthStencilState>,
    ps_shader_resource: [Option<ID3D11ShaderResourceView>; 1],
    _ps_sampler: Option<ID3D11SamplerState>,
    pixel_shader: Option<ID3D11PixelShader>,
    vertex_shader: Option<ID3D11VertexShader>,
    ps_instances_count: u32,
    vs_instances_count: u32,
    ps_instances: Option<ID3D11ClassInstance>,
    vs_instances: Option<ID3D11ClassInstance>,
    primitive_topology: D3D_PRIMITIVE_TOPOLOGY,
    index_buffer: Option<ID3D11Buffer>,
    vertex_buffer: Option<ID3D11Buffer>,
    vertex_constant_buffer: [Option<ID3D11Buffer>; 1],
    index_buffer_offset: u32,
    vertex_buffer_stride: u32,
    vertex_buffer_offset: u32,
    index_buffer_format: DXGI_FORMAT,
    input_layout: Option<ID3D11InputLayout>,
}

impl StateBackup {
    fn backup(ctx: ID3D11DeviceContext) -> StateBackup {
        let mut r: StateBackup = StateBackup {
            scissor_rects_count: D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE as _,
            viewports_count: D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE as _,
            ..Default::default()
        };
        unsafe {
            ctx.RSGetScissorRects(
                &mut r.scissor_rects_count,
                &mut r.scissor_rects as *mut _ as *mut _,
            );
            ctx.RSGetViewports(&mut r.viewports_count, &mut r.viewports as *mut _ as *mut _);
            ctx.RSGetState(&mut r.rasterizer_state);
            ctx.OMGetBlendState(&mut r.blend_state, &mut r.blend_factor as _, &mut r.sample_mask);
            ctx.OMGetDepthStencilState(&mut r.depth_stencil_state, &mut r.stencil_ref);
            ctx.PSGetShaderResources(0, &mut r.ps_shader_resource);
            r.ps_instances_count = 256;
            r.vs_instances_count = 256;
            ctx.PSGetShader(
                &mut r.pixel_shader,
                &mut r.ps_instances as *mut _,
                &mut r.ps_instances_count,
            );
            ctx.VSGetShader(&mut r.vertex_shader, &mut r.vs_instances, &mut r.vs_instances_count);
            ctx.VSGetConstantBuffers(0, &mut r.vertex_constant_buffer);
            ctx.IAGetPrimitiveTopology(&mut r.primitive_topology);
            ctx.IAGetIndexBuffer(
                &mut r.index_buffer,
                &mut r.index_buffer_format,
                &mut r.index_buffer_offset,
            );
            ctx.IAGetVertexBuffers(
                0,
                1,
                &mut r.vertex_buffer,
                &mut r.vertex_buffer_stride,
                &mut r.vertex_buffer_offset,
            );
            ctx.IAGetInputLayout(&mut r.input_layout);
        }

        r
    }

    fn restore(self, ctx: ID3D11DeviceContext) {
        unsafe {
            ctx.RSSetScissorRects(&self.scissor_rects);
            ctx.RSSetViewports(&self.viewports);
            ctx.RSSetState(self.rasterizer_state.as_ref());
            ctx.OMSetBlendState(
                self.blend_state.as_ref(),
                &self.blend_factor as _,
                self.sample_mask,
            );
            ctx.OMSetDepthStencilState(self.depth_stencil_state.as_ref(), self.stencil_ref);
            ctx.PSSetShaderResources(0, &self.ps_shader_resource);
            // ctx.PSSetSamplers(0, &[self.ps_sampler]);
            if self.ps_instances.is_some() {
                ctx.PSSetShader(self.pixel_shader.as_ref(), &[self.ps_instances]);
            }
            if self.vs_instances.is_some() {
                ctx.VSSetShader(self.vertex_shader.as_ref(), &[self.vs_instances]);
            }
            ctx.IASetPrimitiveTopology(self.primitive_topology);
            ctx.IASetIndexBuffer(
                self.index_buffer.as_ref(),
                self.index_buffer_format,
                self.index_buffer_offset,
            );
            ctx.IASetVertexBuffers(
                0,
                1,
                &self.vertex_buffer,
                &self.vertex_buffer_stride,
                &self.vertex_buffer_offset,
            );
            ctx.IASetInputLayout(self.input_layout.as_ref());
        }
    }
}

////////////////////////////////////////////////////////////////////////////////
// Shaders
////////////////////////////////////////////////////////////////////////////////

const VERTEX_SHADER_SRC: &str = r"
  cbuffer vertexBuffer : register(b0) {
    float4x4 ProjectionMatrix;
  };
  struct VS_INPUT {
    float2 pos : POSITION;
    float4 col : COLOR0;
    float2 uv  : TEXCOORD0;
  };
  struct PS_INPUT {
    float4 pos : SV_POSITION;
    float4 col : COLOR0;
    float2 uv  : TEXCOORD0;
  };
  PS_INPUT main(VS_INPUT input) {
    PS_INPUT output;
    output.pos = mul(ProjectionMatrix, float4(input.pos.xy, 0.f, 1.f));
    output.col = input.col;
    output.uv  = input.uv;
    return output;
  }
";

const PIXEL_SHADER_SRC: &str = r"
  struct PS_INPUT {
    float4 pos : SV_POSITION;
    float4 col : COLOR0;
    float2 uv  : TEXCOORD0;
  };
  sampler sampler0;
  Texture2D texture0;
  float4 main(PS_INPUT input) : SV_Target {
    float4 out_col = input.col * texture0.Sample(sampler0, input.uv);
    return out_col;
  };
";

struct ShaderProgram {
    vtx_shader: ID3D11VertexShader,
    pix_shader: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    sampler: ID3D11SamplerState,
    rasterizer_state: ID3D11RasterizerState,
    blend_state: ID3D11BlendState,
    depth_stencil_state: ID3D11DepthStencilState,
}

impl ShaderProgram {
    fn new(dasc: &DeviceAndSwapChain) -> Result<ShaderProgram, Error> {
        let mut vs_blob: Option<ID3DBlob> = None;
        let mut ps_blob: Option<ID3DBlob> = None;

        unsafe {
            D3DCompile(
                VERTEX_SHADER_SRC.as_ptr() as _,
                VERTEX_SHADER_SRC.len(),
                None,
                null_mut(),
                None,
                PCSTR("main\0".as_ptr() as _),
                PCSTR("vs_4_0\0".as_ptr() as _),
                0,
                0,
                &mut vs_blob,
                &mut None,
            )?
        };

        unsafe {
            D3DCompile(
                PIXEL_SHADER_SRC.as_ptr() as _,
                PIXEL_SHADER_SRC.len(),
                None,
                null_mut(),
                None,
                PCSTR("main\0".as_ptr() as _),
                PCSTR("ps_4_0\0".as_ptr() as _),
                0,
                0,
                &mut ps_blob as *mut _ as _,
                &mut None,
            )?
        };

        let vtx_shader = unsafe {
            let vs_blob = vs_blob.as_ref().unwrap();
            let ptr = vs_blob.GetBufferPointer();
            let size = vs_blob.GetBufferSize();
            dasc.dev().CreateVertexShader(std::slice::from_raw_parts(ptr as _, size), None)?
        };

        let pix_shader = unsafe {
            let ps_blob = ps_blob.as_ref().unwrap();
            let ptr = ps_blob.GetBufferPointer();
            let size = ps_blob.GetBufferSize();
            dasc.dev().CreatePixelShader(std::slice::from_raw_parts(ptr as _, size), None)?
        };

        let layout = unsafe {
            let vs_blob = vs_blob.as_ref().unwrap();
            let ptr = vs_blob.GetBufferPointer();
            let size = vs_blob.GetBufferSize();
            dasc.dev().CreateInputLayout(
                &[
                    D3D11_INPUT_ELEMENT_DESC {
                        SemanticName: PCSTR("POSITION\0".as_ptr() as _),
                        SemanticIndex: 0,
                        Format: DXGI_FORMAT_R32G32_FLOAT,
                        InputSlot: 0,
                        AlignedByteOffset: 0,
                        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                        InstanceDataStepRate: 0,
                    },
                    D3D11_INPUT_ELEMENT_DESC {
                        SemanticName: PCSTR("TEXCOORD\0".as_ptr() as _),
                        SemanticIndex: 0,
                        Format: DXGI_FORMAT_R32G32_FLOAT,
                        InputSlot: 0,
                        AlignedByteOffset: 8,
                        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                        InstanceDataStepRate: 0,
                    },
                    D3D11_INPUT_ELEMENT_DESC {
                        SemanticName: PCSTR("COLOR\0".as_ptr() as _),
                        SemanticIndex: 0,
                        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                        InputSlot: 0,
                        AlignedByteOffset: 16,
                        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                        InstanceDataStepRate: 0,
                    },
                ],
                std::slice::from_raw_parts(ptr as _, size),
            )?
        };

        let sampler = unsafe {
            dasc.dev().CreateSamplerState(&D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_WRAP,
                AddressV: D3D11_TEXTURE_ADDRESS_WRAP,
                AddressW: D3D11_TEXTURE_ADDRESS_WRAP,
                MipLODBias: 0.,
                ComparisonFunc: D3D11_COMPARISON_ALWAYS,
                MinLOD: 0.,
                MaxLOD: 0.,
                BorderColor: [0.; 4],
                MaxAnisotropy: 0,
            })?
        };

        let blend_state = unsafe {
            dasc.dev().CreateBlendState(&D3D11_BLEND_DESC {
                AlphaToCoverageEnable: BOOL(0),
                IndependentBlendEnable: BOOL(0),
                RenderTarget: [
                    D3D11_RENDER_TARGET_BLEND_DESC {
                        BlendEnable: BOOL(1),
                        SrcBlend: D3D11_BLEND_SRC_ALPHA,
                        DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
                        BlendOp: D3D11_BLEND_OP_ADD,
                        SrcBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
                        DestBlendAlpha: D3D11_BLEND_ZERO,
                        BlendOpAlpha: D3D11_BLEND_OP_ADD,
                        RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as _,
                    },
                    std::mem::zeroed(),
                    std::mem::zeroed(),
                    std::mem::zeroed(),
                    std::mem::zeroed(),
                    std::mem::zeroed(),
                    std::mem::zeroed(),
                    std::mem::zeroed(),
                ],
            } as *const _)?
        };

        let rasterizer_state = unsafe {
            dasc.dev().CreateRasterizerState(&D3D11_RASTERIZER_DESC {
                FillMode: D3D11_FILL_SOLID,
                CullMode: D3D11_CULL_NONE,
                ScissorEnable: BOOL(1),
                DepthClipEnable: BOOL(1),
                DepthBias: 0,
                DepthBiasClamp: 0.,
                SlopeScaledDepthBias: 0.,
                MultisampleEnable: BOOL(0),
                AntialiasedLineEnable: BOOL(0),
                FrontCounterClockwise: BOOL(0),
            })?
        };

        let depth_stencil_state = unsafe {
            dasc.dev().CreateDepthStencilState(&D3D11_DEPTH_STENCIL_DESC {
                DepthEnable: BOOL(0),
                DepthFunc: D3D11_COMPARISON_ALWAYS,
                DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL,
                StencilEnable: BOOL(0),
                StencilReadMask: 0,
                StencilWriteMask: 0,
                FrontFace: D3D11_DEPTH_STENCILOP_DESC {
                    StencilFailOp: D3D11_STENCIL_OP_KEEP,
                    StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
                    StencilPassOp: D3D11_STENCIL_OP_KEEP,
                    StencilFunc: D3D11_COMPARISON_ALWAYS,
                },
                BackFace: D3D11_DEPTH_STENCILOP_DESC {
                    StencilFailOp: D3D11_STENCIL_OP_KEEP,
                    StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
                    StencilPassOp: D3D11_STENCIL_OP_KEEP,
                    StencilFunc: D3D11_COMPARISON_ALWAYS,
                },
            })?
        };

        Ok(ShaderProgram {
            vtx_shader,
            pix_shader,
            layout,
            sampler,
            blend_state,
            depth_stencil_state,
            rasterizer_state,
        })
    }

    unsafe fn set_state(&self, dasc: &DeviceAndSwapChain) {
        dasc.dev_ctx().VSSetShader(&self.vtx_shader, &[]);
        dasc.dev_ctx().PSSetShader(&self.pix_shader, &[]);
        dasc.dev_ctx().IASetInputLayout(&self.layout);
        dasc.dev_ctx().PSSetSamplers(0, &[Some(self.sampler.clone())]);
        dasc.dev_ctx().OMSetBlendState(&self.blend_state, &[0f32; 4] as _, 0xFFFFFFFF);
        dasc.dev_ctx().OMSetDepthStencilState(&self.depth_stencil_state, 0);
        dasc.dev_ctx().RSSetState(&self.rasterizer_state);
    }
}
