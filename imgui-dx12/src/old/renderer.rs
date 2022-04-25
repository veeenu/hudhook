use std::ptr::null;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

pub struct ImguiDX12Renderer {
    hwnd: HWND,
    backend: crate::ImguiDX12,
    cmd_list: ID3D12GraphicsCommandList,
    render_targets: Vec<ID3D12Resource>,
}

impl ImguiDX12Renderer {
    pub fn new(hwnd: HWND) -> Self {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut rect) };

        let factory: IDXGIFactory4 = unsafe { CreateDXGIFactory() }.unwrap();
        let adapter: IDXGIAdapter = unsafe { factory.EnumAdapters(0) }.unwrap();
        let mut dev: Option<ID3D12Device> = None;
        unsafe { D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut dev) }.unwrap();
        let dev = dev.unwrap();

        let mut queue_desc = D3D12_COMMAND_QUEUE_DESC::default();
        queue_desc.Type = D3D12_COMMAND_LIST_TYPE_DIRECT;
        queue_desc.Priority = 0;
        queue_desc.Flags = D3D12_COMMAND_QUEUE_FLAG_NONE;
        queue_desc.NodeMask = 0;

        let cmd_queue: ID3D12CommandQueue = unsafe { dev.CreateCommandQueue(&queue_desc) }.unwrap();

        let swap_chain: IDXGISwapChain1 = unsafe {
            factory.CreateSwapChainForHwnd(
                &cmd_queue,
                hwnd,
                &DXGI_SWAP_CHAIN_DESC1 {
                    Width: (rect.right - rect.left) as u32,
                    Height: (rect.bottom - rect.top) as u32,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    Stereo: Default::default(),
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                    BufferCount: 2,
                    Scaling: Default::default(),
                    SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                    AlphaMode: Default::default(),
                    Flags: Default::default(),
                },
                null(),
                None,
            )
        }
        .unwrap();

        unsafe { factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER) }.unwrap();

        let rtv_heap: ID3D12DescriptorHeap = unsafe {
            dev.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                NumDescriptors: 2,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
                NodeMask: 0,
            })
        }
        .unwrap();

        let rtv_descriptor_size =
            unsafe { dev.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV) };

        let mut rtv_handle: D3D12_CPU_DESCRIPTOR_HANDLE =
            unsafe { rtv_heap.GetCPUDescriptorHandleForHeapStart() };

        let render_targets = (0..2)
            .map(|i| {
                let render_target: ID3D12Resource = unsafe { swap_chain.GetBuffer(i) }.unwrap();
                unsafe {
                    dev.CreateRenderTargetView(
                        &render_target,
                        &D3D12_RENDER_TARGET_VIEW_DESC::default(),
                        rtv_handle,
                    )
                };
                rtv_handle.ptr += rtv_descriptor_size as usize;
                render_target
            })
            .collect::<Vec<_>>();

        let cmd_allocator: ID3D12CommandAllocator =
            unsafe { dev.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }.unwrap();

        let cmd_list: ID3D12GraphicsCommandList = unsafe {
            dev.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, cmd_allocator, None)
        }
        .unwrap();

        let cpu_desc = unsafe { rtv_heap.GetCPUDescriptorHandleForHeapStart() };
        let gpu_desc = unsafe { rtv_heap.GetGPUDescriptorHandleForHeapStart() };

        let mut backend = crate::ImguiDX12::new(
            dev,
            2,
            DXGI_FORMAT_R8G8B8A8_UNORM,
            rtv_heap,
            cpu_desc,
            gpu_desc,
        );
        backend.create_device_objects();

        Self {
            hwnd,
            backend,
            cmd_list,
            render_targets,
        }
    }

    pub fn render<F: FnOnce(&mut imgui::Ui)>(&mut self, f: F) {
        {
            let mut rect = RECT::default();
            unsafe { GetWindowRect(self.hwnd, &mut rect) };

            self.backend.ctx().io_mut().display_size = [
                (rect.right - rect.left) as f32,
                (rect.bottom - rect.top) as f32,
            ];
        }

        self.backend.render_draw_data(f, &self.cmd_list);
    }
}

