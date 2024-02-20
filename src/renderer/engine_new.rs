// NOTE: see this for ManuallyDrop instanceshttps://github.com/microsoft/windows-rs/issues/2386

use std::mem::{self, ManuallyDrop};
use std::{ptr, slice};

use imgui::{BackendFlags, Context, DrawData, DrawVert, TextureId};
use memoffset::offset_of;

use tracing::error;
use windows::core::{s, w, ComInterface, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::Fxc::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::util::{try_out_param, try_out_ptr, Barrier};

pub struct RenderEngine {
    device: ID3D12Device,

    command_queue: ID3D12CommandQueue,
    command_allocator: ID3D12CommandAllocator,
    command_list: ID3D12GraphicsCommandList,

    rtv_heap: ID3D12DescriptorHeap,
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

        let device: ID3D12Device = try_out_ptr(|v| unsafe {
            D3D12CreateDevice(&dxgi_adapter, D3D_FEATURE_LEVEL_11_1, v)
        })?;

        let (command_queue, command_allocator, command_list) = unsafe {
            create_command_objects(&device, w!("hudhook Command Queue"), w!("hudhook Command List"))
        }?;

        let (rtv_heap, mut texture_heap) = unsafe { create_heaps(&device) }?;
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

    pub fn render(&mut self, draw_data: DrawData) -> Result<()> {
        todo!();
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
    try_out_ptr(|v| {
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

    let vtx_shader: ID3DBlob = try_out_ptr(|v| unsafe {
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

    let pix_shader = try_out_ptr(|v| unsafe {
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
        try_out_ptr(|v| unsafe {
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

    fn extend<I: Iterator<Item = T>>(&mut self, it: I) {
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

        let texture: ID3D12Resource = try_out_ptr(|v| unsafe {
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
        let upload_buffer: ID3D12Resource = try_out_ptr(|v| unsafe {
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
        self.command_list.ResourceBarrier(&[D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                    pResource: mem::transmute_copy(&texture),
                    Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                    StateBefore: D3D12_RESOURCE_STATE_COPY_DEST,
                    StateAfter: D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                }),
            },
        }]);
        self.command_list.Close()?;
        self.command_queue.ExecuteCommandLists(&[Some(self.command_list.cast()?)]);
        self.command_queue.Signal(&self.fence.fence, self.fence.value)?;
        self.fence.wait()?;

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

struct Fence {
    fence: ID3D12Fence,
    value: u64,
    event: HANDLE,
}

impl Fence {
    fn new(device: &ID3D12Device) -> Result<Self> {
        let fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }?;
        let value = 0;
        let event = unsafe { CreateEventW(None, false, false, None) }?;

        Ok(Fence { fence, value, event })
    }

    fn wait(&mut self) -> Result<()> {
        unsafe {
            if self.fence.GetCompletedValue() < self.value {
                self.fence.SetEventOnCompletion(self.value, self.event)?;
                WaitForSingleObjectEx(self.event, u32::MAX, false);
            }
        }
        self.value += 1;

        Ok(())
    }
}
