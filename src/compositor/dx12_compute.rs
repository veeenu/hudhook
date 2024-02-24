use core::slice;
use std::mem::ManuallyDrop;
use std::ptr;

use tracing::trace;
use windows::core::{s, w, ComInterface, Error, Interface, Result, HRESULT, PCWSTR};
use windows::Win32::Graphics::Direct3D::Fxc::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;

use crate::renderer::print_dxgi_debug_messages;
use crate::util::{self, create_barrier, Fence};

pub struct Compositor {
    device: ID3D12Device,

    command_queue: ID3D12CommandQueue,
    command_allocator: ID3D12CommandAllocator,
    command_list: ID3D12GraphicsCommandList,

    srv_uav_heap: ID3D12DescriptorHeap,

    srv_handle_cpu: D3D12_CPU_DESCRIPTOR_HANDLE,
    srv_handle_gpu: D3D12_GPU_DESCRIPTOR_HANDLE,
    uav_handle_cpu: D3D12_CPU_DESCRIPTOR_HANDLE,
    uav_handle_gpu: D3D12_GPU_DESCRIPTOR_HANDLE,

    source_texture: ID3D12Resource,
    target_texture: ID3D12Resource,

    root_signature: ID3D12RootSignature,
    pipeline_state: ID3D12PipelineState,

    fence: Fence,
}

impl Compositor {
    pub fn new(device: &ID3D12Device) -> Result<Self> {
        let (command_queue, command_allocator, command_list) =
            unsafe { create_command_objects(device) }?;
        let srv_uav_heap = unsafe { create_descriptor_heap(device) }?;
        let (source_texture, target_texture) = unsafe { create_textures(device, 800, 600) }?;
        let root_signature = unsafe { create_root_signature(device) }?;
        let pipeline_state = unsafe { create_pipeline_state(device, &root_signature) }?;

        let increment_size = unsafe {
            device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV)
        };
        let srv_handle_cpu = unsafe { srv_uav_heap.GetCPUDescriptorHandleForHeapStart() };
        let srv_handle_gpu = unsafe { srv_uav_heap.GetGPUDescriptorHandleForHeapStart() };
        let uav_handle_cpu =
            D3D12_CPU_DESCRIPTOR_HANDLE { ptr: srv_handle_cpu.ptr + increment_size as usize };
        let uav_handle_gpu =
            D3D12_GPU_DESCRIPTOR_HANDLE { ptr: srv_handle_gpu.ptr + increment_size as u64 };

        let fence = Fence::new(device)?;

        Ok(Self {
            device: device.clone(),
            command_queue,
            command_allocator,
            command_list,
            srv_uav_heap,
            srv_handle_cpu,
            srv_handle_gpu,
            uav_handle_cpu,
            uav_handle_gpu,
            source_texture,
            target_texture,
            root_signature,
            pipeline_state,
            fence,
        })
    }

    pub fn composite(
        &self,
        source: ID3D12Resource,
        target: ID3D12Resource,
        cq: &ID3D12CommandQueue,
    ) -> Result<ID3D12Resource> {
        // Transition to a state where we can copy the incoming resources into
        // our compute shader resources.
        let barriers_copy_start = [
            util::create_barrier(
                &source,
                D3D12_RESOURCE_STATE_COMMON,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            ),
            util::create_barrier(
                &target,
                D3D12_RESOURCE_STATE_COMMON,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            ),
        ];

        // Transition to a state where we can execute the compute shader.
        let barriers_compute = [
            util::create_barrier(
                &self.source_texture,
                D3D12_RESOURCE_STATE_COPY_DEST,
                D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
            ),
            util::create_barrier(
                &self.target_texture,
                D3D12_RESOURCE_STATE_COPY_DEST,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            ),
        ];

        // Transition to a state where we can copy back the results to the
        // target resource.
        let barriers_copy_end = [util::create_barrier(
            &self.target_texture,
            D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            D3D12_RESOURCE_STATE_COPY_SOURCE,
        )];

        // Transition to a state where the target resource can be presented.
        let barriers_finish = [
            util::create_barrier(
                &source,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                D3D12_RESOURCE_STATE_PRESENT,
            ),
            util::create_barrier(
                &target,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                D3D12_RESOURCE_STATE_PRESENT,
            ),
            util::create_barrier(
                &self.source_texture,
                D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                D3D12_RESOURCE_STATE_COPY_DEST,
            ),
            util::create_barrier(
                &self.target_texture,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                D3D12_RESOURCE_STATE_COPY_DEST,
            ),
        ];

        unsafe {
            trace!("Cooking");
            self.device.CreateUnorderedAccessView(
                &self.target_texture,
                None,
                None,
                self.uav_handle_cpu,
            );
            self.device.CreateShaderResourceView(&self.source_texture, None, self.srv_handle_cpu);

            self.command_allocator.Reset()?;
            self.command_list.Reset(&self.command_allocator, None)?;
            self.command_list.ResourceBarrier(&barriers_copy_start);
            self.command_list.CopyResource(&self.source_texture, &source);
            self.command_list.CopyResource(&self.target_texture, &target);

            self.command_list.ResourceBarrier(&barriers_compute);
            self.command_list.SetComputeRootSignature(&self.root_signature);
            self.command_list.SetPipelineState(&self.pipeline_state);
            self.command_list.SetDescriptorHeaps(&[Some(self.srv_uav_heap.clone())]);
            self.command_list.SetComputeRootDescriptorTable(0, self.uav_handle_gpu);
            self.command_list.SetComputeRootDescriptorTable(1, self.srv_handle_gpu);
            self.command_list.Dispatch(32, 32, 1);

            self.command_list.ResourceBarrier(&barriers_copy_end);
            // self.command_list.CopyResource(&target, &self.target_texture);

            self.command_list.ResourceBarrier(&barriers_finish);
            self.command_list.Close()?;
            self.command_queue.ExecuteCommandLists(&[Some(self.command_list.cast()?)]);
            self.command_queue.Signal(self.fence.fence(), self.fence.value())?;
            self.fence.wait()?;
            trace!("I cooked");
            print_dxgi_debug_messages();
        }

        barriers_copy_start.into_iter().for_each(util::drop_barrier);
        barriers_compute.into_iter().for_each(util::drop_barrier);
        barriers_copy_end.into_iter().for_each(util::drop_barrier);
        barriers_finish.into_iter().for_each(util::drop_barrier);

        Ok(self.target_texture.clone())
    }
}

unsafe fn create_command_objects(
    device: &ID3D12Device,
) -> Result<(ID3D12CommandQueue, ID3D12CommandAllocator, ID3D12GraphicsCommandList)> {
    let command_queue: ID3D12CommandQueue =
        device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_COMPUTE,
            Priority: 0,
            Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
            NodeMask: 0,
        })?;

    let command_allocator: ID3D12CommandAllocator =
        device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_COMPUTE)?;

    let command_list: ID3D12GraphicsCommandList =
        device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_COMPUTE, &command_allocator, None)?;

    command_queue.SetName(w!("hudhook Compositor Command Queue"))?;
    command_allocator.SetName(w!("hudhook Compositor Command Allocator"))?;
    command_list.SetName(w!("hudhook Compositor Command list"))?;
    command_list.Close()?;

    Ok((command_queue, command_allocator, command_list))
}

unsafe fn create_descriptor_heap(device: &ID3D12Device) -> Result<ID3D12DescriptorHeap> {
    device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
        Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
        NumDescriptors: 2,
        Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
        NodeMask: 0,
    })
}

unsafe fn create_textures(
    device: &ID3D12Device,
    width: u32,
    height: u32,
) -> Result<(ID3D12Resource, ID3D12Resource)> {
    let heap_properties = D3D12_HEAP_PROPERTIES {
        Type: D3D12_HEAP_TYPE_DEFAULT,
        CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
        MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
        CreationNodeMask: 0,
        VisibleNodeMask: 0,
    };

    let texture_descriptor = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Alignment: 0,
        Width: width as u64,
        Height: height,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
        Flags: D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
    };

    let target_texture = util::try_out_ptr(|v| {
        device.CreateCommittedResource(
            &heap_properties,
            D3D12_HEAP_FLAG_NONE,
            &texture_descriptor,
            D3D12_RESOURCE_STATE_COPY_DEST,
            None,
            v,
        )
    })?;

    let source_texture = util::try_out_ptr(|v| {
        device.CreateCommittedResource(
            &heap_properties,
            D3D12_HEAP_FLAG_NONE,
            &texture_descriptor,
            D3D12_RESOURCE_STATE_COPY_DEST,
            None,
            v,
        )
    })?;

    Ok((source_texture, target_texture))
}

unsafe fn create_root_signature(device: &ID3D12Device) -> Result<ID3D12RootSignature> {
    let uav_range = [D3D12_DESCRIPTOR_RANGE {
        RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_UAV,
        NumDescriptors: 1,
        BaseShaderRegister: 0,
        RegisterSpace: 0,
        OffsetInDescriptorsFromTableStart: D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND,
    }];

    let srv_range = [D3D12_DESCRIPTOR_RANGE {
        RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
        NumDescriptors: 1,
        BaseShaderRegister: 0,
        RegisterSpace: 0,
        OffsetInDescriptorsFromTableStart: D3D12_DESCRIPTOR_RANGE_OFFSET_APPEND,
    }];

    let root_parameters = [
        D3D12_ROOT_PARAMETER {
            ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
            Anonymous: D3D12_ROOT_PARAMETER_0 {
                DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                    NumDescriptorRanges: 1,
                    pDescriptorRanges: uav_range.as_ptr(),
                },
            },
            ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
        },
        D3D12_ROOT_PARAMETER {
            ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
            Anonymous: D3D12_ROOT_PARAMETER_0 {
                DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                    NumDescriptorRanges: 1,
                    pDescriptorRanges: srv_range.as_ptr(),
                },
            },
            ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
        },
    ];

    let blob = util::try_out_err_blob(|v, err_blob| {
        D3D12SerializeRootSignature(
            &D3D12_ROOT_SIGNATURE_DESC {
                NumParameters: 2,
                pParameters: root_parameters.as_ptr(),
                NumStaticSamplers: 0,
                pStaticSamplers: ptr::null(),
                Flags: D3D12_ROOT_SIGNATURE_FLAG_DENY_VERTEX_SHADER_ROOT_ACCESS
                    | D3D12_ROOT_SIGNATURE_FLAG_DENY_HULL_SHADER_ROOT_ACCESS
                    | D3D12_ROOT_SIGNATURE_FLAG_DENY_DOMAIN_SHADER_ROOT_ACCESS
                    | D3D12_ROOT_SIGNATURE_FLAG_DENY_GEOMETRY_SHADER_ROOT_ACCESS
                    | D3D12_ROOT_SIGNATURE_FLAG_DENY_AMPLIFICATION_SHADER_ROOT_ACCESS
                    | D3D12_ROOT_SIGNATURE_FLAG_DENY_MESH_SHADER_ROOT_ACCESS,
            },
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
    root_signature.SetName(w!("hudhook Compositor Root Signature"))?;

    Ok(root_signature)
}

unsafe fn create_pipeline_state(
    device: &ID3D12Device,
    root_signature: &ID3D12RootSignature,
) -> Result<ID3D12PipelineState> {
    const CS: &str = r#"
        #define THREAD_GROUP_SIZE_X 16
        #define THREAD_GROUP_SIZE_Y 16

        RWTexture2D<float4> dst: register(u0); // UAV
        Texture2D<float4> src: register(t0); // SRV

        [numthreads(THREAD_GROUP_SIZE_X, THREAD_GROUP_SIZE_Y, 1)]
        void main(uint3 dispatchThreadID: SV_DispatchThreadID) {
            uint2 pixel = dispatchThreadID.xy;

            float4 srcColor = src.Load(int3(pixel, 0));
            float4 dstColor = dst[pixel];

            float4 outColor = srcColor + dstColor * (1 - srcColor.a);

            dst[pixel] = outColor;
        }
    "#;

    let compute_shader: ID3DBlob = util::try_out_err_blob(|v, err_blob| {
        D3DCompile(
            CS.as_ptr() as _,
            CS.len(),
            None,
            None,
            None::<&ID3DInclude>,
            s!("main\0"),
            s!("cs_5_0\0"),
            0,
            0,
            v,
            Some(err_blob),
        )
    })
    .map_err(util::print_error_blob("Compiling compute shader"))
    .expect("D3DCompile");

    let pso_desc = D3D12_COMPUTE_PIPELINE_STATE_DESC {
        pRootSignature: ManuallyDrop::new(Some(root_signature.clone())),
        CS: D3D12_SHADER_BYTECODE {
            pShaderBytecode: compute_shader.GetBufferPointer(),
            BytecodeLength: compute_shader.GetBufferSize(),
        },
        NodeMask: 0,
        CachedPSO: D3D12_CACHED_PIPELINE_STATE {
            pCachedBlob: ptr::null(),
            CachedBlobSizeInBytes: 0,
        },
        Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
    };

    let pipeline_state = device.CreateComputePipelineState(&pso_desc)?;

    let _ = ManuallyDrop::into_inner(pso_desc.pRootSignature);

    Ok(pipeline_state)
}
