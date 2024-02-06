use std::ffi::c_void;
use std::mem::{self, size_of, ManuallyDrop};
use std::ptr::{self, null_mut};

use memoffset::offset_of;
use tracing::error;
use windows::core::{s, w, ComInterface, Result};
use windows::Win32::Foundation::{BOOL, HANDLE, RECT};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, ID3DInclude, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D12::{
    D3D12SerializeRootSignature, ID3D12CommandAllocator, ID3D12CommandQueue, ID3D12DescriptorHeap,
    ID3D12Device, ID3D12Fence, ID3D12GraphicsCommandList, ID3D12PipelineState, ID3D12Resource,
    ID3D12RootSignature, D3D12_BLEND_INV_SRC_ALPHA, D3D12_BLEND_ONE, D3D12_BLEND_OP_ADD,
    D3D12_BLEND_SRC_ALPHA, D3D12_COLOR_WRITE_ENABLE_ALL, D3D12_COMMAND_LIST_TYPE_DIRECT,
    D3D12_COMPARISON_FUNC_ALWAYS, D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
    D3D12_CPU_PAGE_PROPERTY_UNKNOWN, D3D12_CULL_MODE_NONE, D3D12_DEFAULT_DEPTH_BIAS,
    D3D12_DEFAULT_DEPTH_BIAS_CLAMP, D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
    D3D12_DEFAULT_SLOPE_SCALED_DEPTH_BIAS, D3D12_DESCRIPTOR_HEAP_DESC,
    D3D12_DESCRIPTOR_HEAP_FLAG_NONE, D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
    D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, D3D12_DESCRIPTOR_HEAP_TYPE_RTV, D3D12_DESCRIPTOR_RANGE,
    D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND, D3D12_DESCRIPTOR_RANGE_TYPE_SRV, D3D12_FENCE_FLAG_NONE,
    D3D12_FILL_MODE_SOLID, D3D12_FILTER_MIN_MAG_MIP_LINEAR, D3D12_GRAPHICS_PIPELINE_STATE_DESC,
    D3D12_HEAP_FLAG_NONE, D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_UPLOAD, D3D12_INDEX_BUFFER_VIEW,
    D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA, D3D12_INPUT_ELEMENT_DESC, D3D12_INPUT_LAYOUT_DESC,
    D3D12_MEMORY_POOL_UNKNOWN, D3D12_PIPELINE_STATE_FLAG_NONE,
    D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE, D3D12_RANGE, D3D12_RASTERIZER_DESC,
    D3D12_RENDER_TARGET_BLEND_DESC, D3D12_RESOURCE_DESC, D3D12_RESOURCE_DIMENSION_BUFFER,
    D3D12_RESOURCE_FLAG_NONE, D3D12_RESOURCE_STATE_GENERIC_READ, D3D12_RESOURCE_STATE_PRESENT,
    D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_ROOT_CONSTANTS, D3D12_ROOT_DESCRIPTOR_TABLE,
    D3D12_ROOT_PARAMETER, D3D12_ROOT_PARAMETER_0, D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
    D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE, D3D12_ROOT_SIGNATURE_DESC,
    D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
    D3D12_ROOT_SIGNATURE_FLAG_DENY_DOMAIN_SHADER_ROOT_ACCESS,
    D3D12_ROOT_SIGNATURE_FLAG_DENY_GEOMETRY_SHADER_ROOT_ACCESS,
    D3D12_ROOT_SIGNATURE_FLAG_DENY_HULL_SHADER_ROOT_ACCESS, D3D12_SHADER_BYTECODE,
    D3D12_SHADER_RESOURCE_VIEW_DESC, D3D12_SHADER_RESOURCE_VIEW_DESC_0,
    D3D12_SHADER_VISIBILITY_PIXEL, D3D12_SHADER_VISIBILITY_VERTEX, D3D12_SRV_DIMENSION_TEXTURE2D,
    D3D12_STATIC_BORDER_COLOR_TRANSPARENT_BLACK, D3D12_STATIC_SAMPLER_DESC, D3D12_TEX2D_SRV,
    D3D12_TEXTURE_ADDRESS_MODE_WRAP, D3D12_TEXTURE_LAYOUT_ROW_MAJOR, D3D12_VERTEX_BUFFER_VIEW,
    D3D12_VIEWPORT, D3D_ROOT_SIGNATURE_VERSION_1_0,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_D32_FLOAT, DXGI_FORMAT_R16_UINT,
    DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{IDXGISwapChain3, DXGI_SWAP_CHAIN_DESC};
use windows::Win32::System::Threading::{
    CreateEventExW, WaitForSingleObjectEx, CREATE_EVENT, INFINITE,
};

use crate::util::{try_out_param, try_out_ptr, Barrier};

struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
}

pub struct Compositor(CompositorState);

enum CompositorState {
    Builder(Option<ID3D12Device>, Option<IDXGISwapChain3>, Option<ID3D12CommandQueue>),
    Compositor(CompositorInner),
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compositor {
    pub fn new() -> Self {
        Self(CompositorState::Builder(None, None, None))
    }

    pub fn with_swap_chain(&mut self, swap_chain: &IDXGISwapChain3) -> Result<&mut Self> {
        if let CompositorState::Builder(ref mut builder_device, ref mut builder_swap_chain, _) =
            self.0
        {
            *builder_device = Some(unsafe { swap_chain.GetDevice::<ID3D12Device>() }?);
            *builder_swap_chain = Some(swap_chain.clone());
        }

        self.check()?;

        Ok(self)
    }

    pub fn with_command_queue(&mut self, command_queue: &ID3D12CommandQueue) -> Result<&mut Self> {
        if let CompositorState::Builder(_, _, ref mut builder_command_queue) = self.0 {
            let desc = unsafe { command_queue.GetDesc() };
            if desc.Type.0 == 0 {
                *builder_command_queue = Some(command_queue.clone());
            }
        }

        self.check()?;

        Ok(self)
    }

    pub fn composite(&mut self, source_resource: ID3D12Resource) -> Result<()> {
        if let CompositorState::Compositor(ref mut inner) = self.0 {
            inner.composite(source_resource)?;
        }

        Ok(())
    }

    fn check(&mut self) -> Result<()> {
        if let CompositorState::Builder(
            ref mut device @ Some(_),
            ref mut swap_chain @ Some(_),
            ref mut command_queue @ Some(_),
        ) = self.0
        {
            self.0 = CompositorState::Compositor(CompositorInner::new(
                device.take().unwrap(),
                swap_chain.take().unwrap(),
                command_queue.take().unwrap(),
            )?);
        }

        Ok(())
    }
}

struct CompositorInner {
    device: ID3D12Device,
    target_swap_chain: IDXGISwapChain3,
    heap: ID3D12DescriptorHeap,
    rtv_heap: ID3D12DescriptorHeap,
    pipeline_state: ID3D12PipelineState,
    root_signature: ID3D12RootSignature,
    command_allocator: ID3D12CommandAllocator,
    command_list: ID3D12GraphicsCommandList,
    target_command_queue: ID3D12CommandQueue,
    fence: ID3D12Fence,
    fence_val: u64,
    fence_event: HANDLE,
    vertex_buffer: ID3D12Resource,
    index_buffer: ID3D12Resource,
}

impl CompositorInner {
    fn new(
        device: ID3D12Device,
        target_swap_chain: IDXGISwapChain3,
        target_command_queue: ID3D12CommandQueue,
    ) -> Result<Self> {
        let command_allocator: ID3D12CommandAllocator =
            unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }?;

        let command_list: ID3D12GraphicsCommandList = unsafe {
            device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &command_allocator, None)
        }?;
        unsafe { command_list.Close() }?;

        let heap: ID3D12DescriptorHeap = unsafe {
            device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                NumDescriptors: 8,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                NodeMask: 0,
            })
        }?;

        let rtv_heap: ID3D12DescriptorHeap = unsafe {
            device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                NumDescriptors: 8,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
                NodeMask: 0,
            })
        }?;

        let (pipeline_state, root_signature) = Self::create_device_objects(&device)?;

        let fence: ID3D12Fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }?;
        let fence_event = unsafe { CreateEventExW(None, None, CREATE_EVENT(0), 0x1F0003) }?;

        let vertex_buffer: ID3D12Resource = try_out_param(|v| unsafe {
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
                    Alignment: 65536,
                    Width: (64 * size_of::<Vertex>()) as u64,
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
        })?
        .unwrap();
        unsafe { vertex_buffer.SetName(w!("Compositor Vertex Buffer"))? };

        let index_buffer: ID3D12Resource = try_out_param(|v| unsafe {
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
                    Width: (64 * size_of::<u16>()) as u64,
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
        })?
        .unwrap();
        unsafe { vertex_buffer.SetName(w!("Compositor Index Buffer"))? };

        let vtx: &[Vertex] = &[
            Vertex { pos: [-1., -1.], uv: [0., 1.] },
            Vertex { pos: [1., -1.], uv: [1., 1.] },
            Vertex { pos: [-1., 1.], uv: [0., 0.] },
            Vertex { pos: [1., 1.], uv: [1., 0.] },
        ];
        let idx: &[u16] = &[0, 1, 2, 1, 3, 2];

        unsafe {
            let range = D3D12_RANGE { Begin: 0, End: mem::size_of_val(vtx) };
            let mut resource: *mut Vertex = null_mut();
            vertex_buffer.Map(
                0,
                Some(&range),
                Some(&mut resource as *mut _ as *mut *mut c_void),
            )?;
            ptr::copy_nonoverlapping(vtx.as_ptr(), resource, vtx.len());
            vertex_buffer.Unmap(0, Some(&range));
        }

        unsafe {
            let range = D3D12_RANGE { Begin: 0, End: mem::size_of_val(idx) };
            let mut resource: *mut u16 = null_mut();
            index_buffer.Map(0, Some(&range), Some(&mut resource as *mut _ as *mut *mut c_void))?;
            ptr::copy_nonoverlapping(idx.as_ptr(), resource, idx.len());
            index_buffer.Unmap(0, Some(&range));
        }

        Ok(Self {
            device,
            target_swap_chain,
            heap,
            rtv_heap,
            pipeline_state,
            root_signature,
            command_list,
            command_allocator,
            target_command_queue,
            fence,
            fence_val: 0,
            fence_event,
            vertex_buffer,
            index_buffer,
        })
    }

    fn create_device_objects(
        device: &ID3D12Device,
    ) -> Result<(ID3D12PipelineState, ID3D12RootSignature)> {
        let desc_range = D3D12_DESCRIPTOR_RANGE {
            RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
            NumDescriptors: 1,
            BaseShaderRegister: 0,
            RegisterSpace: 0,
            OffsetInDescriptorsFromTableStart: D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND,
        };

        let params = [
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
                        pDescriptorRanges: &desc_range,
                    },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
            },
        ];

        let sampler = D3D12_STATIC_SAMPLER_DESC {
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
        };

        let root_signature_desc = D3D12_ROOT_SIGNATURE_DESC {
            NumParameters: 2,
            pParameters: params.as_ptr(),
            NumStaticSamplers: 1,
            pStaticSamplers: &sampler,
            Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT
                | D3D12_ROOT_SIGNATURE_FLAG_DENY_HULL_SHADER_ROOT_ACCESS
                | D3D12_ROOT_SIGNATURE_FLAG_DENY_DOMAIN_SHADER_ROOT_ACCESS
                | D3D12_ROOT_SIGNATURE_FLAG_DENY_GEOMETRY_SHADER_ROOT_ACCESS,
        };
        let mut blob: Option<ID3DBlob> = None;
        let mut err_blob: Option<ID3DBlob> = None;
        if let Err(e) = unsafe {
            D3D12SerializeRootSignature(
                &root_signature_desc,
                D3D_ROOT_SIGNATURE_VERSION_1_0,
                &mut blob,
                Some(&mut err_blob),
            )
        } {
            if let Some(err_blob) = err_blob {
                let buf_ptr = unsafe { err_blob.GetBufferPointer() } as *mut u8;
                let buf_size = unsafe { err_blob.GetBufferSize() };
                let s = unsafe { String::from_raw_parts(buf_ptr, buf_size, buf_size + 1) };
                error!("Serializing root signature: {}: {}", e, s);
            }
            return Err(e);
        }

        let blob = blob.unwrap();
        let root_signature: ID3D12RootSignature = unsafe {
            device.CreateRootSignature(
                0,
                std::slice::from_raw_parts(
                    blob.GetBufferPointer() as *const u8,
                    blob.GetBufferSize(),
                ),
            )
        }?;

        let vs = r#"
                struct VS_INPUT
                {
                  float2 pos : POSITION;
                  float2 uv  : TEXCOORD0;
                };

                struct PS_INPUT
                {
                  float4 pos : SV_POSITION;
                  float2 uv  : TEXCOORD0;
                };

                PS_INPUT main(VS_INPUT input)
                {
                  PS_INPUT output;
                  output.pos = float4(input.pos.xy, 0.0f, 1.0f);
                  output.uv  = input.uv;
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
                struct PS_INPUT
                {
                  float4 pos : SV_POSITION;
                  float2 uv  : TEXCOORD0;
                };
                Texture2D texture0 : register(t0);
                SamplerState sampler0 : register(s0);

                float4 main(PS_INPUT input) : SV_Target
                {
                  float4 overlay_color = texture0.Sample(sampler0, input.uv);
                  return overlay_color;
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

        let mut pso_desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
            pRootSignature: ManuallyDrop::new(Some(root_signature.clone())),
            NodeMask: 1,
            PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
            SampleMask: u32::MAX,
            NumRenderTargets: 1,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
            ..Default::default()
        };
        pso_desc.RTVFormats[0] = DXGI_FORMAT_R8G8B8A8_UNORM;
        pso_desc.DSVFormat = DXGI_FORMAT_D32_FLOAT;

        pso_desc.VS = D3D12_SHADER_BYTECODE {
            pShaderBytecode: unsafe { vtx_shader.GetBufferPointer() },
            BytecodeLength: unsafe { vtx_shader.GetBufferSize() },
        };

        pso_desc.PS = D3D12_SHADER_BYTECODE {
            pShaderBytecode: unsafe { pix_shader.GetBufferPointer() },
            BytecodeLength: unsafe { pix_shader.GetBufferSize() },
        };

        let elem_descs = [
            D3D12_INPUT_ELEMENT_DESC {
                SemanticName: s!("POSITION\0"),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: offset_of!(Vertex, pos) as u32,
                InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D12_INPUT_ELEMENT_DESC {
                SemanticName: s!("TEXCOORD\0"),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: offset_of!(Vertex, uv) as u32,
                InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
        ];

        pso_desc.InputLayout =
            D3D12_INPUT_LAYOUT_DESC { pInputElementDescs: elem_descs.as_ptr(), NumElements: 2 };

        pso_desc.BlendState.AlphaToCoverageEnable = BOOL::from(false);
        pso_desc.BlendState.RenderTarget[0] = D3D12_RENDER_TARGET_BLEND_DESC {
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
        };
        pso_desc.RasterizerState = D3D12_RASTERIZER_DESC {
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
        };

        let pipeline_state: ID3D12PipelineState =
            unsafe { device.CreateGraphicsPipelineState(&pso_desc)? };

        Ok((pipeline_state, root_signature))
    }

    fn composite(&mut self, source_resource: ID3D12Resource) -> Result<()> {
        let desc: DXGI_SWAP_CHAIN_DESC =
            try_out_param(|v| unsafe { self.target_swap_chain.GetDesc(v) })?;
        let target_resource: ID3D12Resource = unsafe {
            self.target_swap_chain.GetBuffer(self.target_swap_chain.GetCurrentBackBufferIndex())
        }?;

        unsafe {
            if self.fence.GetCompletedValue() < self.fence_val {
                self.fence.SetEventOnCompletion(self.fence_val, self.fence_event)?;
                WaitForSingleObjectEx(self.fence_event, INFINITE, false);
            }
            self.fence_val += 1;
        }

        unsafe {
            self.device.CreateRenderTargetView(
                &target_resource,
                None,
                self.rtv_heap.GetCPUDescriptorHandleForHeapStart(),
            )
        };

        unsafe {
            self.device.CreateShaderResourceView(
                &source_resource,
                Some(&D3D12_SHADER_RESOURCE_VIEW_DESC {
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
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
                self.heap.GetCPUDescriptorHandleForHeapStart(),
            )
        };

        let target_rt_barrier = Barrier::new(
            target_resource.clone(),
            D3D12_RESOURCE_STATE_PRESENT,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        );

        unsafe {
            self.command_allocator.Reset()?;
            self.command_list.Reset(&self.command_allocator, None)?;

            self.command_list.ResourceBarrier(target_rt_barrier.as_ref());

            self.command_list.RSSetViewports(&[D3D12_VIEWPORT {
                TopLeftX: 0f32,
                TopLeftY: 0f32,
                Width: desc.BufferDesc.Width as f32,
                Height: desc.BufferDesc.Height as f32,
                MinDepth: 0f32,
                MaxDepth: 1f32,
            }]);
            self.command_list.RSSetScissorRects(&[RECT {
                left: 0,
                top: 0,
                right: desc.BufferDesc.Width as _,
                bottom: desc.BufferDesc.Height as _,
            }]);

            self.command_list.IASetVertexBuffers(
                0,
                Some(&[D3D12_VERTEX_BUFFER_VIEW {
                    BufferLocation: self.vertex_buffer.GetGPUVirtualAddress(),
                    SizeInBytes: (4 * size_of::<Vertex>()) as _,
                    StrideInBytes: size_of::<Vertex>() as _,
                }]),
            );

            self.command_list.IASetIndexBuffer(Some(&D3D12_INDEX_BUFFER_VIEW {
                BufferLocation: self.index_buffer.GetGPUVirtualAddress(),
                SizeInBytes: (6 * size_of::<u16>()) as _,
                Format: DXGI_FORMAT_R16_UINT,
            }));
            self.command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.command_list.SetPipelineState(&self.pipeline_state);
            self.command_list.SetGraphicsRootSignature(&self.root_signature);
            self.command_list.OMSetBlendFactor(Some(&[0f32; 4]));

            self.command_list.OMSetRenderTargets(
                1,
                Some(&self.rtv_heap.GetCPUDescriptorHandleForHeapStart()),
                BOOL::from(false),
                None,
            );
            self.command_list.SetDescriptorHeaps(&[Some(self.heap.clone())]);

            self.command_list
                .SetGraphicsRootDescriptorTable(1, self.heap.GetGPUDescriptorHandleForHeapStart());
            self.command_list.DrawIndexedInstanced(6, 1, 0, 0, 0);
            self.command_list.Close()?;

            self.target_command_queue
                .ExecuteCommandLists(&[Some(self.command_list.clone().cast()?)]);
            self.target_command_queue.Signal(&self.fence, self.fence_val)?;
        }

        Ok(())
    }
}
