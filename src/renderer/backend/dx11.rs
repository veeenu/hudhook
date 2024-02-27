use std::ffi::c_void;
use std::{mem, ptr, slice};

use imgui::internal::RawWrapper;
use imgui::{BackendFlags, Context, DrawCmd, DrawData, DrawIdx, DrawVert, TextureId};
use memoffset::offset_of;
use tracing::debug;
use windows::core::{s, Interface, Result};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;

use crate::renderer::RenderEngine;
use crate::util;

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
        let mut texture_heap = TextureHeap::new(&device)?;

        ctx.set_ini_filename(None);
        ctx.io_mut().backend_flags |= BackendFlags::RENDERER_HAS_VTX_OFFSET;
        ctx.set_renderer_name(String::from(concat!("imgui-dx12@", env!("CARGO_PKG_VERSION"))));
        let fonts = ctx.fonts();
        let fonts_texture = fonts.build_rgba32_texture();
        fonts.tex_id = unsafe {
            texture_heap.create_texture(
                fonts_texture.data,
                fonts_texture.width,
                fonts_texture.height,
            )
        }?;

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

impl RenderEngine for D3D11RenderEngine {
    type RenderTarget = ID3D11Texture2D;

    fn load_image(&mut self, data: &[u8], width: u32, height: u32) -> Result<imgui::TextureId> {
        unsafe { self.texture_heap.create_texture(data, width, height) }
    }

    fn render(
        &mut self,
        draw_data: &imgui::DrawData,
        render_target: Self::RenderTarget,
    ) -> Result<()> {
        // For the time being, state backup/restore is disabled as it leads to some
        // unintelligible crashes. So far I have not found instances where this
        // tampers with the underlying game's render loop, which means more
        // computation saved.

        // let state_backup = unsafe { StateBackup::backup(&self.device_context) };

        let render_target: ID3D11RenderTargetView = util::try_out_ptr(|v| unsafe {
            self.device.CreateRenderTargetView(&render_target, None, Some(v))
        })?;

        unsafe { self.device_context.OMSetRenderTargets(Some(&[Some(render_target)]), None) };

        unsafe { self.render_draw_data(draw_data) }?;

        // unsafe { state_backup.restore(&self.device_context) };

        Ok(())
    }
}

impl D3D11RenderEngine {
    unsafe fn render_draw_data(&mut self, draw_data: &DrawData) -> Result<()> {
        if draw_data.display_size[0] <= 0f32 || draw_data.display_size[1] <= 0f32 {
            debug!(
                "Insufficent display size {}x{}, skip rendering",
                draw_data.display_size[0], draw_data.display_size[1]
            );
            return Ok(());
        }

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

        #[rustfmt::skip]
        self.projection_buffer.push({
            let [l, t, r, b] = [
                draw_data.display_pos[0],
                draw_data.display_pos[1],
                draw_data.display_pos[0] + draw_data.display_size[0],
                draw_data.display_pos[1] + draw_data.display_size[1],
            ];

            [
                [2. / (r - l), 0., 0., 0.],
                [0., 2. / (t - b), 0., 0.],
                [0., 0., 0.5, 0.],
                [(r + l) / (l - r), (t + b) / (b - t), 0.5, 1.0],
            ]
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
                            let srv = ID3D11ShaderResourceView::from_raw(
                                cmd_params.texture_id.id() as *mut c_void,
                            );
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

// const BACKUP_OBJECT_COUNT: usize = 16;
//
// #[derive(Default)]
// struct StateBackup {
//     scissor: (u32, [RECT; BACKUP_OBJECT_COUNT]),
//     viewports: (u32, [D3D11_VIEWPORT; BACKUP_OBJECT_COUNT]),
//
//     rasterizer_state: Option<ID3D11RasterizerState>,
//
//     blend_state: Option<ID3D11BlendState>,
//     blend_factor: [f32; 4],
//     sample_mask: u32,
//
//     depth_stencil_state: Option<ID3D11DepthStencilState>,
//     depth_stencil_ref: u32,
//
//     shader_resources: [Option<ID3D11ShaderResourceView>; 1],
//     sampler: [Option<ID3D11SamplerState>; 1],
//
//     vertex_shader: Option<ID3D11VertexShader>,
//     pixel_shader: Option<ID3D11PixelShader>,
//
//     vs_instances: Option<ID3D11ClassInstance>,
//     vs_instances_count: u32,
//     ps_instances: Option<ID3D11ClassInstance>,
//     ps_instances_count: u32,
//
//     vertex_buffer: Option<ID3D11Buffer>,
//     vertex_buffer_stride: u32,
//     vertex_buffer_offset: u32,
//     index_buffer: Option<ID3D11Buffer>,
//     index_buffer_offset: u32,
//     index_buffer_format: DXGI_FORMAT,
//     constant_buffer: [Option<ID3D11Buffer>; 1],
//
//     primitive_topology: D3D_PRIMITIVE_TOPOLOGY,
//     input_layout: Option<ID3D11InputLayout>,
// }
//
// impl StateBackup {
//     unsafe fn backup(device_context: &ID3D11DeviceContext) -> StateBackup {
//         let mut r = statebackup::default();
//         r.scissor.0 = BACKUP_OBJECT_COUNT as _;
//         r.viewports.0 = BACKUP_OBJECT_COUNT as _;
//
//         device_context.RSGetScissorRects(&mut r.scissor.0,
// Some(r.scissor.1.as_mut_ptr()));         device_context.RSGetViewports(&mut
// r.viewports.0, Some(r.viewports.1.as_mut_ptr()));         r.rasterizer_state
// = device_context.RSGetState().ok();         device_context.OMGetBlendState(
//             Some(&mut r.blend_state),
//             Some(&mut r.blend_factor),
//             Some(&mut r.sample_mask),
//         );
//         device_context.OMGetDepthStencilState(
//             Some(&mut r.depth_stencil_state),
//             Some(&mut r.depth_stencil_ref),
//         );
//         device_context.PSGetShaderResources(0, Some(&mut
// r.shader_resources));         device_context.PSGetSamplers(0, Some(&mut
// r.sampler));
//
//         r.vs_instances_count = 256;
//         device_context.VSGetShader(
//             &mut r.vertex_shader,
//             Some(&mut r.vs_instances),
//             Some(&mut r.vs_instances_count),
//         );
//         r.ps_instances_count = 256;
//         device_context.PSGetShader(
//             &mut r.pixel_shader,
//             Some(&mut r.ps_instances),
//             Some(&mut r.ps_instances_count),
//         );
//
//         device_context.IAGetVertexBuffers(
//             0,
//             1,
//             Some(&mut r.vertex_buffer),
//             Some(&mut r.vertex_buffer_stride),
//             Some(&mut r.vertex_buffer_offset),
//         );
//         device_context.IAGetIndexBuffer(
//             Some(&mut r.index_buffer),
//             Some(&mut r.index_buffer_format),
//             Some(&mut r.index_buffer_offset),
//         );
//         device_context.VSGetConstantBuffers(0, Some(&mut r.constant_buffer));
//         r.primitive_topology = device_context.IAGetPrimitiveTopology();
//         r.input_layout = device_context.IAGetInputLayout().ok();
//
//         r
//     }
//
//     unsafe fn restore(self, device_context: &ID3D11DeviceContext) {
//         device_context.RSSetScissorRects(Some(&self.scissor.1[..self.scissor.
// 0 as usize]));         device_context.RSSetViewports(Some(&self.viewports.1[.
// .self.viewports.0 as usize]));         device_context.RSSetState(self.
// rasterizer_state.as_ref());         device_context.OMSetBlendState(
//             self.blend_state.as_ref(),
//             Some(&self.blend_factor),
//             self.sample_mask,
//         );
//         device_context
//             .OMSetDepthStencilState(self.depth_stencil_state.as_ref(),
// self.depth_stencil_ref);         device_context.PSSetShaderResources(0,
// Some(&self.shader_resources));         device_context.VSSetConstantBuffers(0,
// Some(&self.constant_buffer));         device_context.
// IASetPrimitiveTopology(self.primitive_topology);         if let Some(il) =
// self.input_layout {             device_context.IASetInputLayout(&il)
//         };
//     }
// }

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
}

struct TextureHeap {
    device: ID3D11Device,
    textures: Vec<Texture>,
}

impl TextureHeap {
    fn new(device: &ID3D11Device) -> Result<Self> {
        Ok(Self { device: device.clone(), textures: Vec::with_capacity(8) })
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

        let id = TextureId::from(shader_resource_view.as_raw());

        let texture = Texture { resource, shader_resource_view, id };
        self.textures.push(texture);

        Ok(id)
    }
}
