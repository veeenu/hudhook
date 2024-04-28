use std::ffi::c_void;
use std::{mem, ptr, slice};

use imgui::internal::RawWrapper;
use imgui::{BackendFlags, Context, DrawCmd, DrawData, DrawIdx, DrawVert, TextureId};
use memoffset::offset_of;
use tracing::error;
use windows::core::{s, Error, Result, HRESULT};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;

use crate::renderer::RenderEngine;
use crate::{util, RenderContext};

pub struct D3D11RenderEngine {
    device: ID3D11Device,
    device_context: ID3D11DeviceContext,

    shader_program: ShaderProgram,
    texture_heap: TextureHeap,

    vertex_buffer: Buffer<DrawVert>,
    index_buffer: Buffer<DrawIdx>,
    projection_buffer: Buffer<[[f32; 4]; 4]>,
}

impl D3D11RenderEngine {
    pub fn new(device: &ID3D11Device, ctx: &mut Context) -> Result<Self> {
        let device = device.clone();
        let device_context = unsafe { device.GetImmediateContext() }?;

        let vertex_buffer = Buffer::new(&device, 5000, D3D11_BIND_VERTEX_BUFFER)?;
        let index_buffer = Buffer::new(&device, 10000, D3D11_BIND_INDEX_BUFFER)?;
        let projection_buffer = Buffer::new(&device, 1, D3D11_BIND_CONSTANT_BUFFER)?;

        let shader_program = ShaderProgram::new(&device)?;
        let texture_heap = TextureHeap::new(&device, &device_context)?;

        ctx.set_ini_filename(None);
        ctx.io_mut().backend_flags |= BackendFlags::RENDERER_HAS_VTX_OFFSET;
        ctx.set_renderer_name(String::from(concat!("hudhook-dx11@", env!("CARGO_PKG_VERSION"))));

        Ok(Self {
            device,
            device_context,
            shader_program,
            texture_heap,
            vertex_buffer,
            index_buffer,
            projection_buffer,
        })
    }
}

impl RenderContext for D3D11RenderEngine {
    fn load_texture(&mut self, data: &[u8], width: u32, height: u32) -> Result<TextureId> {
        unsafe { self.texture_heap.create_texture(data, width, height) }
    }

    fn replace_texture(
        &mut self,
        texture_id: TextureId,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<()> {
        unsafe { self.texture_heap.update_texture(texture_id, data, width, height) }
    }
}

impl RenderEngine for D3D11RenderEngine {
    type RenderTarget = ID3D11Texture2D;

    fn render(
        &mut self,
        draw_data: &imgui::DrawData,
        render_target: Self::RenderTarget,
    ) -> Result<()> {
        unsafe {
            let state_backup = StateBackup::backup(&self.device_context);

            let render_target: ID3D11RenderTargetView = util::try_out_ptr(|v| {
                self.device.CreateRenderTargetView(&render_target, None, Some(v))
            })?;

            self.device_context.OMSetRenderTargets(Some(&[Some(render_target)]), None);
            self.render_draw_data(draw_data)?;
            state_backup.restore(&self.device_context);
        };

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

impl D3D11RenderEngine {
    unsafe fn render_draw_data(&mut self, draw_data: &DrawData) -> Result<()> {
        self.vertex_buffer.clear();
        self.index_buffer.clear();
        self.projection_buffer.clear();

        draw_data
            .draw_lists()
            .map(|draw_list| {
                (draw_list.vtx_buffer().iter().copied(), draw_list.idx_buffer().iter().copied())
            })
            .for_each(|(vertices, indices)| {
                self.vertex_buffer.extend(vertices);
                self.index_buffer.extend(indices);
            });

        self.projection_buffer.push({
            let [l, t, r, b] = [
                draw_data.display_pos[0],
                draw_data.display_pos[1],
                draw_data.display_pos[0] + draw_data.display_size[0],
                draw_data.display_pos[1] + draw_data.display_size[1],
            ];

            [[2. / (r - l), 0., 0., 0.], [0., 2. / (t - b), 0., 0.], [0., 0., 0.5, 0.], [
                (r + l) / (l - r),
                (t + b) / (b - t),
                0.5,
                1.0,
            ]]
        });

        self.vertex_buffer.upload(&self.device, &self.device_context)?;
        self.index_buffer.upload(&self.device, &self.device_context)?;
        self.projection_buffer.upload(&self.device, &self.device_context)?;

        self.setup_render_state(draw_data);

        let mut vtx_offset = 0usize;
        let mut idx_offset = 0usize;

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

                        if r.right > r.left && r.bottom > r.top {
                            let srv = self.texture_heap.textures[cmd_params.texture_id.id()]
                                .shader_resource_view
                                .clone();
                            unsafe {
                                self.device_context.PSSetShaderResources(0, Some(&[Some(srv)]));
                                self.device_context.RSSetScissorRects(Some(&[r]));
                                self.device_context.DrawIndexed(
                                    count as _,
                                    (cmd_params.idx_offset + idx_offset) as _,
                                    (cmd_params.vtx_offset + vtx_offset) as _,
                                );
                            }
                        }
                    },
                    DrawCmd::ResetRenderState => {
                        // Q: looking at the commands recorded in here, it
                        // doesn't seem like this should have any effect
                        // whatsoever. What am I doing wrong?
                        self.setup_render_state(draw_data);
                    },
                    DrawCmd::RawCallback { callback, raw_cmd } => unsafe {
                        callback(cl.raw(), raw_cmd)
                    },
                }
            }
            idx_offset += cl.idx_buffer().len();
            vtx_offset += cl.vtx_buffer().len();
        }

        Ok(())
    }

    unsafe fn setup_render_state(&self, draw_data: &DrawData) {
        self.device_context.RSSetViewports(Some(&[D3D11_VIEWPORT {
            TopLeftX: 0f32,
            TopLeftY: 0f32,
            Width: draw_data.display_size[0],
            Height: draw_data.display_size[1],
            MinDepth: 0f32,
            MaxDepth: 1f32,
        }]));
        self.device_context.IASetInputLayout(&self.shader_program.input_layout);
        self.device_context.IASetVertexBuffers(
            0,
            1,
            Some(&Some(self.vertex_buffer.resource.clone())),
            Some(&(mem::size_of::<DrawVert>() as u32)),
            Some(&0),
        );
        self.device_context.IASetIndexBuffer(
            Some(&self.index_buffer.resource),
            if mem::size_of::<DrawIdx>() == 2 {
                DXGI_FORMAT_R16_UINT
            } else {
                DXGI_FORMAT_R32_UINT
            },
            0,
        );
        self.device_context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        self.device_context.VSSetShader(&self.shader_program.vertex_shader, Some(&[]));
        self.device_context
            .VSSetConstantBuffers(0, Some(&[Some(self.projection_buffer.resource.clone())]));
        self.device_context.PSSetShader(&self.shader_program.pixel_shader, Some(&[]));
        self.device_context
            .PSSetSamplers(0, Some(&[Some(self.shader_program.sampler_state.clone())]));
        self.device_context.OMSetBlendState(
            &self.shader_program.blend_state,
            Some(&[0.; 4]),
            0xffffffff,
        );
        self.device_context.OMSetDepthStencilState(&self.shader_program.depth_stencil_state, 0);
        self.device_context.RSSetState(&self.shader_program.rasterizer_state);
    }
}

struct ShaderProgram {
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    input_layout: ID3D11InputLayout,
    sampler_state: ID3D11SamplerState,
    blend_state: ID3D11BlendState,
    depth_stencil_state: ID3D11DepthStencilState,
    rasterizer_state: ID3D11RasterizerState,
}

impl ShaderProgram {
    fn new(device: &ID3D11Device) -> Result<Self> {
        const VERTEX_SHADER_SRC: &str = r"
        cbuffer vertex_buffer: register(b0) {
            float4x4 projection;
        };

        struct VS_INPUT {
          float2 pos: POSITION;
          float4 col: COLOR0;
          float2 uv: TEXCOORD0;
        };

        struct PS_INPUT {
          float4 pos: SV_POSITION;
          float4 col: COLOR0;
          float2 uv: TEXCOORD0;
        };

        PS_INPUT main(VS_INPUT input) {
          PS_INPUT output;
          output.pos = mul(projection, float4(input.pos.xy, 0.0f, 1.0f));
          output.col = input.col;
          output.uv = input.uv.xy;
          return output;
        }
        ";

        const PIXEL_SHADER_SRC: &str = r"
        struct PS_INPUT {
          float4 pos: SV_POSITION;
          float4 col: COLOR0;
          float2 uv: TEXCOORD0;
        };

        Texture2D texture0: register(t0);
        SamplerState sampler0: register(s0);

        float4 main(PS_INPUT input): SV_Target {
          float4 col = input.col * texture0.Sample(sampler0, input.uv);
          return col;
        }
        ";

        let vs_blob: ID3DBlob = util::try_out_err_blob(|v, err_blob| unsafe {
            D3DCompile(
                VERTEX_SHADER_SRC.as_ptr() as _,
                VERTEX_SHADER_SRC.len(),
                None,
                None,
                None,
                s!("main\0"),
                s!("vs_4_0\0"),
                0,
                0,
                v,
                Some(err_blob),
            )
        })
        .map_err(util::print_error_blob("Compiling vertex shader"))
        .expect("D3DCompile");

        let ps_blob = util::try_out_err_blob(|v, err_blob| unsafe {
            D3DCompile(
                PIXEL_SHADER_SRC.as_ptr() as _,
                PIXEL_SHADER_SRC.len(),
                None,
                None,
                None,
                s!("main\0"),
                s!("ps_4_0\0"),
                0,
                0,
                v,
                Some(err_blob),
            )
        })
        .map_err(util::print_error_blob("Compiling pixel shader"))
        .expect("D3DCompile");

        let vertex_shader = util::try_out_ptr(|v| unsafe {
            let ptr = vs_blob.GetBufferPointer();
            let size = vs_blob.GetBufferSize();
            device.CreateVertexShader(slice::from_raw_parts(ptr as _, size), None, Some(v))
        })?;

        let pixel_shader = util::try_out_ptr(|v| unsafe {
            let ptr = ps_blob.GetBufferPointer();
            let size = ps_blob.GetBufferSize();
            device.CreatePixelShader(slice::from_raw_parts(ptr as _, size), None, Some(v))
        })?;

        let input_layout = util::try_out_ptr(|v| unsafe {
            let ptr = vs_blob.GetBufferPointer();
            let size = vs_blob.GetBufferSize();
            device.CreateInputLayout(
                &[
                    D3D11_INPUT_ELEMENT_DESC {
                        SemanticName: s!("POSITION"),
                        SemanticIndex: 0,
                        Format: DXGI_FORMAT_R32G32_FLOAT,
                        InputSlot: 0,
                        AlignedByteOffset: offset_of!(DrawVert, pos) as u32,
                        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                        InstanceDataStepRate: 0,
                    },
                    D3D11_INPUT_ELEMENT_DESC {
                        SemanticName: s!("COLOR"),
                        SemanticIndex: 0,
                        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                        InputSlot: 0,
                        AlignedByteOffset: offset_of!(DrawVert, col) as u32,
                        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                        InstanceDataStepRate: 0,
                    },
                    D3D11_INPUT_ELEMENT_DESC {
                        SemanticName: s!("TEXCOORD"),
                        SemanticIndex: 0,
                        Format: DXGI_FORMAT_R32G32_FLOAT,
                        InputSlot: 0,
                        AlignedByteOffset: offset_of!(DrawVert, uv) as u32,
                        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                        InstanceDataStepRate: 0,
                    },
                ],
                slice::from_raw_parts(ptr as _, size),
                Some(v),
            )
        })?;

        let sampler_state = util::try_out_ptr(|v| unsafe {
            device.CreateSamplerState(
                &D3D11_SAMPLER_DESC {
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
                },
                Some(v),
            )
        })?;
        let blend_state = util::try_out_ptr(|v| unsafe {
            device.CreateBlendState(
                &D3D11_BLEND_DESC {
                    AlphaToCoverageEnable: false.into(),
                    IndependentBlendEnable: false.into(),
                    RenderTarget: [
                        D3D11_RENDER_TARGET_BLEND_DESC {
                            BlendEnable: true.into(),
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
                },
                Some(v),
            )
        })?;

        let rasterizer_state = util::try_out_ptr(|v| unsafe {
            device.CreateRasterizerState(
                &D3D11_RASTERIZER_DESC {
                    FillMode: D3D11_FILL_SOLID,
                    CullMode: D3D11_CULL_NONE,
                    ScissorEnable: true.into(),
                    DepthClipEnable: true.into(),
                    DepthBias: 0,
                    DepthBiasClamp: 0.,
                    SlopeScaledDepthBias: 0.,
                    MultisampleEnable: false.into(),
                    AntialiasedLineEnable: false.into(),
                    FrontCounterClockwise: false.into(),
                },
                Some(v),
            )
        })?;

        let depth_stencil_state = util::try_out_ptr(|v| unsafe {
            device.CreateDepthStencilState(
                &D3D11_DEPTH_STENCIL_DESC {
                    DepthEnable: false.into(),
                    DepthFunc: D3D11_COMPARISON_ALWAYS,
                    DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL,
                    StencilEnable: false.into(),
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
                },
                Some(v),
            )
        })?;

        Ok(ShaderProgram {
            vertex_shader,
            pixel_shader,
            input_layout,
            sampler_state,
            blend_state,
            depth_stencil_state,
            rasterizer_state,
        })
    }
}

struct Buffer<T: Sized> {
    bind_flag: D3D11_BIND_FLAG,
    resource: ID3D11Buffer,
    resource_capacity: usize,
    data: Vec<T>,
}

impl<T> Buffer<T> {
    fn new(
        device: &ID3D11Device,
        resource_capacity: usize,
        bind_flag: D3D11_BIND_FLAG,
    ) -> Result<Self> {
        let resource = Self::create_resource(device, resource_capacity, bind_flag)?;
        let data = Vec::with_capacity(resource_capacity);

        Ok(Self { bind_flag, resource, resource_capacity, data })
    }

    fn create_resource(
        device: &ID3D11Device,
        resource_capacity: usize,
        bind_flag: D3D11_BIND_FLAG,
    ) -> Result<ID3D11Buffer> {
        util::try_out_ptr(|v| unsafe {
            device.CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: (resource_capacity * mem::size_of::<T>()) as u32,
                    Usage: D3D11_USAGE_DYNAMIC,
                    BindFlags: bind_flag.0 as u32,
                    CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as _,
                    MiscFlags: 0,
                    StructureByteStride: 0,
                },
                None,
                Some(v),
            )
        })
    }

    fn clear(&mut self) {
        self.data.clear();
    }

    fn extend<I: IntoIterator<Item = T>>(&mut self, it: I) {
        self.data.extend(it)
    }

    fn push(&mut self, t: T) {
        self.data.push(t)
    }

    fn upload(
        &mut self,
        device: &ID3D11Device,
        device_context: &ID3D11DeviceContext,
    ) -> Result<()> {
        let capacity = self.data.capacity();
        if capacity > self.resource_capacity {
            drop(mem::replace(
                &mut self.resource,
                Self::create_resource(device, capacity, self.bind_flag)?,
            ));
            self.resource_capacity = capacity;
        }

        unsafe {
            let mut resource_ptr = Default::default();
            device_context.Map(
                &self.resource,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut resource_ptr),
            )?;
            ptr::copy_nonoverlapping(
                self.data.as_ptr(),
                resource_ptr.pData as *mut T,
                self.data.len(),
            );
            device_context.Unmap(&self.resource, 0);
        }

        Ok(())
    }
}

#[derive(Debug)]
#[allow(unused)]
struct Texture {
    resource: ID3D11Texture2D,
    shader_resource_view: ID3D11ShaderResourceView,
    id: TextureId,
    width: u32,
    height: u32,
}

struct TextureHeap {
    device: ID3D11Device,
    device_context: ID3D11DeviceContext,
    textures: Vec<Texture>,
}

impl TextureHeap {
    fn new(device: &ID3D11Device, device_context: &ID3D11DeviceContext) -> Result<Self> {
        Ok(Self {
            device: device.clone(),
            device_context: device_context.clone(),
            textures: Vec::with_capacity(8),
        })
    }

    unsafe fn create_texture(&mut self, data: &[u8], width: u32, height: u32) -> Result<TextureId> {
        let resource: ID3D11Texture2D = util::try_out_ptr(|v| {
            self.device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: width,
                    Height: height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                    CPUAccessFlags: 0,
                    MiscFlags: 0,
                },
                Some(&D3D11_SUBRESOURCE_DATA {
                    pSysMem: data.as_ptr() as *const c_void,
                    SysMemPitch: width * 4,
                    SysMemSlicePitch: 0,
                }),
                Some(v),
            )
        })?;

        let shader_resource_view = util::try_out_ptr(|v| {
            self.device.CreateShaderResourceView(
                &resource,
                Some(&D3D11_SHADER_RESOURCE_VIEW_DESC {
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    ViewDimension: D3D11_SRV_DIMENSION_TEXTURE2D,
                    Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
                        Texture2D: D3D11_TEX2D_SRV { MostDetailedMip: 0, MipLevels: 1 },
                    },
                }),
                Some(v),
            )
        })?;

        let id = TextureId::from(self.textures.len());
        self.textures.push(Texture { resource, shader_resource_view, id, width, height });

        Ok(id)
    }

    unsafe fn update_texture(
        &mut self,
        texture_id: TextureId,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<()> {
        let texture = &mut self.textures[texture_id.id()];
        if texture.width != width || texture.height != height {
            error!(
                "image size {width}x{height} do not match expected {}x{}",
                texture.width, texture.height
            );
            return Err(Error::from_hresult(HRESULT(-1)));
        }

        self.device_context.UpdateSubresource(
            &texture.resource,
            0,
            None,
            data.as_ptr() as *const c_void,
            width * 4,
            0,
        );

        Ok(())
    }
}

const BACKUP_OBJECT_COUNT: usize = 16;

struct StateBackup {
    scissor_count: u32,
    scissor_rects: [RECT; BACKUP_OBJECT_COUNT],
    viewport_count: u32,
    viewports: [D3D11_VIEWPORT; BACKUP_OBJECT_COUNT],

    rasterizer_state: Option<ID3D11RasterizerState>,

    blend_state: Option<ID3D11BlendState>,
    blend_factor: [f32; 4],
    sample_mask: u32,

    depth_stencil_state: Option<ID3D11DepthStencilState>,
    depth_stencil_ref: u32,

    shader_resources: [Option<ID3D11ShaderResourceView>; 1],
    sampler: [Option<ID3D11SamplerState>; 1],

    vertex_shader: Option<ID3D11VertexShader>,
    pixel_shader: Option<ID3D11PixelShader>,

    vs_instances: *mut Option<ID3D11ClassInstance>,
    vs_instances_count: u32,
    ps_instances: *mut Option<ID3D11ClassInstance>,
    ps_instances_count: u32,

    vertex_buffer: Option<ID3D11Buffer>,
    vertex_buffer_stride: u32,
    vertex_buffer_offset: u32,
    index_buffer: Option<ID3D11Buffer>,
    index_buffer_offset: u32,
    index_buffer_format: DXGI_FORMAT,
    constant_buffer: [Option<ID3D11Buffer>; BACKUP_OBJECT_COUNT],

    primitive_topology: D3D_PRIMITIVE_TOPOLOGY,
    input_layout: Option<ID3D11InputLayout>,
}

impl StateBackup {
    unsafe fn backup(device_context: &ID3D11DeviceContext) -> StateBackup {
        let mut scissor_count = 0;
        let mut scissor_rects: [RECT; BACKUP_OBJECT_COUNT] = Default::default();
        device_context.RSGetScissorRects(&mut scissor_count, None);
        device_context.RSGetScissorRects(&mut scissor_count, Some(scissor_rects.as_mut_ptr()));

        let mut viewport_count = 0;
        let mut viewports: [D3D11_VIEWPORT; BACKUP_OBJECT_COUNT] = Default::default();
        device_context.RSGetViewports(&mut viewport_count, None);
        device_context.RSGetViewports(&mut viewport_count, Some(viewports.as_mut_ptr()));

        let (mut blend_state, mut blend_factor, mut sample_mask) = Default::default();
        device_context.OMGetBlendState(
            Some(&mut blend_state),
            Some(&mut blend_factor),
            Some(&mut sample_mask),
        );

        let (mut depth_stencil_state, mut depth_stencil_ref) = Default::default();
        device_context
            .OMGetDepthStencilState(Some(&mut depth_stencil_state), Some(&mut depth_stencil_ref));

        let mut shader_resources: [Option<ID3D11ShaderResourceView>; 1] = Default::default();
        device_context.PSGetShaderResources(0, Some(&mut shader_resources));

        let mut sampler: [Option<ID3D11SamplerState>; 1] = Default::default();
        device_context.PSGetSamplers(0, Some(&mut sampler));

        let mut vertex_shader = Default::default();
        let vs_instances: *mut Option<ID3D11ClassInstance> = ptr::null_mut();
        let mut vs_instances_count = 256;
        device_context.VSGetShader(
            &mut vertex_shader,
            Some(vs_instances),
            Some(&mut vs_instances_count),
        );
        let mut pixel_shader = Default::default();
        let ps_instances: *mut Option<ID3D11ClassInstance> = ptr::null_mut();
        let mut ps_instances_count = 256;
        device_context.PSGetShader(
            &mut pixel_shader,
            Some(ps_instances),
            Some(&mut ps_instances_count),
        );

        let (mut vertex_buffer, mut vertex_buffer_stride, mut vertex_buffer_offset) =
            Default::default();
        device_context.IAGetVertexBuffers(
            0,
            1,
            Some(&mut vertex_buffer),
            Some(&mut vertex_buffer_stride),
            Some(&mut vertex_buffer_offset),
        );

        let (mut index_buffer, mut index_buffer_format, mut index_buffer_offset) =
            Default::default();
        device_context.IAGetIndexBuffer(
            Some(&mut index_buffer),
            Some(&mut index_buffer_format),
            Some(&mut index_buffer_offset),
        );

        let mut constant_buffer: [Option<ID3D11Buffer>; BACKUP_OBJECT_COUNT] = Default::default();
        device_context.VSGetConstantBuffers(0, Some(&mut constant_buffer));

        let rasterizer_state = device_context.RSGetState().ok();
        let primitive_topology = device_context.IAGetPrimitiveTopology();
        let input_layout = device_context.IAGetInputLayout().ok();

        Self {
            scissor_count,
            scissor_rects,
            viewport_count,
            viewports,
            rasterizer_state,
            blend_state,
            blend_factor,
            sample_mask,
            depth_stencil_state,
            depth_stencil_ref,
            shader_resources,
            sampler,
            vertex_shader,
            pixel_shader,
            vs_instances,
            vs_instances_count,
            ps_instances,
            ps_instances_count,
            vertex_buffer,
            vertex_buffer_stride,
            vertex_buffer_offset,
            index_buffer,
            index_buffer_offset,
            index_buffer_format,
            constant_buffer,
            primitive_topology,
            input_layout,
        }
    }

    unsafe fn restore(self, device_context: &ID3D11DeviceContext) {
        device_context.RSSetScissorRects(Some(&self.scissor_rects[..self.scissor_count as usize]));
        device_context.RSSetViewports(Some(&self.viewports[..self.viewport_count as usize]));

        device_context.OMSetBlendState(
            self.blend_state.as_ref(),
            Some(&self.blend_factor),
            self.sample_mask,
        );
        device_context
            .OMSetDepthStencilState(self.depth_stencil_state.as_ref(), self.depth_stencil_ref);

        if self.shader_resources[0].is_some() {
            device_context.PSSetShaderResources(0, Some(&self.shader_resources));
        }
        if self.sampler[0].is_some() {
            device_context.PSSetSamplers(0, Some(&self.sampler));
        }

        device_context.PSSetShader(
            self.pixel_shader.as_ref(),
            if self.ps_instances_count > 0 {
                Some(slice::from_raw_parts(self.ps_instances, self.ps_instances_count as usize))
            } else {
                None
            },
        );

        device_context.VSSetShader(
            self.vertex_shader.as_ref(),
            if self.vs_instances_count > 0 {
                Some(slice::from_raw_parts(self.vs_instances, self.vs_instances_count as usize))
            } else {
                None
            },
        );

        device_context.IASetVertexBuffers(
            0,
            1,
            Some(&self.vertex_buffer),
            Some(&self.vertex_buffer_stride),
            Some(&self.vertex_buffer_offset),
        );

        device_context.IASetIndexBuffer(
            self.index_buffer.as_ref(),
            self.index_buffer_format,
            self.index_buffer_offset,
        );

        if self.constant_buffer[0].is_some() {
            let count = self.constant_buffer.iter().take_while(|x| x.is_some()).count();
            device_context.VSSetConstantBuffers(0, Some(&self.constant_buffer[..count]));
        }

        device_context.RSSetState(self.rasterizer_state.as_ref());
        device_context.IASetPrimitiveTopology(self.primitive_topology);
        if let Some(il) = self.input_layout {
            device_context.IASetInputLayout(&il);
        }
    }
}
