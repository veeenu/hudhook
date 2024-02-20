// NOTE: see this for ManuallyDrop instanceshttps://github.com/microsoft/windows-rs/issues/2386

use std::ffi::c_void;
use std::{mem, ptr, slice};

use imgui::internal::RawWrapper;
use imgui::{BackendFlags, Context, DrawCmd, DrawData, DrawIdx, DrawVert, TextureId};
use memoffset::offset_of;
use tracing::{error, trace};
use windows::core::{s, w, ComInterface, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::Fxc::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::keys;
use crate::util::{self, Fence};

pub struct RenderEngine {
    device: ID3D12Device,

    command_queue: ID3D12CommandQueue,
    command_allocator: ID3D12CommandAllocator,
    command_list: ID3D12GraphicsCommandList,

    rtv_heap: ID3D12DescriptorHeap,
    rtv_heap_start: D3D12_CPU_DESCRIPTOR_HANDLE,
    rtv_target: ID3D12Resource,

    texture_heap: TextureHeap,

    root_signature: ID3D12RootSignature,
    pipeline_state: ID3D12PipelineState,

    vertex_buffer: Buffer<DrawVert>,
    index_buffer: Buffer<u16>,
    projection_buffer: [[f32; 4]; 4],

    fence: Fence,
}

impl RenderEngine {
    pub fn new(ctx: &mut Context, width: u32, height: u32) -> Result<Self> {
        let dxgi_factory: IDXGIFactory2 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_DEBUG) }?;
        let dxgi_adapter = unsafe { dxgi_factory.EnumAdapters(0) }?;

        let device: ID3D12Device = util::try_out_ptr(|v| unsafe {
            D3D12CreateDevice(&dxgi_adapter, D3D_FEATURE_LEVEL_11_1, v)
        })?;

        let (command_queue, command_allocator, command_list) = unsafe {
            create_command_objects(&device, w!("hudhook Command Queue"), w!("hudhook Command List"))
        }?;

        let (rtv_heap, mut texture_heap) = unsafe { create_heaps(&device) }?;
        let rtv_heap_start = unsafe { rtv_heap.GetCPUDescriptorHandleForHeapStart() };
        let rtv_target = unsafe { create_render_target(&device, width, height) }?;

        let (root_signature, pipeline_state) = unsafe { create_shader_program(&device) }?;

        let vertex_buffer = Buffer::new(&device, 5000)?;
        let index_buffer = Buffer::new(&device, 10000)?;

        let fence = Fence::new(&device)?;

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
            command_queue,
            command_allocator,
            command_list,
            rtv_heap,
            rtv_heap_start,
            rtv_target,
            texture_heap,
            root_signature,
            pipeline_state,
            vertex_buffer,
            index_buffer,
            projection_buffer: Default::default(),
            fence,
        })
    }

    pub fn load_image(&mut self, data: &[u8], width: u32, height: u32) -> Result<TextureId> {
        unsafe { self.texture_heap.create_texture(data, width, height) }
    }

    pub fn render(
        &mut self,
        hwnd: HWND,
        ctx: &mut Context,
        draw_data: DrawData,
    ) -> Result<ID3D12Resource> {
        unsafe {
            self.render_setup(hwnd, ctx)?;

            self.command_allocator.Reset()?;
            self.command_list.Reset(&self.command_allocator, None)?;

            let present_to_rtv_barriers = [util::create_barrier(
                &self.rtv_target,
                D3D12_RESOURCE_STATE_PRESENT,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            )];

            let rtv_to_present_barriers = [util::create_barrier(
                &self.rtv_target,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_PRESENT,
            )];

            self.command_list.ResourceBarrier(&present_to_rtv_barriers);
            self.command_list.OMSetRenderTargets(1, Some(&self.rtv_heap_start), false, None);
            self.command_list.SetDescriptorHeaps(&[Some(self.texture_heap.srv_heap.clone())]);
            self.command_list.ClearRenderTargetView(self.rtv_heap_start, &[0.0; 4], None);

            self.render_draw_data(draw_data)?;

            self.command_list.ResourceBarrier(&rtv_to_present_barriers);
            self.command_queue.ExecuteCommandLists(&[Some(self.command_list.cast()?)]);
            self.command_queue.Signal(self.fence.fence(), self.fence.value())?;
            self.fence.wait()?;
            self.fence.incr();

            present_to_rtv_barriers.into_iter().for_each(util::drop_barrier);
            rtv_to_present_barriers.into_iter().for_each(util::drop_barrier);
        }

        Ok(self.rtv_target.clone())
    }

    unsafe fn render_setup(&mut self, hwnd: HWND, ctx: &mut Context) -> Result<()> {
        let desc = self.rtv_target.GetDesc();
        let (width, height) = util::win_size(hwnd);
        let (width, height) = (width as u32, height as u32);

        let io = ctx.io_mut();

        if width as u64 != desc.Width || height != desc.Height {
            self.resize(width, height)?;
            io.display_size = [width as f32, height as f32];
        }

        let active_window = GetForegroundWindow();
        if active_window == hwnd
            || (!HANDLE(active_window.0).is_invalid() && IsChild(active_window, hwnd).as_bool())
        {
            let mut pos = util::try_out_param(|v| GetCursorPos(v))?;
            if ScreenToClient(hwnd, &mut pos).as_bool() {
                io.mouse_pos = [pos.x as f32, pos.y as f32];
            }
        }

        io.nav_active = true;
        io.nav_visible = true;

        for (key, virtual_key) in keys::KEYS {
            io[key] = virtual_key.0 as u32;
        }

        Ok(())
    }

    unsafe fn render_draw_data(&mut self, draw_data: DrawData) -> Result<()> {
        if draw_data.display_size[0] <= 0f32 || draw_data.display_size[1] <= 0f32 {
            trace!(
                "Insufficent display size {}x{}, skip rendering",
                draw_data.display_size[0],
                draw_data.display_size[1]
            );
            return Ok(());
        }

        self.vertex_buffer.clear();
        self.index_buffer.clear();

        draw_data
            .draw_lists()
            .map(|draw_list| {
                (draw_list.vtx_buffer().iter().copied(), draw_list.idx_buffer().iter().copied())
            })
            .for_each(|(vertices, indices)| {
                self.vertex_buffer.extend(vertices);
                self.index_buffer.extend(indices);
            });

        self.vertex_buffer.upload(&self.device)?;
        self.index_buffer.upload(&self.device)?;
        self.projection_buffer = {
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
        };

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
                            let tex_handle = D3D12_GPU_DESCRIPTOR_HANDLE {
                                ptr: cmd_params.texture_id.id() as _,
                            };
                            unsafe {
                                self.command_list.SetGraphicsRootDescriptorTable(1, tex_handle);
                                self.command_list.RSSetScissorRects(&[r]);
                                self.command_list.DrawIndexedInstanced(
                                    count as _,
                                    1,
                                    (cmd_params.idx_offset + idx_offset) as _,
                                    (cmd_params.vtx_offset + vtx_offset) as _,
                                    0,
                                );
                            }
                        }
                    },
                    DrawCmd::ResetRenderState => {
                        self.setup_render_state(&draw_data);
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
        self.command_list.RSSetViewports(&[D3D12_VIEWPORT {
            TopLeftX: 0f32,
            TopLeftY: 0f32,
            Width: draw_data.display_size[0],
            Height: draw_data.display_size[1],
            MinDepth: 0f32,
            MaxDepth: 1f32,
        }]);

        self.command_list.IASetVertexBuffers(
            0,
            Some(&[D3D12_VERTEX_BUFFER_VIEW {
                BufferLocation: self.vertex_buffer.resource.GetGPUVirtualAddress(),
                SizeInBytes: (self.vertex_buffer.data.len() * mem::size_of::<DrawVert>()) as _,
                StrideInBytes: mem::size_of::<DrawVert>() as _,
            }]),
        );

        self.command_list.IASetIndexBuffer(Some(&D3D12_INDEX_BUFFER_VIEW {
            BufferLocation: self.index_buffer.resource.GetGPUVirtualAddress(),
            SizeInBytes: (self.index_buffer.data.len() * mem::size_of::<DrawIdx>()) as _,
            Format: if mem::size_of::<DrawIdx>() == 2 {
                DXGI_FORMAT_R16_UINT
            } else {
                DXGI_FORMAT_R32_UINT
            },
        }));
        self.command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        self.command_list.SetPipelineState(&self.pipeline_state);
        self.command_list.SetGraphicsRootSignature(&self.root_signature);
        self.command_list.SetGraphicsRoot32BitConstants(
            0,
            16,
            self.projection_buffer.as_ptr() as *const c_void,
            0,
        );
        self.command_list.OMSetBlendFactor(Some(&[0f32; 4]));
    }

    unsafe fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        drop(mem::replace(
            &mut self.rtv_target,
            create_render_target(&self.device, width, height)?,
        ));

        Ok(())
    }
}

unsafe fn create_command_objects(
    device: &ID3D12Device,
    command_queue_name: PCWSTR,
    command_list_name: PCWSTR,
) -> Result<(ID3D12CommandQueue, ID3D12CommandAllocator, ID3D12GraphicsCommandList)> {
    let command_queue: ID3D12CommandQueue =
        device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            Priority: 0,
            Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
            NodeMask: 0,
        })?;
    command_queue.SetName(command_queue_name)?;

    let command_allocator: ID3D12CommandAllocator =
        device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)?;

    let command_list: ID3D12GraphicsCommandList =
        device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &command_allocator, None)?;
    command_list.Close().unwrap();
    command_list.SetName(command_list_name)?;

    Ok((command_queue, command_allocator, command_list))
}

unsafe fn create_render_target(
    device: &ID3D12Device,
    width: u32,
    height: u32,
) -> Result<ID3D12Resource> {
    util::try_out_ptr(|v| {
        device.CreateCommittedResource(
            &D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                CreationNodeMask: 0,
                VisibleNodeMask: 0,
            },
            D3D12_HEAP_FLAG_SHARED,
            &D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                Alignment: 65536,
                Width: width as u64,
                Height: height,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
                Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
            },
            D3D12_RESOURCE_STATE_RENDER_TARGET,
            Some(&D3D12_CLEAR_VALUE {
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Anonymous: D3D12_CLEAR_VALUE_0 { Color: [0.0, 0.0, 0.0, 0.0] },
            }),
            v,
        )
    })
}

unsafe fn create_heaps(device: &ID3D12Device) -> Result<(ID3D12DescriptorHeap, TextureHeap)> {
    let rtv_heap: ID3D12DescriptorHeap =
        device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
            Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
            NumDescriptors: 1,
            Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
            NodeMask: 1,
        })?;

    let srv_heap: ID3D12DescriptorHeap =
        device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
            Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            NumDescriptors: 8,
            Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
            NodeMask: 0,
        })?;

    let texture_heap = TextureHeap::new(device, srv_heap)?;

    Ok((rtv_heap, texture_heap))
}

unsafe fn create_shader_program(
    device: &ID3D12Device,
) -> Result<(ID3D12RootSignature, ID3D12PipelineState)> {
    let root_signature_desc = D3D12_ROOT_SIGNATURE_DESC {
        NumParameters: 2,
        pParameters: [
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Constants: D3D12_ROOT_CONSTANTS {
                        ShaderRegister: 0,
                        RegisterSpace: 0,
                        Num32BitValues: 16,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_VERTEX,
            },
            D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                        NumDescriptorRanges: 1,
                        pDescriptorRanges: &D3D12_DESCRIPTOR_RANGE {
                            RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                            NumDescriptors: 1,
                            BaseShaderRegister: 0,
                            RegisterSpace: 0,
                            OffsetInDescriptorsFromTableStart: 0,
                        },
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
            },
        ]
        .as_ptr(),
        NumStaticSamplers: 1,
        pStaticSamplers: &D3D12_STATIC_SAMPLER_DESC {
            Filter: D3D12_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
            AddressV: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
            AddressW: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
            MipLODBias: 0f32,
            MaxAnisotropy: 0,
            ComparisonFunc: D3D12_COMPARISON_FUNC_ALWAYS,
            BorderColor: D3D12_STATIC_BORDER_COLOR_TRANSPARENT_BLACK,
            MinLOD: 0f32,
            MaxLOD: 0f32,
            ShaderRegister: 0,
            RegisterSpace: 0,
            ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
        },
        Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT
            | D3D12_ROOT_SIGNATURE_FLAG_DENY_HULL_SHADER_ROOT_ACCESS
            | D3D12_ROOT_SIGNATURE_FLAG_DENY_DOMAIN_SHADER_ROOT_ACCESS
            | D3D12_ROOT_SIGNATURE_FLAG_DENY_GEOMETRY_SHADER_ROOT_ACCESS,
    };
    let mut blob: Option<ID3DBlob> = None;
    let mut err_blob: Option<ID3DBlob> = None;
    if let Err(e) = D3D12SerializeRootSignature(
        &root_signature_desc,
        D3D_ROOT_SIGNATURE_VERSION_1_0,
        &mut blob,
        Some(&mut err_blob),
    ) {
        if let Some(err_blob) = err_blob {
            let buf_ptr = err_blob.GetBufferPointer() as *mut u8;
            let buf_size = err_blob.GetBufferSize();
            let s = String::from_raw_parts(buf_ptr, buf_size, buf_size + 1);
            error!("Serializing root signature: {}: {}", e, s);
        }
        return Err(e);
    }

    let blob = blob.unwrap();
    let root_signature: ID3D12RootSignature = device.CreateRootSignature(
        0,
        slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize()),
    )?;

    let vs = r#"
            cbuffer vertexBuffer : register(b0) {
              float4x4 ProjectionMatrix;
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
              output.pos = mul( ProjectionMatrix, float4(input.pos.xy, 0.f, 1.f));
              output.col = input.col;
              output.uv = input.uv;
              return output;
            }"#;

    let vtx_shader: ID3DBlob = util::try_out_ptr(|v| unsafe {
        D3DCompile(
            vs.as_ptr() as _,
            vs.len(),
            None,
            None,
            None::<&ID3DInclude>,
            s!("main\0"),
            s!("vs_5_0\0"),
            0,
            0,
            v,
            None,
        )
    })
    .expect("D3DCompile vertex shader");

    let ps = r#"
            struct PS_INPUT {
              float4 pos: SV_POSITION;
              float4 col: COLOR0;
              float2 uv: TEXCOORD0;
            };

            SamplerState sampler0: register(s0);
            Texture2D texture0: register(t0);

            float4 main(PS_INPUT input): SV_Target {
              float4 out_col = input.col * texture0.Sample(sampler0, input.uv);
              return out_col;
            }"#;

    let pix_shader = util::try_out_ptr(|v| unsafe {
        D3DCompile(
            ps.as_ptr() as _,
            ps.len(),
            None,
            None,
            None::<&ID3DInclude>,
            s!("main\0"),
            s!("ps_5_0\0"),
            0,
            0,
            v,
            None,
        )
    })
    .expect("D3DCompile pixel shader");

    let pso_desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
        pRootSignature: mem::transmute_copy(&root_signature),
        NodeMask: 1,
        PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
        SampleMask: u32::MAX,
        NumRenderTargets: 1,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
        RTVFormats: [
            DXGI_FORMAT_B8G8R8A8_UNORM,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        ],
        DSVFormat: DXGI_FORMAT_D32_FLOAT,
        VS: D3D12_SHADER_BYTECODE {
            pShaderBytecode: unsafe { vtx_shader.GetBufferPointer() },
            BytecodeLength: unsafe { vtx_shader.GetBufferSize() },
        },
        PS: D3D12_SHADER_BYTECODE {
            pShaderBytecode: unsafe { pix_shader.GetBufferPointer() },
            BytecodeLength: unsafe { pix_shader.GetBufferSize() },
        },
        InputLayout: D3D12_INPUT_LAYOUT_DESC {
            pInputElementDescs: [
                D3D12_INPUT_ELEMENT_DESC {
                    SemanticName: s!("POSITION"),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R32G32_FLOAT,
                    InputSlot: 0,
                    AlignedByteOffset: offset_of!(DrawVert, pos) as u32,
                    InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
                D3D12_INPUT_ELEMENT_DESC {
                    SemanticName: s!("TEXCOORD"),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R32G32_FLOAT,
                    InputSlot: 0,
                    AlignedByteOffset: offset_of!(DrawVert, uv) as u32,
                    InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
                D3D12_INPUT_ELEMENT_DESC {
                    SemanticName: s!("COLOR"),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    InputSlot: 0,
                    AlignedByteOffset: offset_of!(DrawVert, col) as u32,
                    InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
            ]
            .as_ptr(),
            NumElements: 3,
        },
        BlendState: D3D12_BLEND_DESC {
            AlphaToCoverageEnable: false.into(),
            IndependentBlendEnable: false.into(),
            RenderTarget: [
                D3D12_RENDER_TARGET_BLEND_DESC {
                    BlendEnable: true.into(),
                    LogicOpEnable: false.into(),
                    SrcBlend: D3D12_BLEND_SRC_ALPHA,
                    DestBlend: D3D12_BLEND_INV_SRC_ALPHA,
                    BlendOp: D3D12_BLEND_OP_ADD,
                    SrcBlendAlpha: D3D12_BLEND_ONE,
                    DestBlendAlpha: D3D12_BLEND_INV_SRC_ALPHA,
                    BlendOpAlpha: D3D12_BLEND_OP_ADD,
                    LogicOp: Default::default(),
                    RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as _,
                },
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
            ],
        },
        RasterizerState: D3D12_RASTERIZER_DESC {
            FillMode: D3D12_FILL_MODE_SOLID,
            CullMode: D3D12_CULL_MODE_NONE,
            FrontCounterClockwise: false.into(),
            DepthBias: D3D12_DEFAULT_DEPTH_BIAS,
            DepthBiasClamp: D3D12_DEFAULT_DEPTH_BIAS_CLAMP,
            SlopeScaledDepthBias: D3D12_DEFAULT_SLOPE_SCALED_DEPTH_BIAS,
            DepthClipEnable: true.into(),
            MultisampleEnable: false.into(),
            AntialiasedLineEnable: false.into(),
            ForcedSampleCount: 0,
            ConservativeRaster: D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
        },
        ..Default::default()
    };

    let pipeline_state = unsafe { device.CreateGraphicsPipelineState(&pso_desc)? };

    Ok((root_signature, pipeline_state))
}

struct Buffer<T: Sized> {
    resource: ID3D12Resource,
    resource_capacity: usize,
    data: Vec<T>,
}

impl<T> Buffer<T> {
    fn new(device: &ID3D12Device, resource_capacity: usize) -> Result<Self> {
        let resource = Self::create_resource(device, resource_capacity)?;
        let data = Vec::with_capacity(resource_capacity);

        Ok(Self { resource, resource_capacity, data })
    }

    fn create_resource(device: &ID3D12Device, resource_capacity: usize) -> Result<ID3D12Resource> {
        util::try_out_ptr(|v| unsafe {
            device.CreateCommittedResource(
                &D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_UPLOAD,
                    CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                    MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                    CreationNodeMask: 0,
                    VisibleNodeMask: 0,
                },
                D3D12_HEAP_FLAG_NONE,
                &D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                    Alignment: 0,
                    Width: (resource_capacity * mem::size_of::<T>()) as u64,
                    Height: 1,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: DXGI_FORMAT_UNKNOWN,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                    Flags: D3D12_RESOURCE_FLAG_NONE,
                },
                D3D12_RESOURCE_STATE_GENERIC_READ,
                None,
                v,
            )
        })
    }

    fn clear(&mut self) {
        self.data.clear();
    }

    fn extend<I: IntoIterator<Item = T>>(&mut self, it: I) {
        self.data.extend(it)
    }

    fn upload(&mut self, device: &ID3D12Device) -> Result<()> {
        let capacity = self.data.capacity();
        if capacity > self.resource_capacity {
            drop(mem::replace(&mut self.resource, Self::create_resource(device, capacity)?));
        }

        unsafe {
            let mut resource_ptr = ptr::null_mut();
            self.resource.Map(0, None, Some(&mut resource_ptr))?;
            ptr::copy_nonoverlapping(self.data.as_ptr(), resource_ptr as *mut T, self.data.len());
            self.resource.Unmap(0, None);
        }

        Ok(())
    }

    fn resource(&self) -> &ID3D12Resource {
        &self.resource
    }
}

struct Texture {
    resource: ID3D12Resource,
    id: TextureId,
}

struct TextureHeap {
    device: ID3D12Device,
    srv_heap: ID3D12DescriptorHeap,
    textures: Vec<Texture>,
    command_queue: ID3D12CommandQueue,
    command_allocator: ID3D12CommandAllocator,
    command_list: ID3D12GraphicsCommandList,
    fence: Fence,
}

impl TextureHeap {
    fn new(device: &ID3D12Device, srv_heap: ID3D12DescriptorHeap) -> Result<Self> {
        let (command_queue, command_allocator, command_list) = unsafe {
            create_command_objects(
                device,
                w!("hudhook Texture Heap Command Queue"),
                w!("hudhook Texture Heap Command List"),
            )
        }?;

        let fence = Fence::new(device)?;

        Ok(Self {
            device: device.clone(),
            srv_heap,
            textures: Vec::new(),
            command_queue,
            command_allocator,
            command_list,
            fence,
        })
    }

    unsafe fn resize_heap(&mut self) -> Result<()> {
        let mut desc = self.srv_heap.GetDesc();
        let old_num_descriptors = desc.NumDescriptors;

        if old_num_descriptors <= self.textures.len() as _ {
            desc.NumDescriptors *= 2;

            let srv_heap: ID3D12DescriptorHeap = self.device.CreateDescriptorHeap(&desc)?;
            self.device.CopyDescriptorsSimple(
                old_num_descriptors,
                srv_heap.GetCPUDescriptorHandleForHeapStart(),
                self.srv_heap.GetCPUDescriptorHandleForHeapStart(),
                D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            );
            self.srv_heap = srv_heap;
        }

        Ok(())
    }

    unsafe fn create_texture(&mut self, data: &[u8], width: u32, height: u32) -> Result<TextureId> {
        self.resize_heap()?;

        let cpu_heap_start = self.srv_heap.GetCPUDescriptorHandleForHeapStart();
        let gpu_heap_start = self.srv_heap.GetGPUDescriptorHandleForHeapStart();
        let heap_inc_size =
            self.device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV);

        let texture_index = self.textures.len() as u32;

        let cpu_desc = D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: cpu_heap_start.ptr + (texture_index * heap_inc_size) as usize,
        };

        let gpu_desc = D3D12_GPU_DESCRIPTOR_HANDLE {
            ptr: gpu_heap_start.ptr + (texture_index * heap_inc_size) as u64,
        };

        let texture: ID3D12Resource = util::try_out_ptr(|v| unsafe {
            self.device.CreateCommittedResource(
                &D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_DEFAULT,
                    CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                    MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                    CreationNodeMask: Default::default(),
                    VisibleNodeMask: Default::default(),
                },
                D3D12_HEAP_FLAG_NONE,
                &D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                    Alignment: 0,
                    Width: width as _,
                    Height: height as _,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
                    Flags: D3D12_RESOURCE_FLAG_NONE,
                },
                D3D12_RESOURCE_STATE_COPY_DEST,
                None,
                v,
            )
        })?;

        let upload_pitch = width * 4;
        let upload_size = height * upload_pitch;
        let upload_buffer: ID3D12Resource = util::try_out_ptr(|v| unsafe {
            self.device.CreateCommittedResource(
                &D3D12_HEAP_PROPERTIES {
                    Type: D3D12_HEAP_TYPE_UPLOAD,
                    CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                    MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                    CreationNodeMask: Default::default(),
                    VisibleNodeMask: Default::default(),
                },
                D3D12_HEAP_FLAG_NONE,
                &D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                    Alignment: 0,
                    Width: upload_size as _,
                    Height: 1,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: DXGI_FORMAT_UNKNOWN,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                    Flags: D3D12_RESOURCE_FLAG_NONE,
                },
                D3D12_RESOURCE_STATE_GENERIC_READ,
                None,
                v,
            )
        })?;

        let mut upload_buffer_ptr = ptr::null_mut();
        upload_buffer.Map(0, None, Some(&mut upload_buffer_ptr))?;
        ptr::copy_nonoverlapping(data.as_ptr(), upload_buffer_ptr as *mut u8, data.len());
        upload_buffer.Unmap(0, None);

        self.command_allocator.Reset()?;
        self.command_list.Reset(&self.command_allocator, None)?;
        self.command_list.CopyTextureRegion(
            &D3D12_TEXTURE_COPY_LOCATION {
                pResource: mem::transmute_copy(&texture),
                Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
            },
            0,
            0,
            0,
            &D3D12_TEXTURE_COPY_LOCATION {
                pResource: mem::transmute_copy(&upload_buffer),
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                        Offset: 0,
                        Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                            Width: width,
                            Height: height,
                            Depth: 1,
                            RowPitch: upload_pitch,
                        },
                    },
                },
            },
            None,
        );
        let barriers = [util::create_barrier(
            &texture,
            D3D12_RESOURCE_STATE_COPY_DEST,
            D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
        )];

        self.command_list.ResourceBarrier(&barriers);
        self.command_list.Close()?;
        self.command_queue.ExecuteCommandLists(&[Some(self.command_list.cast()?)]);
        self.command_queue.Signal(self.fence.fence(), self.fence.value())?;
        self.fence.wait()?;

        barriers.into_iter().for_each(util::drop_barrier);

        self.device.CreateShaderResourceView(
            &texture,
            Some(&D3D12_SHADER_RESOURCE_VIEW_DESC {
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                    Texture2D: D3D12_TEX2D_SRV {
                        MostDetailedMip: 0,
                        MipLevels: 1,
                        PlaneSlice: Default::default(),
                        ResourceMinLODClamp: Default::default(),
                    },
                },
            }),
            cpu_desc,
        );

        let texture_id = TextureId::from(gpu_desc.ptr as usize);
        self.textures.push(Texture { resource: texture, id: texture_id });

        Ok(texture_id)
    }
}
