use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::{self, size_of, ManuallyDrop};
use std::ptr::{null, null_mut};
use std::rc::Rc;

pub use imgui;
use imgui::internal::RawWrapper;
use imgui::{BackendFlags, Context, DrawCmd, DrawData, DrawIdx, DrawVert, TextureId, Ui};
use memoffset::offset_of;
use tracing::{error, trace};
use windows::core::{s, w, ComInterface, Result, PCSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, ID3DInclude, D3D_FEATURE_LEVEL_11_1, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIFactory2, DXGI_CREATE_FACTORY_DEBUG,
};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Threading::{
    CreateEventA, CreateEventExW, WaitForSingleObject, WaitForSingleObjectEx, CREATE_EVENT,
    INFINITE,
};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow, IsChild};

use super::{keys, RenderedSurface};
use crate::util::{try_out_param, try_out_ptr, Barrier};

struct RenderTarget {
    texture: ID3D12Resource,
    desc_handle: D3D12_CPU_DESCRIPTOR_HANDLE,
}

impl RenderTarget {
    fn new(
        device: &ID3D12Device,
        desc_handle: D3D12_CPU_DESCRIPTOR_HANDLE,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let texture = try_out_param(|v| unsafe {
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
        })?;

        let texture: ID3D12Resource = texture.unwrap();
        unsafe { texture.SetName(w!("Hudhook Render Target"))? };

        unsafe { device.CreateRenderTargetView(&texture, None, desc_handle) };

        Ok(Self { texture, desc_handle })
    }
}

impl Drop for RenderTarget {
    fn drop(&mut self) {
        trace!("Dropping render target");
    }
}

// Holds D3D12 buffers for a frame.
struct Buffers {
    vertex_buffer: ID3D12Resource,
    index_buffer: ID3D12Resource,
    vertex_buffer_size: usize,
    index_buffer_size: usize,
    vertices: Vec<DrawVert>,
    indices: Vec<DrawIdx>,
    projection_buffer: [[f32; 4]; 4],
}

impl Buffers {
    fn new(device: &ID3D12Device) -> Result<Self> {
        const INITIAL_VERTEX_CAPACITY: usize = 5000;
        const INITIAL_INDEX_CAPACITY: usize = 10000;
        Ok(Self {
            vertex_buffer: Self::create_vertex_buffer(device, INITIAL_VERTEX_CAPACITY)?,
            index_buffer: Self::create_index_buffer(device, INITIAL_INDEX_CAPACITY)?,
            vertex_buffer_size: INITIAL_VERTEX_CAPACITY,
            index_buffer_size: INITIAL_INDEX_CAPACITY,
            vertices: Vec::with_capacity(INITIAL_VERTEX_CAPACITY),
            indices: Vec::with_capacity(INITIAL_INDEX_CAPACITY),
            projection_buffer: Default::default(),
        })
    }

    fn create_vertex_buffer(device: &ID3D12Device, vertices: usize) -> Result<ID3D12Resource> {
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
                    Alignment: 65536,
                    Width: (vertices * size_of::<imgui::DrawVert>()) as u64,
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
        .map_err(|e| {
            error!("Creating vertex buffer: {:?}", e);
            e
        })
    }

    fn create_index_buffer(device: &ID3D12Device, indices: usize) -> Result<ID3D12Resource> {
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
                    Width: (indices * size_of::<imgui::DrawIdx>()) as u64,
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
        .map_err(|e| {
            error!("Creating index buffer: {:?}", e);
            e
        })
    }

    fn resize(&mut self, device: &ID3D12Device, indices: usize, vertices: usize) -> Result<()> {
        if self.vertex_buffer_size < vertices {
            let vertices = vertices + 5000;
            drop(mem::replace(
                &mut self.vertex_buffer,
                Buffers::create_vertex_buffer(device, vertices)?,
            ));

            self.vertex_buffer_size = vertices;
        }

        if self.index_buffer_size < indices {
            let indices = indices + 5000;
            drop(mem::replace(
                &mut self.index_buffer,
                Buffers::create_index_buffer(device, indices)?,
            ));

            self.index_buffer_size = indices;
        }
        Ok(())
    }
}

/// The [`hudhook`](crate) render engine.
///
/// Most of the operations of this structures are managed by the library itself
/// and are not available to the clients. For this reason, it can't be
/// instantiated directly but only by [`Hooks`](crate::Hooks) implementations
/// via [`RenderState`](crate::renderer::RenderState).
pub struct RenderEngine {
    target_hwnd: HWND,

    device: ID3D12Device,
    render_target: RenderTarget,

    command_queue: ID3D12CommandQueue,
    command_allocator: ID3D12CommandAllocator,
    command_list: ID3D12GraphicsCommandList,

    textures_heap: ID3D12DescriptorHeap,
    rtv_heap: ID3D12DescriptorHeap,

    cpu_desc: D3D12_CPU_DESCRIPTOR_HANDLE,
    gpu_desc: D3D12_GPU_DESCRIPTOR_HANDLE,
    buffers: Buffers,

    ctx: Rc<RefCell<Context>>,

    font_texture_resource: Option<ID3D12Resource>,
    root_signature: ID3D12RootSignature,
    pipeline_state: ID3D12PipelineState,

    textures: Vec<(ID3D12Resource, TextureId)>,
    fence: ID3D12Fence,
    fence_val: u64,
    fence_event: HANDLE,

    width: i32,
    height: i32,
}

impl RenderEngine {
    pub(crate) fn new(target_hwnd: HWND) -> Result<Self> {
        // Build device and swap chain.
        let dxgi_factory: IDXGIFactory2 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_DEBUG) }?;

        let dxgi_adapter = unsafe { dxgi_factory.EnumAdapters(0) }?;

        let device: ID3D12Device = try_out_ptr(|v| unsafe {
            D3D12CreateDevice(&dxgi_adapter, D3D_FEATURE_LEVEL_11_1, v)
        })?;

        let command_queue: ID3D12CommandQueue = unsafe {
            device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
                Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                Priority: 0,
                Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
                NodeMask: 0,
            })
        }?;

        unsafe { command_queue.SetName(w!("Render engine Command Queue")) }?;

        let (width, height) = crate::util::win_size(target_hwnd);

        // Descriptor heap for textures (font + user-defined)
        let textures_heap: ID3D12DescriptorHeap = unsafe {
            device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                NumDescriptors: 8,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                NodeMask: 0,
            })
        }?;

        let rtv_heap: ID3D12DescriptorHeap = unsafe {
            device
                .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                    NumDescriptors: 1,
                    Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
                    NodeMask: 1,
                })
                .unwrap()
        };

        let rtv_heap_handle_start = unsafe { rtv_heap.GetCPUDescriptorHandleForHeapStart() };

        let render_target =
            RenderTarget::new(&device, rtv_heap_handle_start, width as _, height as _)?;

        let buffers = Buffers::new(&device)?;

        // Build command objects.
        let command_allocator: ID3D12CommandAllocator =
            unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }?;

        let command_list: ID3D12GraphicsCommandList = unsafe {
            device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &command_allocator, None)
        }?;
        unsafe {
            command_list.Close().unwrap();
            command_list.SetName(w!("hudhook Command List"))?;
        };

        let cpu_desc = unsafe { textures_heap.GetCPUDescriptorHandleForHeapStart() };
        let gpu_desc = unsafe { textures_heap.GetGPUDescriptorHandleForHeapStart() };

        let (root_signature, pipeline_state) = Self::create_shader_program(&device)?;

        let mut ctx = Context::create();
        ctx.set_ini_filename(None);
        ctx.io_mut().backend_flags |= BackendFlags::RENDERER_HAS_VTX_OFFSET;
        ctx.set_renderer_name(String::from(concat!("imgui-dx12@", env!("CARGO_PKG_VERSION"))));

        let ctx = Rc::new(RefCell::new(ctx));

        let fence: ID3D12Fence = unsafe { device.CreateFence(0, D3D12_FENCE_FLAG_NONE)? };
        let fence_event = unsafe { CreateEventExW(None, None, CREATE_EVENT(0), 0x1f0003)? };

        let mut engine = Self {
            target_hwnd,
            device,
            render_target,
            command_queue,
            command_allocator,
            command_list,
            textures_heap,
            rtv_heap,
            cpu_desc,
            gpu_desc,
            buffers,
            ctx,
            font_texture_resource: None,
            root_signature,
            pipeline_state,
            textures: Vec::new(),
            fence,
            fence_val: 0,
            fence_event,
            width,
            height,
        };

        // TODO change this
        unsafe { engine.create_font_texture()? };

        Ok(engine)
    }

    pub(crate) fn resize(&mut self) -> Result<()> {
        let (width, height) = crate::util::win_size(self.target_hwnd);

        trace!("Resizing to {width}x{height}");

        self.render_target = RenderTarget::new(
            &self.device,
            unsafe { self.rtv_heap.GetCPUDescriptorHandleForHeapStart() },
            width as _,
            height as _,
        )?;

        self.width = width;
        self.height = height;

        Ok(())
    }

    pub(crate) fn render<F: FnMut(&mut Ui)>(&mut self, mut render_loop: F) -> Result<()> {
        unsafe { self.setup_io()? };

        // Reset command allocator and list state.
        unsafe {
            self.command_allocator.Reset().unwrap();
            self.command_list.Reset(&self.command_allocator, None).unwrap();
        }

        // Setup a barrier that waits for the back buffer to transition to a render
        // target.
        let back_buffer_to_rt_barrier = Barrier::new(
            self.render_target.texture.clone(),
            D3D12_RESOURCE_STATE_PRESENT,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        );

        unsafe {
            self.command_list.ResourceBarrier(back_buffer_to_rt_barrier.as_ref());

            // Setup the back buffer as a render target and clear it.
            self.command_list.OMSetRenderTargets(
                1,
                Some(&self.render_target.desc_handle),
                BOOL::from(false),
                None,
            );
            self.command_list.SetDescriptorHeaps(&[Some(self.textures_heap.clone())]);
            self.command_list.ClearRenderTargetView(
                self.render_target.desc_handle,
                &[0.0, 0.0, 0.0, 0.0],
                None,
            );
        }

        // Draw data loop.
        let ctx = Rc::clone(&self.ctx);
        let mut ctx = ctx.borrow_mut();
        let ui = ctx.frame();
        render_loop(ui);
        let draw_data = ctx.render();

        if let Err(e) = unsafe { self.render_draw_data(draw_data) } {
            eprintln!("{}", e);
        };

        // Setup a barrier to wait for the back buffer to transition to presentable
        // state.
        let back_buffer_to_present_barrier = Barrier::new(
            self.render_target.texture.clone(),
            D3D12_RESOURCE_STATE_RENDER_TARGET,
            D3D12_RESOURCE_STATE_PRESENT,
        );

        unsafe {
            self.command_list.ResourceBarrier(back_buffer_to_present_barrier.as_ref());

            // Close and execute the command list.
            self.command_list.Close()?;
            self.command_queue.ExecuteCommandLists(&[Some(self.command_list.cast()?)]);
            self.command_queue.Signal(&self.fence, self.fence_val)?;
            if self.fence.GetCompletedValue() < self.fence_val {
                self.fence.SetEventOnCompletion(self.fence_val, self.fence_event)?;
                WaitForSingleObjectEx(self.fence_event, INFINITE, false);
            }
            self.fence_val += 1;
        }

        // Drop the barriers.
        drop(back_buffer_to_rt_barrier);
        drop(back_buffer_to_present_barrier);

        Ok(())
    }
}

impl RenderEngine {
    /// Returns the [`HWND`] the UI is composited on top of.
    pub fn hwnd(&self) -> HWND {
        self.target_hwnd
    }

    /// Returns an internally mutable reference to the active
    /// [`imgui::Context`].
    pub fn ctx(&mut self) -> Rc<RefCell<Context>> {
        Rc::clone(&self.ctx)
    }

    /// Upload an image into a texture and return a [`imgui::TextureId`].
    ///
    /// Should be used ideally ahead-of-time in
    /// [`ImguiRenderLoop::initialize`](crate::ImguiRenderLoop::initialize), but
    /// can be used in
    /// [`ImguiRenderLoop::before_render`](crate::ImguiRenderLoop::before_render)
    /// provided the performance cost is acceptable.
    pub fn load_image(&mut self, data: &[u8], width: u32, height: u32) -> Result<TextureId> {
        unsafe { self.resize_texture_heap()? };

        let (p_texture, tex_id) =
            self.create_texture_inner(data, width, height, self.textures.len() as u32 + 1)?;

        self.textures.push((p_texture, tex_id));

        Ok(tex_id)
    }

    pub fn surface(&self) -> Result<RenderedSurface> {
        let device = self.device.clone();
        let command_queue = self.command_queue.clone();
        let command_allocator = self.command_allocator.clone();
        let command_list = self.command_list.clone();
        let resource = self.render_target.texture.clone();

        Ok(RenderedSurface { device, resource, command_queue, command_allocator, command_list })
    }
}

impl RenderEngine {
    unsafe fn setup_io(&mut self) -> Result<()> {
        let mut ctx = self.ctx.borrow_mut();

        // Setup display size and cursor position.
        let io = ctx.io_mut();

        io.display_size = [self.width as f32, self.height as f32];

        let active_window = unsafe { GetForegroundWindow() };
        if !HANDLE(active_window.0).is_invalid()
            && (active_window == self.target_hwnd
                || unsafe { IsChild(active_window, self.target_hwnd) }.as_bool())
        {
            let mut pos = Default::default();
            let gcp = unsafe { GetCursorPos(&mut pos) };
            if gcp.is_ok()
                && unsafe { ScreenToClient(self.target_hwnd, &mut pos as *mut _) }.as_bool()
            {
                io.mouse_pos[0] = pos.x as _;
                io.mouse_pos[1] = pos.y as _;
            }
        }

        io.nav_active = true;
        io.nav_visible = true;

        // Map key indices to the virtual key codes
        for i in keys::KEYS {
            io[i.0] = i.1 .0 as _;
        }

        Ok(())
    }

    fn create_shader_program(
        device: &ID3D12Device,
    ) -> Result<(ID3D12RootSignature, ID3D12PipelineState)> {
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
            pRootSignature: ManuallyDrop::new(Some(root_signature.clone())),
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
        drop(ManuallyDrop::into_inner(pso_desc.pRootSignature));

        Ok((root_signature, pipeline_state))
    }

    unsafe fn resize_texture_heap(&mut self) -> Result<()> {
        let mut desc = self.textures_heap.GetDesc();
        let old_num_descriptors = desc.NumDescriptors;
        if (old_num_descriptors as usize) < self.textures.len() {
            desc.NumDescriptors *= 2;
            let new_texture_heap: ID3D12DescriptorHeap = self.device.CreateDescriptorHeap(&desc)?;
            self.device.CopyDescriptorsSimple(
                old_num_descriptors,
                new_texture_heap.GetCPUDescriptorHandleForHeapStart(),
                self.textures_heap.GetCPUDescriptorHandleForHeapStart(),
                D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
            );
            self.textures_heap = new_texture_heap;
        }

        Ok(())
    }

    // TODO refactor this to pre-create command allocators/lists in the top level
    // object
    fn create_texture_inner(
        &mut self,
        data: &[u8],
        width: u32,
        height: u32,
        tex_index: u32,
    ) -> Result<(ID3D12Resource, TextureId)> {
        let heap_inc_size = unsafe {
            self.device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV)
        };

        let cpu_desc = D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: self.cpu_desc.ptr + (tex_index * heap_inc_size) as usize,
        };

        let gpu_desc = D3D12_GPU_DESCRIPTOR_HANDLE {
            ptr: self.gpu_desc.ptr + (tex_index * heap_inc_size) as u64,
        };

        let p_texture: ManuallyDrop<Option<ID3D12Resource>> =
            ManuallyDrop::new(try_out_param(|v| unsafe {
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
            })?);

        let mut upload_buffer: Option<ID3D12Resource> = None;
        let upload_pitch = width * 4;
        let upload_size = height * upload_pitch;
        unsafe {
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
                &mut upload_buffer,
            )
        }?;
        let upload_buffer = ManuallyDrop::new(upload_buffer);

        let range = D3D12_RANGE { Begin: 0, End: upload_size as usize };
        if let Some(ub) = upload_buffer.as_ref() {
            unsafe {
                let mut ptr: *mut u8 = null_mut();
                ub.Map(0, Some(&range), Some(&mut ptr as *mut _ as *mut *mut c_void)).unwrap();
                std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
                ub.Unmap(0, Some(&range));
            }
        };

        let fence: ID3D12Fence = unsafe { self.device.CreateFence(0, D3D12_FENCE_FLAG_NONE) }?;

        let event =
            unsafe { CreateEventA(None, BOOL::from(false), BOOL::from(false), PCSTR(null())) }?;

        let cmd_allocator: ID3D12CommandAllocator =
            unsafe { self.device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }?;

        unsafe { cmd_allocator.SetName(w!("hudhook texture Command Allocator")) }?;

        let cmd_list: ID3D12GraphicsCommandList = unsafe {
            self.device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &cmd_allocator, None)
        }?;

        unsafe { cmd_list.SetName(w!("hudhook texture Command List")) }?;

        let src_location = D3D12_TEXTURE_COPY_LOCATION {
            pResource: upload_buffer,
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
        };

        let dst_location = D3D12_TEXTURE_COPY_LOCATION {
            pResource: p_texture.clone(),
            Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
        };

        unsafe {
            cmd_list.CopyTextureRegion(&dst_location, 0, 0, 0, &src_location, None);
            cmd_list.ResourceBarrier(&[D3D12_RESOURCE_BARRIER {
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
            }]);
            cmd_list.Close().unwrap();
            self.command_queue.ExecuteCommandLists(&[Some(cmd_list.cast()?)]);
            self.command_queue.Signal(&fence, 1)?;
            fence.SetEventOnCompletion(1, event)?;
            WaitForSingleObject(event, u32::MAX);
        };

        unsafe { CloseHandle(event)? };

        unsafe {
            self.device.CreateShaderResourceView(
                p_texture.as_ref(),
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
            )
        };

        let tex_id = TextureId::from(gpu_desc.ptr as usize);

        drop(ManuallyDrop::into_inner(src_location.pResource));

        Ok((ManuallyDrop::into_inner(p_texture).unwrap(), tex_id))
    }

    unsafe fn create_font_texture(&mut self) -> Result<()> {
        let ctx = Rc::clone(&self.ctx);
        let mut ctx = ctx.borrow_mut();
        let fonts = ctx.fonts();
        let texture = fonts.build_rgba32_texture();

        self.resize_texture_heap()?;
        let (p_texture, tex_id) =
            self.create_texture_inner(texture.data, texture.width, texture.height, 0)?;

        drop(self.font_texture_resource.take());
        self.font_texture_resource = Some(p_texture);
        fonts.tex_id = tex_id;

        Ok(())
    }

    unsafe fn setup_render_state(&mut self, draw_data: &DrawData) {
        let display_size = draw_data.display_size;

        trace!("Display size {}x{}", display_size[0], display_size[1]);
        self.command_list.RSSetViewports(&[D3D12_VIEWPORT {
            TopLeftX: 0f32,
            TopLeftY: 0f32,
            Width: display_size[0],
            Height: display_size[1],
            MinDepth: 0f32,
            MaxDepth: 1f32,
        }]);

        self.command_list.IASetVertexBuffers(
            0,
            Some(&[D3D12_VERTEX_BUFFER_VIEW {
                BufferLocation: self.buffers.vertex_buffer.GetGPUVirtualAddress(),
                SizeInBytes: (self.buffers.vertex_buffer_size * size_of::<DrawVert>()) as _,
                StrideInBytes: size_of::<DrawVert>() as _,
            }]),
        );

        self.command_list.IASetIndexBuffer(Some(&D3D12_INDEX_BUFFER_VIEW {
            BufferLocation: self.buffers.index_buffer.GetGPUVirtualAddress(),
            SizeInBytes: (self.buffers.index_buffer_size * size_of::<DrawIdx>()) as _,
            Format: if size_of::<DrawIdx>() == 2 {
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
            self.buffers.projection_buffer.as_ptr() as *const c_void,
            0,
        );
        self.command_list.OMSetBlendFactor(Some(&[0f32; 4]));
    }

    unsafe fn render_draw_data(&mut self, draw_data: &DrawData) -> Result<()> {
        let print_device_removed_reason = |e: windows::core::Error| -> windows::core::Error {
            trace!("Device removed reason: {:?}", unsafe { self.device.GetDeviceRemovedReason() });
            e
        };

        if draw_data.display_size[0] <= 0f32 || draw_data.display_size[1] <= 0f32 {
            trace!(
                "Insufficent display size {}x{}, skip rendering",
                draw_data.display_size[0],
                draw_data.display_size[1]
            );
            return Ok(());
        }

        self.buffers
            .resize(
                &self.device,
                draw_data.total_idx_count as usize,
                draw_data.total_vtx_count as usize,
            )
            .map_err(print_device_removed_reason)?;

        let range = D3D12_RANGE::default();
        let mut vtx_resource: *mut imgui::DrawVert = null_mut();
        let mut idx_resource: *mut imgui::DrawIdx = null_mut();

        self.buffers.vertices.clear();
        self.buffers.indices.clear();
        draw_data.draw_lists().map(|m| (m.vtx_buffer().iter(), m.idx_buffer().iter())).for_each(
            |(v, i)| {
                self.buffers.vertices.extend(v);
                self.buffers.indices.extend(i);
            },
        );

        let vertices = &self.buffers.vertices;
        let indices = &self.buffers.indices;

        {
            self.buffers
                .vertex_buffer
                .Map(0, Some(&range), Some(&mut vtx_resource as *mut _ as *mut *mut c_void))
                .map_err(print_device_removed_reason)?;
            std::ptr::copy_nonoverlapping(vertices.as_ptr(), vtx_resource, vertices.len());
            self.buffers.vertex_buffer.Unmap(0, Some(&range));

            self.buffers
                .index_buffer
                .Map(0, Some(&range), Some(&mut idx_resource as *mut _ as *mut *mut c_void))
                .map_err(print_device_removed_reason)?;
            std::ptr::copy_nonoverlapping(indices.as_ptr(), idx_resource, indices.len());
            self.buffers.index_buffer.Unmap(0, Some(&range));

            self.buffers.projection_buffer = {
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
        }

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
}
