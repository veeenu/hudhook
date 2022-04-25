use std::ptr::{null_mut, NonNull};

use imgui::{DrawListIterator, DrawVert};
use winapi::um::d3d11::*;

use crate::common::check_hresult;
use crate::device_and_swapchain::DeviceAndSwapChain;

pub(crate) struct Buffers {
    vtx_buffer: NonNull<ID3D11Buffer>,
    idx_buffer: NonNull<ID3D11Buffer>,
    mtx_buffer: NonNull<ID3D11Buffer>,

    vtx_count: usize,
    idx_count: usize,
}

#[repr(transparent)]
struct VertexConstantBuffer([[f32; 4]; 4]);

impl Buffers {
    pub(crate) fn new(dasc: &DeviceAndSwapChain) -> Self {
        let mut mtx_buffer = null_mut();

        check_hresult(unsafe {
            dasc.dev().CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: std::mem::size_of::<VertexConstantBuffer>() as u32,
                    Usage: D3D11_USAGE_DYNAMIC,
                    BindFlags: D3D11_BIND_CONSTANT_BUFFER,
                    CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
                    MiscFlags: 0,
                    StructureByteStride: 0,
                } as *const _,
                null_mut(),
                &mut mtx_buffer as _,
            )
        });

        let vtx_buffer = Buffers::create_vertex_buffer(dasc, 1);
        let idx_buffer = Buffers::create_index_buffer(dasc, 1);

        Buffers {
            vtx_buffer,
            idx_buffer,
            mtx_buffer: NonNull::new(mtx_buffer).expect("Null matrix buffer"),
            vtx_count: 1,
            idx_count: 1,
        }
    }

    pub(crate) fn set_constant_buffer(&mut self, dasc: &DeviceAndSwapChain, rect: [f32; 4]) {
        let [l, t, r, b] = rect;

        dasc.with_mapped(self.mtx_buffer, |ms| unsafe {
            std::ptr::copy_nonoverlapping(
                &VertexConstantBuffer([
                    [2. / (r - l), 0., 0., 0.],
                    [0., 2. / (t - b), 0., 0.],
                    [0., 0., 0.5, 0.],
                    [(r + l) / (l - r), (t + b) / (b - t), 0.5, 1.0],
                ]),
                ms.pData as *mut _,
                1,
            );
        })
    }

    pub(crate) fn set_buffers(&mut self, dasc: &DeviceAndSwapChain, meshes: DrawListIterator) {
        let (vertices, indices): (Vec<DrawVert>, Vec<u16>) = meshes
            .map(|m| (m.vtx_buffer().iter(), m.idx_buffer().iter()))
            .fold((Vec::new(), Vec::new()), |(mut ov, mut oi), (v, i)| {
                ov.extend(v);
                oi.extend(i);
                (ov, oi)
            });

        self.resize(dasc, vertices.len(), indices.len());

        dasc.with_mapped(self.vtx_buffer, |ms| unsafe {
            std::ptr::copy_nonoverlapping(vertices.as_ptr(), ms.pData as _, vertices.len());
        });

        dasc.with_mapped(self.idx_buffer, |ms| unsafe {
            std::ptr::copy_nonoverlapping(indices.as_ptr(), ms.pData as _, indices.len());
        });
    }

    pub(crate) fn resize(&mut self, dasc: &DeviceAndSwapChain, vtx_count: usize, idx_count: usize) {
        if self.vtx_count <= vtx_count || (self.vtx_count == 0 && vtx_count == 0) {
            unsafe { self.vtx_buffer.as_ref().Release() };
            self.vtx_count = vtx_count + 4096;
            self.vtx_buffer = Buffers::create_vertex_buffer(dasc, self.vtx_count);
        }

        if self.idx_count <= idx_count || (self.idx_count == 0 && idx_count == 0) {
            unsafe { self.idx_buffer.as_ref().Release() };
            self.idx_count = idx_count + 4096;
            self.idx_buffer = Buffers::create_index_buffer(dasc, self.idx_count);
        }
    }

    pub(crate) fn create_vertex_buffer(
        dasc: &DeviceAndSwapChain,
        size: usize,
    ) -> NonNull<ID3D11Buffer> {
        let mut buffer = null_mut();

        check_hresult(unsafe {
            dasc.dev().CreateBuffer(
                &D3D11_BUFFER_DESC {
                    Usage: D3D11_USAGE_DYNAMIC,
                    ByteWidth: (size * std::mem::size_of::<imgui::DrawVert>()) as u32,
                    BindFlags: D3D11_BIND_VERTEX_BUFFER,
                    CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
                    MiscFlags: 0,
                    StructureByteStride: 0,
                },
                null_mut(),
                &mut buffer as *mut _,
            )
        });

        NonNull::new(buffer).expect("Null vertex buffer")
    }

    pub(crate) fn create_index_buffer(
        dasc: &DeviceAndSwapChain,
        size: usize,
    ) -> NonNull<ID3D11Buffer> {
        let mut buffer = null_mut();

        check_hresult(unsafe {
            dasc.dev().CreateBuffer(
                &D3D11_BUFFER_DESC {
                    Usage: D3D11_USAGE_DYNAMIC,
                    ByteWidth: (size * std::mem::size_of::<u32>()) as u32,
                    BindFlags: D3D11_BIND_INDEX_BUFFER,
                    CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
                    MiscFlags: 0,
                    StructureByteStride: 0,
                },
                null_mut(),
                &mut buffer as *mut _,
            )
        });

        NonNull::new(buffer).expect("Null index buffer")
    }

    pub(crate) fn vtx_buffer(&self) -> *mut ID3D11Buffer {
        self.vtx_buffer.as_ptr()
    }

    pub(crate) fn idx_buffer(&self) -> *mut ID3D11Buffer {
        self.idx_buffer.as_ptr()
    }

    pub(crate) fn mtx_buffer(&self) -> *mut ID3D11Buffer {
        self.mtx_buffer.as_ptr()
    }
}
