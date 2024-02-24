use core::slice;
use std::mem::ManuallyDrop;
use std::ptr;

use tracing::trace;
use windows::core::{w, ComInterface, Error, Interface, Result, HRESULT, PCWSTR};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;

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

    pub fn composite(&self, source: ID3D12Resource, target: ID3D12Resource) -> Result<()> {
        trace!("Build barriers");
        let barriers_before = [
            util::create_barrier(
                &source,
                D3D12_RESOURCE_STATE_PRESENT,
                D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
            ),
            util::create_barrier(
                &target,
                D3D12_RESOURCE_STATE_PRESENT,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
            ),
        ];

        let barriers_after = [
            util::create_barrier(
                &source,
                D3D12_RESOURCE_STATE_NON_PIXEL_SHADER_RESOURCE,
                D3D12_RESOURCE_STATE_PRESENT,
            ),
            util::create_barrier(
                &target,
                D3D12_RESOURCE_STATE_UNORDERED_ACCESS,
                D3D12_RESOURCE_STATE_PRESENT,
            ),
        ];

        unsafe {
            trace!("Create uav/srv");
            self.device.CreateUnorderedAccessView(&target, None, None, self.uav_handle_cpu);
            self.device.CreateShaderResourceView(&source, None, self.srv_handle_cpu);

            self.command_allocator.Reset()?;
            self.command_list.Reset(&self.command_allocator, None)?;
            trace!("Barriers");
            self.device.CreateUnorderedAccessView(&target, None, None, self.uav_handle_cpu);
            self.command_list.ResourceBarrier(&barriers_before);
            trace!("Set crs");
            self.command_list.SetComputeRootSignature(&self.root_signature);
            trace!("Set ps");
            self.command_list.SetPipelineState(&self.pipeline_state);
            trace!("Set dh");
            self.command_list.SetDescriptorHeaps(&[Some(self.srv_uav_heap.clone())]);
            trace!("Set crdt");
            self.command_list.SetComputeRootDescriptorTable(0, self.uav_handle_gpu);
            self.command_list.SetComputeRootDescriptorTable(1, self.srv_handle_gpu);
            trace!("Dispatch");
            self.command_list.Dispatch(32, 32, 1);
            trace!("Barriers");
            self.command_list.ResourceBarrier(&barriers_after);
            trace!("ECL");
            self.command_queue.ExecuteCommandLists(&[Some(self.command_list.cast()?)]);
            self.command_queue.Signal(self.fence.fence(), self.fence.value())?;
            self.fence.wait()?;
        }

        barriers_before.into_iter().for_each(util::drop_barrier);
        barriers_after.into_iter().for_each(util::drop_barrier);

        Ok(())
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
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
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
    let root_parameters = [
        D3D12_ROOT_PARAMETER {
            ParameterType: D3D12_ROOT_PARAMETER_TYPE_UAV,
            Anonymous: D3D12_ROOT_PARAMETER_0 {
                Descriptor: D3D12_ROOT_DESCRIPTOR { ShaderRegister: 0, RegisterSpace: 0 },
            },
            ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
        },
        D3D12_ROOT_PARAMETER {
            ParameterType: D3D12_ROOT_PARAMETER_TYPE_SRV,
            Anonymous: D3D12_ROOT_PARAMETER_0 {
                Descriptor: D3D12_ROOT_DESCRIPTOR { ShaderRegister: 0, RegisterSpace: 0 },
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
                Flags: D3D12_ROOT_SIGNATURE_FLAG_NONE,
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
    const BYTECODE: &[u8] = include_bytes!("alpha_blend.cso");

    let bytecode = D3D12_SHADER_BYTECODE {
        pShaderBytecode: BYTECODE.as_ptr() as _,
        BytecodeLength: BYTECODE.len(),
    };

    let pso_desc = D3D12_COMPUTE_PIPELINE_STATE_DESC {
        pRootSignature: ManuallyDrop::new(Some(root_signature.clone())),
        CS: bytecode,
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
