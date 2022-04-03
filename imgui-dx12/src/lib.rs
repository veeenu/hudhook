pub use imgui;

use std::ffi::c_void;
use std::mem::{size_of, ManuallyDrop};
use std::ptr::{null, null_mut};

use imgui::internal::RawWrapper;
use imgui::{BackendFlags, DrawCmd, DrawIdx, DrawVert, TextureId};
use log::*;
use memoffset::offset_of;
use windows::core::PCSTR;
use windows::Win32::Foundation::{CloseHandle, BOOL, RECT};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, ID3DInclude, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::System::Threading::{CreateEventA, WaitForSingleObject};

pub struct RenderEngine {
    dev: ID3D12Device,
    rtv_format: DXGI_FORMAT,
    font_srv_cpu_desc_handle: D3D12_CPU_DESCRIPTOR_HANDLE,
    font_srv_gpu_desc_handle: D3D12_GPU_DESCRIPTOR_HANDLE,
    font_texture_resource: Option<ID3D12Resource>,
    frame_resources: Vec<FrameResources>,
    const_buf: [[f32; 4]; 4],

    root_signature: Option<ID3D12RootSignature>,
    pipeline_state: Option<ID3D12PipelineState>,
}

struct FrameResources {
    index_buffer: Option<ID3D12Resource>,
    vertex_buffer: Option<ID3D12Resource>,
    index_buffer_size: usize,
    vertex_buffer_size: usize,
}

impl FrameResources {
    fn resize(
        &mut self,
        dev: &ID3D12Device,
        indices: usize,
        vertices: usize,
    ) -> Result<(), windows::core::Error> {
        if self.vertex_buffer.is_none() || self.vertex_buffer_size < vertices {
            drop(self.vertex_buffer.take());

            self.vertex_buffer_size = vertices + 5000;
            let props = D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_UPLOAD,
                CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                CreationNodeMask: 0,
                VisibleNodeMask: 0,
            };
            let desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Alignment: 0,
                Width: (self.vertex_buffer_size * size_of::<imgui::DrawVert>()) as u64,
                Height: 1,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_UNKNOWN,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                Flags: D3D12_RESOURCE_FLAG_NONE,
            };

            if let Err(e) = unsafe {
                dev.CreateCommittedResource(
                    &props,
                    D3D12_HEAP_FLAG_NONE,
                    &desc,
                    D3D12_RESOURCE_STATE_GENERIC_READ,
                    null(),
                    &mut self.vertex_buffer as *mut Option<_>,
                )
            } {
                error!("{:?}", e);
            }
        }
        if self.index_buffer.is_none() || self.index_buffer_size < indices {
            drop(self.index_buffer.take());
            self.index_buffer_size = indices + 10000;
            let props = D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_UPLOAD,
                CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
                MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
                CreationNodeMask: 0,
                VisibleNodeMask: 0,
            };
            let desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Alignment: 0,
                Width: (self.index_buffer_size * size_of::<imgui::DrawIdx>()) as u64,
                Height: 1,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_UNKNOWN,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                Flags: D3D12_RESOURCE_FLAG_NONE,
            };

            if let Err(e) = unsafe {
                dev.CreateCommittedResource(
                    &props,
                    D3D12_HEAP_FLAG_NONE,
                    &desc,
                    D3D12_RESOURCE_STATE_GENERIC_READ,
                    null(),
                    &mut self.index_buffer as *mut _,
                )
            } {
                error!("{:?}", e);
            }
        }
        Ok(())
    }
}

impl Default for FrameResources {
    fn default() -> Self {
        Self {
            index_buffer: None,
            vertex_buffer: None,
            index_buffer_size: 10000,
            vertex_buffer_size: 5000,
        }
    }
}

impl RenderEngine {
    pub fn new(
        ctx: &mut imgui::Context,
        dev: ID3D12Device,
        num_frames_in_flight: u32,
        rtv_format: DXGI_FORMAT,
        _cb_svr_heap: ID3D12DescriptorHeap,
        font_srv_cpu_desc_handle: D3D12_CPU_DESCRIPTOR_HANDLE,
        font_srv_gpu_desc_handle: D3D12_GPU_DESCRIPTOR_HANDLE,
    ) -> Self {
        ctx.io_mut().backend_flags |= BackendFlags::RENDERER_HAS_VTX_OFFSET;

        let frame_resources = (0..num_frames_in_flight)
            .map(|_| FrameResources::default())
            .collect::<Vec<_>>();

        RenderEngine {
            dev,
            rtv_format,
            font_srv_cpu_desc_handle,
            font_srv_gpu_desc_handle,
            frame_resources,
            const_buf: [[0f32; 4]; 4],
            root_signature: None,
            pipeline_state: None,
            font_texture_resource: None,
        }
    }

    fn setup_render_state(
        &mut self,
        draw_data: &imgui::DrawData,
        cmd_list: &ID3D12GraphicsCommandList,
        frame_resources_idx: usize,
    ) {
        let display_pos = draw_data.display_pos;
        let display_size = draw_data.display_size;

        let frame_resources = &self.frame_resources[frame_resources_idx];
        self.const_buf = {
            let [l, t, r, b] = [
                display_pos[0],
                display_pos[1],
                display_pos[0] + display_size[0],
                display_pos[1] + display_size[1],
            ];

            [
                [2. / (r - l), 0., 0., 0.],
                [0., 2. / (t - b), 0., 0.],
                [0., 0., 0.5, 0.],
                [(r + l) / (l - r), (t + b) / (b - t), 0.5, 1.0],
            ]
        };

        unsafe {
            cmd_list.RSSetViewports(&[D3D12_VIEWPORT {
                TopLeftX: 0f32,
                TopLeftY: 0f32,
                Width: display_size[0],
                Height: display_size[1],
                MinDepth: 0f32,
                MaxDepth: 1f32,
            }])
        };

        unsafe {
            cmd_list.IASetVertexBuffers(
                0,
                &[D3D12_VERTEX_BUFFER_VIEW {
                    BufferLocation: frame_resources
                        .vertex_buffer
                        .as_ref()
                        .unwrap()
                        .GetGPUVirtualAddress(),
                    SizeInBytes: (frame_resources.vertex_buffer_size * size_of::<DrawVert>()) as _,
                    StrideInBytes: size_of::<DrawVert>() as _,
                }],
            )
        };

        unsafe {
            cmd_list.IASetIndexBuffer(&D3D12_INDEX_BUFFER_VIEW {
                BufferLocation: frame_resources
                    .index_buffer
                    .as_ref()
                    .unwrap()
                    .GetGPUVirtualAddress(),
                SizeInBytes: (frame_resources.index_buffer_size * size_of::<DrawIdx>()) as _,
                Format: if size_of::<DrawIdx>() == 2 {
                    DXGI_FORMAT_R16_UINT
                } else {
                    DXGI_FORMAT_R32_UINT
                },
            });
            cmd_list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            cmd_list.SetPipelineState(self.pipeline_state.as_ref().unwrap());
            cmd_list.SetGraphicsRootSignature(self.root_signature.as_ref().unwrap());
            cmd_list.SetGraphicsRoot32BitConstants(
                0,
                16,
                self.const_buf.as_ptr() as *const c_void,
                0,
            );
            cmd_list.OMSetBlendFactor(&[0f32; 4]);
        }
    }

    pub fn render_draw_data(
        &mut self,
        draw_data: &imgui::DrawData,
        cmd_list: &ID3D12GraphicsCommandList,
        frame_resources_idx: usize,
    ) {
        if draw_data.display_size[0] <= 0f32 || draw_data.display_size[1] <= 0f32 {
            return;
        }

        {
            if self.frame_resources[frame_resources_idx]
                .resize(
                    &self.dev,
                    draw_data.total_idx_count as usize,
                    draw_data.total_vtx_count as usize,
                )
                .is_err()
            {
                trace!("{:?}", unsafe { self.dev.GetDeviceRemovedReason() });
                panic!();
            }
        };

        let range = D3D12_RANGE::default();
        let mut vtx_resource: *mut imgui::DrawVert = null_mut();
        let mut idx_resource: *mut imgui::DrawIdx = null_mut();

        // I allocate vectors every single frame SwoleDoge
        let (vertices, indices): (Vec<DrawVert>, Vec<DrawIdx>) = draw_data
            .draw_lists()
            .map(|m| (m.vtx_buffer().iter(), m.idx_buffer().iter()))
            .fold((Vec::new(), Vec::new()), |(mut ov, mut oi), (v, i)| {
                ov.extend(v);
                oi.extend(i);
                (ov, oi)
            });

        {
            let frame_resources = &self.frame_resources[frame_resources_idx];

            frame_resources.vertex_buffer.as_ref().map(|vb| unsafe {
                vb.Map(0, &range, &mut vtx_resource as *mut _ as _).unwrap();
                std::ptr::copy_nonoverlapping(vertices.as_ptr(), vtx_resource, vertices.len());
                vb.Unmap(0, &range);
            });

            frame_resources.index_buffer.as_ref().map(|ib| unsafe {
                ib.Map(0, &range, &mut idx_resource as *mut _ as _).unwrap();
                std::ptr::copy_nonoverlapping(indices.as_ptr(), idx_resource, indices.len());
                ib.Unmap(0, &range);
            });
        }

        {
            self.setup_render_state(draw_data, &cmd_list, frame_resources_idx as usize);
        }

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
                                cmd_list.SetGraphicsRootDescriptorTable(1, tex_handle);
                                cmd_list.RSSetScissorRects(&[r]);
                                cmd_list.DrawIndexedInstanced(
                                    count as _,
                                    1,
                                    idx_offset as _,
                                    vtx_offset as _,
                                    0,
                                );
                            }
                        }

                        idx_offset += count;
                    }
                    DrawCmd::ResetRenderState => {
                        self.setup_render_state(draw_data, &cmd_list, frame_resources_idx);
                    }
                    DrawCmd::RawCallback { callback, raw_cmd } => unsafe {
                        callback(cl.raw(), raw_cmd)
                    },
                }
            }
            vtx_offset += cl.vtx_buffer().len();
        }
    }

    pub fn create_device_objects(&mut self, ctx: &mut imgui::Context) {
        if self.pipeline_state.is_some() {
            self.invalidate_device_objects();
        }

        let mut desc_range = D3D12_DESCRIPTOR_RANGE::default();
        desc_range.RangeType = D3D12_DESCRIPTOR_RANGE_TYPE_SRV;
        desc_range.NumDescriptors = 1;
        desc_range.BaseShaderRegister = 0;
        desc_range.RegisterSpace = 0;
        desc_range.OffsetInDescriptorsFromTableStart = 0;

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

        let mut root_signature_desc = D3D12_ROOT_SIGNATURE_DESC::default();
        root_signature_desc.NumParameters = 2;
        root_signature_desc.pParameters = params.as_ptr();
        root_signature_desc.NumStaticSamplers = 1;
        root_signature_desc.pStaticSamplers = &sampler;
        root_signature_desc.Flags = D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT
            | D3D12_ROOT_SIGNATURE_FLAG_DENY_HULL_SHADER_ROOT_ACCESS
            | D3D12_ROOT_SIGNATURE_FLAG_DENY_DOMAIN_SHADER_ROOT_ACCESS
            | D3D12_ROOT_SIGNATURE_FLAG_DENY_GEOMETRY_SHADER_ROOT_ACCESS;
        let mut blob: Option<ID3DBlob> = None;
        let mut err_blob: Option<ID3DBlob> = None;
        if let Err(e) = unsafe {
            D3D12SerializeRootSignature(
                &root_signature_desc,
                D3D_ROOT_SIGNATURE_VERSION_1_0,
                &mut blob,
                &mut err_blob,
            )
        } {
            if let Some(err_blob) = err_blob {
                let buf_ptr = unsafe { err_blob.GetBufferPointer() } as *mut u8;
                let buf_size = unsafe { err_blob.GetBufferSize() };
                let s = unsafe { String::from_raw_parts(buf_ptr, buf_size, buf_size + 1) };
                error!("{}: {}", e, s);
            }
        }

        let blob = blob.unwrap();
        self.root_signature = Some(
            unsafe {
                self.dev.CreateRootSignature(
                    0,
                    std::slice::from_raw_parts(
                        blob.GetBufferPointer() as *const u8,
                        blob.GetBufferSize(),
                    ),
                )
            }
            .unwrap(),
        );

        let mut vtx_shader: Option<ID3DBlob> = None;
        let mut pix_shader: Option<ID3DBlob> = None;

        let vs = r#"
                cbuffer vertexBuffer : register(b0) 
                {
                  float4x4 ProjectionMatrix; 
                };
                struct VS_INPUT
                {
                  float2 pos : POSITION;
                  float4 col : COLOR0;
                  float2 uv  : TEXCOORD0;
                };
                
                struct PS_INPUT
                {
                  float4 pos : SV_POSITION;
                  float4 col : COLOR0;
                  float2 uv  : TEXCOORD0;
                };
                
                PS_INPUT main(VS_INPUT input)
                {
                  PS_INPUT output;
                  output.pos = mul( ProjectionMatrix, float4(input.pos.xy, 0.f, 1.f));
                  output.col = input.col;
                  output.uv  = input.uv;
                  return output;
                }"#;

        unsafe {
            D3DCompile(
                vs.as_ptr() as _,
                vs.len(),
                PCSTR(null()),
                null(),
                None::<ID3DInclude>,
                PCSTR("main\0".as_ptr()),
                PCSTR("vs_5_0\0".as_ptr()),
                0,
                0,
                &mut vtx_shader as *mut _,
                &mut None as _,
            )
        }
        .unwrap();

        let ps = r#"
                struct PS_INPUT
                {
                  float4 pos : SV_POSITION;
                  float4 col : COLOR0;
                  float2 uv  : TEXCOORD0;
                };
                SamplerState sampler0 : register(s0);
                Texture2D texture0 : register(t0);
                
                float4 main(PS_INPUT input) : SV_Target
                {
                  float4 out_col = input.col * texture0.Sample(sampler0, input.uv); 
                  return out_col; 
                }"#;

        unsafe {
            D3DCompile(
                ps.as_ptr() as _,
                ps.len(),
                PCSTR(null()),
                null(),
                None::<ID3DInclude>,
                PCSTR("main\0".as_ptr()),
                PCSTR("ps_5_0\0".as_ptr()),
                0,
                0,
                &mut pix_shader as *mut _,
                &mut None as _,
            )
        }
        .unwrap();

        let mut pso_desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
            pRootSignature: self.root_signature.clone(),
            NodeMask: 1,
            PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
            SampleMask: u32::MAX,
            NumRenderTargets: 1,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
            ..Default::default()
        };
        pso_desc.RTVFormats[0] = self.rtv_format;
        pso_desc.DSVFormat = DXGI_FORMAT_D32_FLOAT;

        let vtx_shader = vtx_shader.unwrap();
        pso_desc.VS = D3D12_SHADER_BYTECODE {
            pShaderBytecode: unsafe { vtx_shader.GetBufferPointer() },
            BytecodeLength: unsafe { vtx_shader.GetBufferSize() },
        };

        let pix_shader = pix_shader.unwrap();
        pso_desc.PS = D3D12_SHADER_BYTECODE {
            pShaderBytecode: unsafe { pix_shader.GetBufferPointer() },
            BytecodeLength: unsafe { pix_shader.GetBufferSize() },
        };

        let elem_descs = [
            D3D12_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR("POSITION\0".as_ptr()),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: offset_of!(DrawVert, pos) as u32,
                InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D12_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR("TEXCOORD\0".as_ptr()),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R32G32_FLOAT,
                InputSlot: 0,
                AlignedByteOffset: offset_of!(DrawVert, uv) as u32,
                InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
            D3D12_INPUT_ELEMENT_DESC {
                SemanticName: PCSTR("COLOR\0".as_ptr()),
                SemanticIndex: 0,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                InputSlot: 0,
                AlignedByteOffset: offset_of!(DrawVert, col) as u32,
                InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            },
        ];

        pso_desc.InputLayout = D3D12_INPUT_LAYOUT_DESC {
            pInputElementDescs: elem_descs.as_ptr(),
            NumElements: 3,
        };

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

        let pipeline_state = unsafe { self.dev.CreateGraphicsPipelineState(&pso_desc) };
        self.pipeline_state = Some(pipeline_state.unwrap());

        self.create_font_texture(ctx);
    }

    fn invalidate_device_objects(&mut self) {
        if let Some(root_signature) = self.root_signature.take() {
            drop(root_signature);
        }
        if let Some(pipeline_state) = self.pipeline_state.take() {
            drop(pipeline_state);
        }
        if let Some(font_texture_resource) = self.font_texture_resource.take() {
            drop(font_texture_resource);
        }

        self.frame_resources.iter_mut().for_each(|fr| {
            fr.index_buffer.take();
            fr.vertex_buffer.take();
        });
    }

    fn create_font_texture(&mut self, ctx: &mut imgui::Context) {
        let mut fonts = ctx.fonts();
        let texture = fonts.build_rgba32_texture();

        let mut p_texture: Option<ID3D12Resource> = None;
        unsafe {
            self.dev.CreateCommittedResource(
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
                    Width: texture.width as _,
                    Height: texture.height as _,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
                    Flags: D3D12_RESOURCE_FLAG_NONE,
                },
                D3D12_RESOURCE_STATE_COPY_DEST,
                null(),
                &mut p_texture,
            )
        }
        .unwrap();

        let mut upload_buffer: Option<ID3D12Resource> = None;
        let upload_pitch = texture.width * 4;
        let upload_size = texture.height * upload_pitch;
        unsafe {
            self.dev.CreateCommittedResource(
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
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                    Flags: D3D12_RESOURCE_FLAG_NONE,
                },
                D3D12_RESOURCE_STATE_GENERIC_READ,
                null(),
                &mut upload_buffer,
            )
        }
        .unwrap();

        let range = D3D12_RANGE {
            Begin: 0,
            End: upload_size as usize,
        };
        upload_buffer.as_ref().map(|ub| unsafe {
            let mut ptr: *mut u8 = null_mut();
            ub.Map(0, &range, &mut ptr as *mut _ as _).unwrap();
            std::ptr::copy_nonoverlapping(texture.data.as_ptr(), ptr, texture.data.len());
            ub.Unmap(0, &range);
        });

        let fence: ID3D12Fence = unsafe { self.dev.CreateFence(0, D3D12_FENCE_FLAG_NONE) }.unwrap();

        let event =
            unsafe { CreateEventA(null(), BOOL::from(false), BOOL::from(false), PCSTR(null())) };

        let cmd_queue: ID3D12CommandQueue = unsafe {
            self.dev.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                Priority: Default::default(),
                Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
                NodeMask: 1,
            })
        }
        .unwrap();

        let cmd_allocator: ID3D12CommandAllocator = unsafe {
            self.dev
                .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
        }
        .unwrap();

        let cmd_list: ID3D12GraphicsCommandList = unsafe {
            self.dev.CreateCommandList(
                0,
                D3D12_COMMAND_LIST_TYPE_DIRECT,
                cmd_allocator.clone(),
                None,
            )
        }
        .unwrap();

        let src_location = D3D12_TEXTURE_COPY_LOCATION {
            pResource: upload_buffer.clone(),
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                    Offset: 0,
                    Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                        Width: texture.width,
                        Height: texture.height,
                        Depth: 1,
                        RowPitch: upload_pitch,
                    },
                },
            },
        };

        let dst_location = D3D12_TEXTURE_COPY_LOCATION {
            pResource: p_texture.clone(),
            Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                SubresourceIndex: 0,
            },
        };

        let barrier = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 {
                Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                    pResource: p_texture.clone(),
                    Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                    StateBefore: D3D12_RESOURCE_STATE_COPY_DEST,
                    StateAfter: D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                }),
            },
        };

        unsafe {
            cmd_list.CopyTextureRegion(&dst_location, 0, 0, 0, &src_location, null());
            cmd_list.ResourceBarrier(&[barrier]);
            cmd_list.Close().unwrap();
            cmd_queue.ExecuteCommandLists(&[Some(cmd_list.into())]);
            cmd_queue.Signal(&fence, 1).unwrap();
            fence.SetEventOnCompletion(1, &event).unwrap();
            WaitForSingleObject(event, u32::MAX);
        };

        let srv_desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
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
        };

        unsafe { CloseHandle(event) };

        unsafe {
            self.dev.CreateShaderResourceView(
                p_texture.clone(),
                &srv_desc,
                self.font_srv_cpu_desc_handle,
            )
        };
        drop(self.font_texture_resource.take());
        self.font_texture_resource = p_texture;
        fonts.tex_id = TextureId::from(self.font_srv_gpu_desc_handle.ptr as usize);
    }

    fn shutdown(&mut self) {}

    pub fn new_frame(&mut self, ctx: &mut imgui::Context) {
        if self.pipeline_state.is_none() {
            self.create_device_objects(ctx);
        }
    }
}
