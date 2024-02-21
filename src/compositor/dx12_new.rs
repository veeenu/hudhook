use std::{mem, ptr, slice};

use imgui::DrawVert;
use memoffset::offset_of;
use tracing::error;
use windows::core::{s, w, ComInterface, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::Fxc::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;

use crate::renderer::print_dxgi_debug_messages;
use crate::util::{self, Fence};

#[repr(C)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
}

pub struct Compositor {
    device: ID3D12Device,

    command_queue: ID3D12CommandQueue,
    command_allocator: ID3D12CommandAllocator,
    command_list: ID3D12GraphicsCommandList,

    rtv_heap: ID3D12DescriptorHeap,
    srv_heap: ID3D12DescriptorHeap,

    vertex_buffer: ID3D12Resource,
    index_buffer: ID3D12Resource,

    root_signature: ID3D12RootSignature,
    pipeline_state: ID3D12PipelineState,

    fence: Fence,
}

impl Compositor {
    pub fn new(command_queue: ID3D12CommandQueue) -> Result<Self> {
        let device: ID3D12Device = util::try_out_ptr(|v| unsafe { command_queue.GetDevice(v) })?;

        let command_allocator: ID3D12CommandAllocator =
            unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }?;
        let command_list: ID3D12GraphicsCommandList = unsafe {
            device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &command_allocator, None)
        }?;
        unsafe {
            command_list.SetName(w!("hudhook Compositor Command List"))?;
            command_list.Close()?;
        }

        let rtv_heap: ID3D12DescriptorHeap = unsafe {
            device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                NumDescriptors: 1,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
                NodeMask: 1,
            })
        }?;

        let srv_heap: ID3D12DescriptorHeap = unsafe {
            device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                NumDescriptors: 8,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                NodeMask: 0,
            })
        }?;

        let vertex_buffer: ID3D12Resource = util::try_out_ptr(|v| unsafe {
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
                    Width: (64 * mem::size_of::<Vertex>()) as u64,
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
        unsafe { vertex_buffer.SetName(w!("hudhook Compositor Vertex Buffer"))? };

        let index_buffer: ID3D12Resource = util::try_out_ptr(|v| unsafe {
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
                    Width: (64 * mem::size_of::<u16>()) as u64,
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
        unsafe { index_buffer.SetName(w!("hudhook Compositor Index Buffer"))? };

        const VERTICES: [Vertex; 4] = [
            Vertex { pos: [-1., -1.], uv: [0., 1.] },
            Vertex { pos: [1., -1.], uv: [1., 1.] },
            Vertex { pos: [-1., 1.], uv: [0., 0.] },
            Vertex { pos: [1., 1.], uv: [1., 0.] },
        ];
        const INDICES: [u16; 6] = [0, 1, 2, 1, 3, 2];

        unsafe {
            let mut resource = ptr::null_mut();
            vertex_buffer.Map(0, None, Some(&mut resource))?;
            ptr::copy_nonoverlapping(VERTICES.as_ptr(), resource as *mut Vertex, VERTICES.len());
            vertex_buffer.Unmap(0, None);

            let mut resource = ptr::null_mut();
            index_buffer.Map(0, None, Some(&mut resource))?;
            ptr::copy_nonoverlapping(INDICES.as_ptr(), resource as *mut u16, INDICES.len());
            index_buffer.Unmap(0, None);
        }

        let (root_signature, pipeline_state) = unsafe { create_shader_program(&device) }?;
        let fence = Fence::new(&device)?;

        Ok(Self {
            device,
            command_queue,
            command_allocator,
            command_list,
            rtv_heap,
            srv_heap,
            vertex_buffer,
            index_buffer,
            root_signature,
            pipeline_state,
            fence,
        })
    }

    pub fn composite(&self, source: ID3D12Resource, target: ID3D12Resource) -> Result<()> {
        let desc = unsafe { target.GetDesc() };

        let target_rt_barriers = [util::create_barrier(
            &target,
            D3D12_RESOURCE_STATE_PRESENT,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        )];

        let target_present_barriers = [util::create_barrier(
            &target,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
            D3D12_RESOURCE_STATE_PRESENT,
        )];

        unsafe {
            self.device.CreateRenderTargetView(
                &target,
                None,
                self.rtv_heap.GetCPUDescriptorHandleForHeapStart(),
            )
        };

        unsafe {
            self.device.CreateShaderResourceView(
                &source,
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
                self.srv_heap.GetCPUDescriptorHandleForHeapStart(),
            )
        };

        unsafe {
            self.command_allocator.Reset()?;
            self.command_list.Reset(&self.command_allocator, None)?;
            self.command_list.ResourceBarrier(&target_rt_barriers);
            self.command_list.RSSetViewports(&[D3D12_VIEWPORT {
                TopLeftX: 0f32,
                TopLeftY: 0f32,
                Width: desc.Width as f32,
                Height: desc.Height as f32,
                MinDepth: 0f32,
                MaxDepth: 1f32,
            }]);
            self.command_list.RSSetScissorRects(&[RECT {
                left: 0,
                top: 0,
                right: desc.Width as _,
                bottom: desc.Height as _,
            }]);
            self.command_list.IASetVertexBuffers(
                0,
                Some(&[D3D12_VERTEX_BUFFER_VIEW {
                    BufferLocation: self.vertex_buffer.GetGPUVirtualAddress(),
                    SizeInBytes: (4 * mem::size_of::<Vertex>()) as _,
                    StrideInBytes: mem::size_of::<Vertex>() as _,
                }]),
            );
            self.command_list.IASetIndexBuffer(Some(&D3D12_INDEX_BUFFER_VIEW {
                BufferLocation: self.index_buffer.GetGPUVirtualAddress(),
                SizeInBytes: (6 * mem::size_of::<u16>()) as _,
                Format: DXGI_FORMAT_R16_UINT,
            }));
            self.command_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.command_list.SetPipelineState(&self.pipeline_state);
            self.command_list.SetGraphicsRootSignature(&self.root_signature);
            self.command_list.OMSetBlendFactor(Some(&[0f32; 4]));

            self.command_list.OMSetRenderTargets(
                1,
                Some(&self.rtv_heap.GetCPUDescriptorHandleForHeapStart()),
                false,
                None,
            );
            self.command_list.SetDescriptorHeaps(&[Some(self.srv_heap.clone())]);
            self.command_list.SetGraphicsRootDescriptorTable(
                1,
                self.srv_heap.GetGPUDescriptorHandleForHeapStart(),
            );
            self.command_list.DrawIndexedInstanced(6, 1, 0, 0, 0);
            self.command_list.ResourceBarrier(&target_present_barriers);
            self.command_list.Close()?;

            self.command_queue.ExecuteCommandLists(&[Some(self.command_list.clone().cast()?)]);
            self.command_queue.Signal(self.fence.fence(), self.fence.value())?;
            self.fence.wait()?;
            self.fence.incr();
        }

        target_rt_barriers.into_iter().for_each(util::drop_barrier);
        target_present_barriers.into_iter().for_each(util::drop_barrier);

        Ok(())
    }
}

unsafe fn create_shader_program(
    device: &ID3D12Device,
) -> Result<(ID3D12RootSignature, ID3D12PipelineState)> {
    let parameters = [D3D12_ROOT_PARAMETER {
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
    }];
    let root_signature_desc = D3D12_ROOT_SIGNATURE_DESC {
        NumParameters: 1,
        pParameters: parameters.as_ptr(),
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

    let blob = util::try_out_err_blob(|v, err_blob| {
        D3D12SerializeRootSignature(
            &root_signature_desc,
            D3D_ROOT_SIGNATURE_VERSION_1_0,
            v,
            Some(err_blob),
        )
    })
    .map_err(util::print_error_blob("Serializing root signature"))
    .expect("D3D12SerializeRootSignature");

    let root_signature: ID3D12RootSignature = device.CreateRootSignature(
        0,
        slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize()),
    )?;

    const VS: &str = r#"
        struct VS_INPUT {
          float2 pos: POSITION;
          float2 uv: TEXCOORD0;
        };

        struct PS_INPUT {
          float4 pos: SV_POSITION;
          float2 uv: TEXCOORD0;
        };

        PS_INPUT main(VS_INPUT input) {
          PS_INPUT output;
          output.pos = float4(input.pos.xy, 0.f, 1.f);
          output.uv = input.uv;
          return output;
        }"#;

    const PS: &str = r#"
        struct PS_INPUT {
          float4 pos: SV_POSITION;
          float2 uv: TEXCOORD0;
        };

        SamplerState sampler0: register(s0);
        Texture2D texture0: register(t0);

        float4 main(PS_INPUT input): SV_Target {
          float4 out_col = texture0.Sample(sampler0, input.uv);
          return out_col;
        }"#;

    let vtx_shader: ID3DBlob = util::try_out_err_blob(|v, err_blob| unsafe {
        D3DCompile(
            VS.as_ptr() as _,
            VS.len(),
            None,
            None,
            None::<&ID3DInclude>,
            s!("main\0"),
            s!("vs_5_0\0"),
            0,
            0,
            v,
            Some(err_blob),
        )
    })
    .map_err(util::print_error_blob("Compiling vertex shader"))
    .expect("D3DCompile");

    let pix_shader = util::try_out_err_blob(|v, err_blob| unsafe {
        D3DCompile(
            PS.as_ptr() as _,
            PS.len(),
            None,
            None,
            None::<&ID3DInclude>,
            s!("main\0"),
            s!("ps_5_0\0"),
            0,
            0,
            v,
            Some(err_blob),
        )
    })
    .map_err(util::print_error_blob("Compiling pixel shader"))
    .expect("D3DCompile");

    let input_element_desc = [
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
    ];

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
            pInputElementDescs: input_element_desc.as_ptr(),
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
