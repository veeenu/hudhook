use std::{mem, ptr, slice};

use once_cell::sync::OnceCell;
use windows::core::{s, w, ComInterface, Result};
use windows::Win32::Foundation::{CloseHandle, BOOL, GENERIC_ALL, HANDLE, RECT};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D11_SRV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11BlendState, ID3D11Buffer, ID3D11DepthStencilState, ID3D11Device1, ID3D11DeviceContext,
    ID3D11InputLayout, ID3D11PixelShader, ID3D11RasterizerState, ID3D11RenderTargetView,
    ID3D11Resource, ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D,
    ID3D11VertexShader, D3D11_APPEND_ALIGNED_ELEMENT, D3D11_BIND_INDEX_BUFFER,
    D3D11_BIND_VERTEX_BUFFER, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_OP_ADD,
    D3D11_BLEND_SRC_ALPHA, D3D11_BLEND_ZERO, D3D11_BUFFER_DESC, D3D11_COLOR_WRITE_ENABLE_ALL,
    D3D11_COMPARISON_ALWAYS, D3D11_CPU_ACCESS_WRITE, D3D11_CULL_NONE, D3D11_DEPTH_STENCILOP_DESC,
    D3D11_DEPTH_STENCIL_DESC, D3D11_DEPTH_WRITE_MASK_ALL, D3D11_FILL_SOLID,
    D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_INPUT_ELEMENT_DESC, D3D11_INPUT_PER_VERTEX_DATA,
    D3D11_MAP_WRITE_DISCARD, D3D11_RASTERIZER_DESC, D3D11_RENDER_TARGET_BLEND_DESC,
    D3D11_SAMPLER_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC_0,
    D3D11_STENCIL_OP_KEEP, D3D11_SUBRESOURCE_DATA, D3D11_TEX2D_SRV, D3D11_TEXTURE_ADDRESS_WRAP,
    D3D11_USAGE_DYNAMIC, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Direct3D12::ID3D12Resource;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R16_UINT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_UNKNOWN,
};
use windows::Win32::Graphics::Dxgi::{IDXGISwapChain, DXGI_SWAP_CHAIN_DESC};

use crate::renderer::RenderedSurface;
use crate::util::{try_out_param, try_out_ptr};

pub struct Compositor {
    device: ID3D11Device1,
    device_ctx: ID3D11DeviceContext,
    quad_renderer: OnceCell<QuadRenderer>,
}

impl Compositor {
    pub fn new(device: &ID3D11Device1) -> Result<Self> {
        Ok(Self {
            device: device.clone(),
            device_ctx: unsafe { device.GetImmediateContext()? },
            quad_renderer: OnceCell::new(),
        })
    }

    pub fn composite(&mut self, handle: HANDLE, target: &IDXGISwapChain) -> Result<()> {
        let resource: ID3D11Texture2D = unsafe { self.device.OpenSharedResource1(handle) }?;
        unsafe { self.render_quad(resource, target) }?;

        unsafe { CloseHandle(handle)? };

        Ok(())
    }

    unsafe fn quad_renderer(&mut self) -> Result<&QuadRenderer> {
        self.quad_renderer.get_or_try_init(|| QuadRenderer::new(&self.device, &self.device_ctx))
    }

    unsafe fn render_quad(
        &mut self,
        texture: ID3D11Texture2D,
        target: &IDXGISwapChain,
    ) -> Result<()> {
        let device = self.device.clone();
        let ctx = self.device_ctx.clone();
        let quad_renderer = self.quad_renderer()?;

        quad_renderer.setup_state(&ctx);
        quad_renderer.render(&device, &ctx, texture, target)?;

        Ok(())
    }
}

#[repr(C)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
}

struct QuadRenderer {
    vtx_shader: ID3D11VertexShader,
    pix_shader: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    sampler: ID3D11SamplerState,
    rasterizer_state: ID3D11RasterizerState,
    blend_state: ID3D11BlendState,
    depth_stencil_state: ID3D11DepthStencilState,
    vertex_buffer: Option<ID3D11Buffer>,
    index_buffer: ID3D11Buffer,
}

impl QuadRenderer {
    unsafe fn new(d3d11: &ID3D11Device1, d3d11_ctx: &ID3D11DeviceContext) -> Result<Self> {
        const VERTICES: [Vertex; 4] = [
            Vertex { pos: [-1., 1.], uv: [0., 0.] },
            Vertex { pos: [1., 1.], uv: [1., 0.] },
            Vertex { pos: [-1., -1.], uv: [0., 1.] },
            Vertex { pos: [1., -1.], uv: [1., 1.] },
        ];

        const INDICES: [u16; 6] = [0, 1, 2, 1, 3, 2];

        const VERTEX_SHADER_SRC: &str = r"
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
              output.uv  = input.uv.xy;
              return output;
            }
        ";

        const PIXEL_SHADER_SRC: &str = r"
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
            }
        ";

        let vs_blob: ID3DBlob = try_out_ptr(|v| unsafe {
            D3DCompile(
                VERTEX_SHADER_SRC.as_ptr() as _,
                VERTEX_SHADER_SRC.len(),
                None,
                None,
                None,
                s!("main\0"),
                s!("vs_4_0\0"),
                0,
                0,
                v,
                None,
            )
        })?;

        let ps_blob = try_out_ptr(|v| unsafe {
            D3DCompile(
                PIXEL_SHADER_SRC.as_ptr() as _,
                PIXEL_SHADER_SRC.len(),
                None,
                None,
                None,
                s!("main\0"),
                s!("ps_4_0\0"),
                0,
                0,
                v,
                None,
            )
        })?;

        let vtx_shader = try_out_ptr(|v| unsafe {
            let ptr = vs_blob.GetBufferPointer();
            let size = vs_blob.GetBufferSize();
            d3d11.CreateVertexShader(slice::from_raw_parts(ptr as _, size), None, Some(v))
        })?;

        let pix_shader = try_out_ptr(|v| unsafe {
            let ptr = ps_blob.GetBufferPointer();
            let size = ps_blob.GetBufferSize();
            d3d11.CreatePixelShader(slice::from_raw_parts(ptr as _, size), None, Some(v))
        })?;

        let layout = try_out_ptr(|v| unsafe {
            let ptr = vs_blob.GetBufferPointer();
            let size = vs_blob.GetBufferSize();
            d3d11.CreateInputLayout(
                &[
                    D3D11_INPUT_ELEMENT_DESC {
                        SemanticName: s!("POSITION"),
                        SemanticIndex: 0,
                        Format: DXGI_FORMAT_R32G32_FLOAT,
                        InputSlot: 0,
                        AlignedByteOffset: 0,
                        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                        InstanceDataStepRate: 0,
                    },
                    D3D11_INPUT_ELEMENT_DESC {
                        SemanticName: s!("TEXCOORD"),
                        SemanticIndex: 0,
                        Format: DXGI_FORMAT_R32G32_FLOAT,
                        InputSlot: 0,
                        AlignedByteOffset: D3D11_APPEND_ALIGNED_ELEMENT,
                        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                        InstanceDataStepRate: 0,
                    },
                ],
                slice::from_raw_parts(ptr as _, size),
                Some(v),
            )
        })?;

        let sampler = try_out_ptr(|v| unsafe {
            d3d11.CreateSamplerState(
                &D3D11_SAMPLER_DESC {
                    Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                    AddressU: D3D11_TEXTURE_ADDRESS_WRAP,
                    AddressV: D3D11_TEXTURE_ADDRESS_WRAP,
                    AddressW: D3D11_TEXTURE_ADDRESS_WRAP,
                    MipLODBias: 0.,
                    ComparisonFunc: D3D11_COMPARISON_ALWAYS,
                    MinLOD: 0.,
                    MaxLOD: 0.,
                    BorderColor: [0.; 4],
                    MaxAnisotropy: 0,
                },
                Some(v),
            )
        })?;
        let blend_state = try_out_ptr(|v| unsafe {
            d3d11.CreateBlendState(
                &D3D11_BLEND_DESC {
                    AlphaToCoverageEnable: BOOL(0),
                    IndependentBlendEnable: BOOL(0),
                    RenderTarget: [
                        D3D11_RENDER_TARGET_BLEND_DESC {
                            BlendEnable: BOOL(1),
                            SrcBlend: D3D11_BLEND_SRC_ALPHA,
                            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
                            BlendOp: D3D11_BLEND_OP_ADD,
                            SrcBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
                            DestBlendAlpha: D3D11_BLEND_ZERO,
                            BlendOpAlpha: D3D11_BLEND_OP_ADD,
                            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as _,
                        },
                        std::mem::zeroed(),
                        std::mem::zeroed(),
                        std::mem::zeroed(),
                        std::mem::zeroed(),
                        std::mem::zeroed(),
                        std::mem::zeroed(),
                        std::mem::zeroed(),
                    ],
                },
                Some(v),
            )
        })?;

        let rasterizer_state = try_out_ptr(|v| unsafe {
            d3d11.CreateRasterizerState(
                &D3D11_RASTERIZER_DESC {
                    FillMode: D3D11_FILL_SOLID,
                    CullMode: D3D11_CULL_NONE,
                    ScissorEnable: BOOL(1),
                    DepthClipEnable: BOOL(1),
                    DepthBias: 0,
                    DepthBiasClamp: 0.,
                    SlopeScaledDepthBias: 0.,
                    MultisampleEnable: BOOL(0),
                    AntialiasedLineEnable: BOOL(0),
                    FrontCounterClockwise: BOOL(0),
                },
                Some(v),
            )
        })?;

        let depth_stencil_state = try_out_ptr(|v| unsafe {
            d3d11.CreateDepthStencilState(
                &D3D11_DEPTH_STENCIL_DESC {
                    DepthEnable: BOOL(0),
                    DepthFunc: D3D11_COMPARISON_ALWAYS,
                    DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL,
                    StencilEnable: BOOL(0),
                    StencilReadMask: 0,
                    StencilWriteMask: 0,
                    FrontFace: D3D11_DEPTH_STENCILOP_DESC {
                        StencilFailOp: D3D11_STENCIL_OP_KEEP,
                        StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
                        StencilPassOp: D3D11_STENCIL_OP_KEEP,
                        StencilFunc: D3D11_COMPARISON_ALWAYS,
                    },
                    BackFace: D3D11_DEPTH_STENCILOP_DESC {
                        StencilFailOp: D3D11_STENCIL_OP_KEEP,
                        StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
                        StencilPassOp: D3D11_STENCIL_OP_KEEP,
                        StencilFunc: D3D11_COMPARISON_ALWAYS,
                    },
                },
                Some(v),
            )
        })?;

        let vertex_buffer: ID3D11Buffer = try_out_ptr(|v| {
            d3d11.CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: mem::size_of_val(&VERTICES) as _,
                    Usage: D3D11_USAGE_DYNAMIC,
                    BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as _,
                    CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as _,
                    MiscFlags: 0,
                    StructureByteStride: 0,
                },
                Some(&D3D11_SUBRESOURCE_DATA {
                    pSysMem: VERTICES.as_ptr() as *const _,
                    SysMemPitch: 0,
                    SysMemSlicePitch: 0,
                }),
                Some(v),
            )
        })?;

        let index_buffer: ID3D11Buffer = try_out_ptr(|v| {
            d3d11.CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: mem::size_of_val(&INDICES) as _,
                    Usage: D3D11_USAGE_DYNAMIC,
                    BindFlags: D3D11_BIND_INDEX_BUFFER.0 as _,
                    CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as _,
                    MiscFlags: 0,
                    StructureByteStride: 0,
                },
                Some(&D3D11_SUBRESOURCE_DATA {
                    pSysMem: INDICES.as_ptr() as *const _,
                    SysMemPitch: 0,
                    SysMemSlicePitch: 0,
                }),
                Some(v),
            )
        })?;

        let mut ms = Default::default();
        d3d11_ctx.Map(&vertex_buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut ms))?;
        ptr::copy_nonoverlapping(VERTICES.as_ptr(), ms.pData as _, VERTICES.len());
        d3d11_ctx.Unmap(&vertex_buffer, 0);

        let mut ms = Default::default();
        d3d11_ctx.Map(&index_buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut ms))?;
        ptr::copy_nonoverlapping(INDICES.as_ptr(), ms.pData as _, INDICES.len());
        d3d11_ctx.Unmap(&index_buffer, 0);

        Ok(Self {
            vtx_shader,
            pix_shader,
            layout,
            sampler,
            blend_state,
            depth_stencil_state,
            rasterizer_state,
            vertex_buffer: Some(vertex_buffer),
            index_buffer,
        })
    }

    unsafe fn setup_state(&self, d3d11_ctx: &ID3D11DeviceContext) {
        d3d11_ctx.VSSetShader(&self.vtx_shader, Some(&[]));
        d3d11_ctx.PSSetShader(&self.pix_shader, Some(&[]));
        d3d11_ctx.IASetInputLayout(&self.layout);
        d3d11_ctx.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
        d3d11_ctx.OMSetBlendState(&self.blend_state, Some(&[0f32; 4]), 0xFFFFFFFF);
        d3d11_ctx.OMSetDepthStencilState(&self.depth_stencil_state, 0);
        d3d11_ctx.RSSetState(&self.rasterizer_state);
        d3d11_ctx.IASetVertexBuffers(0, 1, Some(&self.vertex_buffer), Some(&16), Some(&0));
        d3d11_ctx.IASetIndexBuffer(&self.index_buffer, DXGI_FORMAT_R16_UINT, 0);
        d3d11_ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    }

    unsafe fn render(
        &self,
        d3d11: &ID3D11Device1,
        d3d11_ctx: &ID3D11DeviceContext,
        texture: ID3D11Texture2D,
        target: &IDXGISwapChain,
    ) -> Result<()> {
        let back_buffer: ID3D11Resource = target.GetBuffer(0)?;
        let back_buffer: ID3D11RenderTargetView =
            try_out_ptr(|v| d3d11.CreateRenderTargetView(&back_buffer, None, Some(v)))?;

        let texture = texture.cast::<ID3D11Resource>()?;
        let srv: ID3D11ShaderResourceView = try_out_ptr(|v| {
            d3d11.CreateShaderResourceView(
                &texture,
                Some(&D3D11_SHADER_RESOURCE_VIEW_DESC {
                    Format: DXGI_FORMAT_UNKNOWN,
                    ViewDimension: D3D11_SRV_DIMENSION_TEXTURE2D,
                    Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
                        Texture2D: D3D11_TEX2D_SRV { MostDetailedMip: 0, MipLevels: 1 },
                    },
                }),
                Some(v),
            )
        })?;

        let desc: DXGI_SWAP_CHAIN_DESC = try_out_param(|v| target.GetDesc(v))?;

        d3d11_ctx.RSSetViewports(Some(&[D3D11_VIEWPORT {
            TopLeftX: 0.,
            TopLeftY: 0.,
            Width: desc.BufferDesc.Width as f32,
            Height: desc.BufferDesc.Height as f32,
            MinDepth: 0.,
            MaxDepth: 1.,
        }]));
        d3d11_ctx.RSSetScissorRects(Some(&[RECT {
            left: 0,
            top: 0,
            right: desc.BufferDesc.Width as _,
            bottom: desc.BufferDesc.Height as _,
        }]));
        d3d11_ctx.PSSetShaderResources(0, Some(&[Some(srv)]));
        d3d11_ctx.OMSetRenderTargets(Some(&[Some(back_buffer)]), None);
        d3d11_ctx.DrawIndexed(6, 0, 0);

        Ok(())
    }
}
