use std::mem::{ManuallyDrop, MaybeUninit};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context as AnyhowCtx, Result};
use imgui::Context;
use windows::core::{w, ComInterface, PCWSTR};
use windows::Win32::Foundation::{BOOL, HANDLE, HWND, POINT};
use windows::Win32::Graphics::Direct3D::{
    D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_12_2, D3D_FEATURE_LEVEL_9_1,
};
use windows::Win32::Graphics::Direct3D12::{
    D3D12CreateDevice, ID3D12CommandAllocator, ID3D12CommandQueue, ID3D12DescriptorHeap,
    ID3D12Device, ID3D12Fence, ID3D12GraphicsCommandList, ID3D12Resource,
    D3D12_COMMAND_LIST_TYPE_DIRECT, D3D12_COMMAND_QUEUE_DESC, D3D12_COMMAND_QUEUE_FLAG_NONE,
    D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_DESCRIPTOR_HEAP_DESC, D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
    D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
    D3D12_DESCRIPTOR_HEAP_TYPE_RTV, D3D12_FENCE_FLAG_NONE, D3D12_RESOURCE_BARRIER,
    D3D12_RESOURCE_BARRIER_0, D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
    D3D12_RESOURCE_BARRIER_FLAG_NONE, D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
    D3D12_RESOURCE_STATES, D3D12_RESOURCE_STATE_PRESENT, D3D12_RESOURCE_STATE_RENDER_TARGET,
    D3D12_RESOURCE_TRANSITION_BARRIER,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGIGetDebugInterface1, IDXGIAdapter, IDXGIFactory2, IDXGIInfoQueue,
    IDXGISwapChain3, DXGI_CREATE_FACTORY_DEBUG, DXGI_DEBUG_ALL, DXGI_INFO_QUEUE_MESSAGE,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Threading::{
    CreateEventExW, WaitForSingleObjectEx, CREATE_EVENT, INFINITE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageA, GetCursorPos, GetForegroundWindow, GetMessageA, IsChild, TranslateMessage,
    WM_CLOSE, WM_QUIT,
};

use crate::imgui_dx12::RenderEngine;

fn try_out_param<T, F, E, O>(mut f: F) -> Result<T, E>
where
    T: Default,
    F: FnMut(&mut T) -> Result<O, E>,
{
    let mut t: T = Default::default();
    match f(&mut t) {
        Ok(_) => Ok(t),
        Err(e) => Err(e),
    }
}

#[derive(Debug)]
struct FrameContext {
    back_buffer: ID3D12Resource,
    desc_handle: D3D12_CPU_DESCRIPTOR_HANDLE,
    command_allocator: ID3D12CommandAllocator,
    fence: ID3D12Fence,
    fence_val: u64,
    fence_event: HANDLE,
}

impl FrameContext {
    fn incr(&mut self) {
        static FENCE_MAX: AtomicU64 = AtomicU64::new(0);
        self.fence_val = FENCE_MAX.fetch_add(1, Ordering::SeqCst);
    }

    fn wait_fence(&mut self) {
        unsafe {
            if self.fence.GetCompletedValue() < self.fence_val {
                self.fence.SetEventOnCompletion(self.fence_val, self.fence_event).unwrap();
                WaitForSingleObjectEx(self.fence_event, INFINITE, false);
            }
        }
    }
}

pub struct Dcomp {
    target_hwnd: HWND,
    dxgi_factory: IDXGIFactory2,
    dxgi_adapter: IDXGIAdapter,
    d3d12_dev: ID3D12Device,
    swap_chain: IDXGISwapChain3,
    command_queue: ID3D12CommandQueue,
    command_list: ID3D12GraphicsCommandList,
    renderer_heap: ID3D12DescriptorHeap,
    rtv_heap: ID3D12DescriptorHeap,

    dcomp_dev: IDCompositionDevice,
    dcomp_target: IDCompositionTarget,
    root_visual: IDCompositionVisual,
    engine: RenderEngine,
    ctx: Context,
    frame_contexts: Vec<FrameContext>,
}

impl Dcomp {
    pub unsafe fn new(target_hwnd: HWND) -> Result<Self> {
        let dxgi_factory: IDXGIFactory2 =
            CreateDXGIFactory2(DXGI_CREATE_FACTORY_DEBUG).context("dxgi factory")?;

        let dxgi_adapter = dxgi_factory.EnumAdapters(0).context("enum adapters")?;

        let mut d3d12_dev: Option<ID3D12Device> = None;
        D3D12CreateDevice(&dxgi_adapter, D3D_FEATURE_LEVEL_11_1, &mut d3d12_dev)
            .context("create device")?;
        let d3d12_dev = d3d12_dev.unwrap();

        let queue_desc = D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
            Priority: 0,
            Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
            NodeMask: 0,
        };

        let command_queue: ID3D12CommandQueue =
            unsafe { d3d12_dev.CreateCommandQueue(&queue_desc as *const _) }.unwrap();

        let (width, height) = crate::win_size(target_hwnd);

        let sd = DXGI_SWAP_CHAIN_DESC1 {
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Width: width as _,
            Height: height as _,
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            ..Default::default()
        };

        let swap_chain = dxgi_factory
            .CreateSwapChainForComposition(&command_queue, &sd, None)
            .ok()
            .context("create swap chain")?
            .cast::<IDXGISwapChain3>()
            .ok()
            .context("query interface")?;

        let renderer_heap: ID3D12DescriptorHeap = unsafe {
            d3d12_dev
                .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                    NumDescriptors: sd.BufferCount,
                    Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                    NodeMask: 0,
                })
                .context("create descriptor heap")?
        };

        let command_allocator: ID3D12CommandAllocator = d3d12_dev
            .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
            .context("create command allocator")?;

        let command_list: ID3D12GraphicsCommandList = d3d12_dev
            .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &command_allocator, None)
            .unwrap();
        command_list.Close().unwrap();

        command_list.SetName(w!("hudhook Command List")).expect("Couldn't set command list name");

        let rtv_heap: ID3D12DescriptorHeap = d3d12_dev
            .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                NumDescriptors: sd.BufferCount,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
                NodeMask: 1,
            })
            .unwrap();

        let rtv_heap_inc_size =
            d3d12_dev.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV);

        let rtv_handle_start = rtv_heap.GetCPUDescriptorHandleForHeapStart();

        let frame_contexts: Vec<FrameContext> = (0..sd.BufferCount)
            .map(|i| {
                const COMMAND_ALLOCATOR_NAMES: [PCWSTR; 8] = [
                    w!("hudhook Command allocator #0"),
                    w!("hudhook Command allocator #1"),
                    w!("hudhook Command allocator #2"),
                    w!("hudhook Command allocator #3"),
                    w!("hudhook Command allocator #4"),
                    w!("hudhook Command allocator #5"),
                    w!("hudhook Command allocator #6"),
                    w!("hudhook Command allocator #7"),
                ];

                let desc_handle = D3D12_CPU_DESCRIPTOR_HANDLE {
                    ptr: rtv_handle_start.ptr + (i * rtv_heap_inc_size) as usize,
                };

                let back_buffer: ID3D12Resource = swap_chain.GetBuffer(i).context("get buffer")?;
                d3d12_dev.CreateRenderTargetView(&back_buffer, None, desc_handle);

                let command_allocator: ID3D12CommandAllocator =
                    d3d12_dev.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT).unwrap();
                let command_allocator_name = COMMAND_ALLOCATOR_NAMES
                    [usize::min(COMMAND_ALLOCATOR_NAMES.len() - 1, i as usize)];

                command_allocator
                    .SetName(command_allocator_name)
                    .context("Couldn't set command allocator name")?;

                Ok(FrameContext {
                    desc_handle,
                    back_buffer,
                    command_allocator,
                    fence: d3d12_dev.CreateFence(0, D3D12_FENCE_FLAG_NONE).unwrap(),
                    fence_val: 0,
                    fence_event: CreateEventExW(None, None, CREATE_EVENT(0), 0x1F0003).unwrap(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        println!("{frame_contexts:?}");

        let mut ctx = Context::create();
        let cpu_desc = renderer_heap.GetCPUDescriptorHandleForHeapStart();
        let gpu_desc = renderer_heap.GetGPUDescriptorHandleForHeapStart();
        let engine = RenderEngine::new(
            &mut ctx,
            d3d12_dev.clone(),
            sd.BufferCount,
            DXGI_FORMAT_R8G8B8A8_UNORM,
            renderer_heap.clone(),
            cpu_desc,
            gpu_desc,
        );

        let dcomp_dev: IDCompositionDevice =
            DCompositionCreateDevice(None).context("create dcomp device")?;
        let dcomp_target = dcomp_dev
            .CreateTargetForHwnd(target_hwnd, BOOL::from(true))
            .context("create target for hwnd")?;

        let root_visual = dcomp_dev.CreateVisual().context("create visual")?;
        dcomp_target.SetRoot(&root_visual).context("set root")?;
        dcomp_dev.Commit().context("commit")?;

        Ok(Self {
            target_hwnd,
            dxgi_factory,
            dxgi_adapter,
            d3d12_dev,
            swap_chain,
            command_queue,
            command_list,
            renderer_heap,
            rtv_heap,
            dcomp_dev,
            dcomp_target,
            root_visual,
            engine,
            ctx,
            frame_contexts,
        })
    }

    pub unsafe fn render(&mut self) -> Result<()> {
        let frame_contexts_idx = unsafe { self.swap_chain.GetCurrentBackBufferIndex() } as usize;
        let frame_context = &mut self.frame_contexts[frame_contexts_idx];

        let sd = try_out_param(|sd| unsafe { self.swap_chain.GetDesc1(sd) }).context("get desc")?;
        let width = sd.Width;
        let height = sd.Height;

        let io = self.ctx.io_mut();

        io.display_size = [width as f32, height as f32];

        let mut pos = POINT { x: 0, y: 0 };

        let active_window = unsafe { GetForegroundWindow() };
        if !HANDLE(active_window.0).is_invalid()
            && (active_window == self.target_hwnd
                || unsafe { IsChild(active_window, self.target_hwnd) }.as_bool())
        {
            let gcp = unsafe { GetCursorPos(&mut pos as *mut _) };
            if gcp.is_ok()
                && unsafe { ScreenToClient(self.target_hwnd, &mut pos as *mut _) }.as_bool()
            {
                io.mouse_pos[0] = pos.x as _;
                io.mouse_pos[1] = pos.y as _;
            }
        }

        self.engine.new_frame(&mut self.ctx);
        let ctx = &mut self.ctx;
        let ui = ctx.frame();
        ui.show_demo_window(&mut true);
        let draw_data = ctx.render();

        let back_buffer = frame_context.back_buffer.clone();

        let back_buffer_to_rt_barrier = Barrier::create(
            back_buffer.clone(),
            D3D12_RESOURCE_STATE_PRESENT,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        );

        frame_context.wait_fence();
        frame_context.incr();
        let command_allocator = &frame_context.command_allocator;

        unsafe {
            command_allocator.Reset().unwrap();
            self.command_list.Reset(command_allocator, None).unwrap();
            self.command_list.ResourceBarrier(&back_buffer_to_rt_barrier);
            self.command_list.OMSetRenderTargets(
                1,
                Some(&frame_context.desc_handle),
                BOOL::from(false),
                None,
            );
            self.command_list.SetDescriptorHeaps(&[Some(self.renderer_heap.clone())]);
            self.command_list.ClearRenderTargetView(
                frame_context.desc_handle,
                &[0.0, 0.0, 0.0, 0.0],
                None,
            );
        };

        if let Err(e) =
            self.engine.render_draw_data(draw_data, &self.command_list, frame_contexts_idx)
        {
            eprintln!("{}", e);
        };

        let back_buffer_to_present_barrier = Barrier::create(
            back_buffer.clone(),
            D3D12_RESOURCE_STATE_RENDER_TARGET,
            D3D12_RESOURCE_STATE_PRESENT,
        );

        unsafe {
            self.command_list.ResourceBarrier(&back_buffer_to_present_barrier);
        }

        unsafe {
            self.command_list.Close()?;
            self.command_queue.ExecuteCommandLists(&[Some(self.command_list.cast()?)]);
            self.command_queue.Signal(&frame_context.fence, frame_context.fence_val)?;
        }

        self.swap_chain.Present(1, 0).ok().context("present")?;

        Barrier::drop(back_buffer_to_rt_barrier);
        Barrier::drop(back_buffer_to_present_barrier);

        self.root_visual.SetContent(&self.swap_chain).context("set content")?;
        self.dcomp_dev.Commit()?;

        Ok(())
    }
}

struct Barrier;

impl Barrier {
    fn create(
        buf: ID3D12Resource,
        before: D3D12_RESOURCE_STATES,
        after: D3D12_RESOURCE_STATES,
    ) -> Vec<D3D12_RESOURCE_BARRIER> {
        let transition_barrier = ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
            pResource: ManuallyDrop::new(Some(buf)),
            Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
            StateBefore: before,
            StateAfter: after,
        });

        let barrier = D3D12_RESOURCE_BARRIER {
            Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
            Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
            Anonymous: D3D12_RESOURCE_BARRIER_0 { Transition: transition_barrier },
        };

        vec![barrier]
    }

    fn drop(barriers: Vec<D3D12_RESOURCE_BARRIER>) {
        for barrier in barriers {
            let transition = ManuallyDrop::into_inner(unsafe { barrier.Anonymous.Transition });
            let _ = ManuallyDrop::into_inner(transition.pResource);
        }
    }
}

unsafe fn print_dxgi_debug_messages() {
    let diq: IDXGIInfoQueue = DXGIGetDebugInterface1(0).unwrap();

    for i in 0..diq.GetNumStoredMessages(DXGI_DEBUG_ALL) {
        let mut msg_len: usize = 0;
        diq.GetMessage(DXGI_DEBUG_ALL, i, None, &mut msg_len as _).unwrap();
        let diqm = vec![0u8; msg_len];
        let pdiqm = diqm.as_ptr() as *mut DXGI_INFO_QUEUE_MESSAGE;
        diq.GetMessage(DXGI_DEBUG_ALL, i, Some(pdiqm), &mut msg_len as _).unwrap();
        let diqm = pdiqm.as_ref().unwrap();
        eprintln!(
            "[DIQ] {}",
            String::from_utf8_lossy(std::slice::from_raw_parts(
                diqm.pDescription,
                diqm.DescriptionByteLength - 1
            ))
        );
    }
    diq.ClearStoredMessages(DXGI_DEBUG_ALL);
}

fn handle_message(window: HWND) -> bool {
    unsafe {
        let mut msg = MaybeUninit::uninit();
        if GetMessageA(msg.as_mut_ptr(), window, 0, 0).0 > 0 {
            TranslateMessage(msg.as_ptr());
            DispatchMessageA(msg.as_ptr());
            msg.as_ptr()
                .as_ref()
                .map(|m| m.message != WM_QUIT && m.message != WM_CLOSE)
                .unwrap_or(true)
        } else {
            false
        }
    }
}

fn run(hwnd: HWND) -> Result<()> {
    let mut dcomp = unsafe { Dcomp::new(hwnd)? };

    loop {
        unsafe { dcomp.render()? };
        if !handle_message(hwnd) {
            break;
        }
    }

    Ok(())
}
