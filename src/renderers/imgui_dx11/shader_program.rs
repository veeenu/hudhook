use std::ptr::null_mut;

use windows::core::{Error, PCSTR};
use windows::Win32::Foundation::BOOL;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::ID3DBlob;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11BlendState, ID3D11DepthStencilState, ID3D11InputLayout, ID3D11PixelShader,
    ID3D11RasterizerState, ID3D11SamplerState, ID3D11VertexShader, D3D11_BLEND_DESC,
    D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_OP_ADD, D3D11_BLEND_SRC_ALPHA, D3D11_BLEND_ZERO,
    D3D11_COLOR_WRITE_ENABLE_ALL, D3D11_COMPARISON_ALWAYS, D3D11_CULL_NONE,
    D3D11_DEPTH_STENCILOP_DESC, D3D11_DEPTH_STENCIL_DESC, D3D11_DEPTH_WRITE_MASK_ALL,
    D3D11_FILL_SOLID, D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_INPUT_ELEMENT_DESC,
    D3D11_INPUT_PER_VERTEX_DATA, D3D11_RASTERIZER_DESC, D3D11_RENDER_TARGET_BLEND_DESC,
    D3D11_SAMPLER_DESC, D3D11_STENCIL_OP_KEEP, D3D11_TEXTURE_ADDRESS_WRAP,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R8G8B8A8_UNORM,
};

use super::device_and_swapchain::*;

const VERTEX_SHADER_SRC: &str = r"
  cbuffer vertexBuffer : register(b0) {
    float4x4 ProjectionMatrix;
  };
  struct VS_INPUT {
    float2 pos : POSITION;
    float4 col : COLOR0;
    float2 uv  : TEXCOORD0;
  };
  struct PS_INPUT {
    float4 pos : SV_POSITION;
    float4 col : COLOR0;
    float2 uv  : TEXCOORD0;
  };
  PS_INPUT main(VS_INPUT input) {
    PS_INPUT output;
    output.pos = mul(ProjectionMatrix, float4(input.pos.xy, 0.f, 1.f));
    output.col = input.col;
    output.uv  = input.uv;
    return output;
  }
";

const PIXEL_SHADER_SRC: &str = r"
  struct PS_INPUT {
    float4 pos : SV_POSITION;
    float4 col : COLOR0;
    float2 uv  : TEXCOORD0;
  };
  sampler sampler0;
  Texture2D texture0;
  float4 main(PS_INPUT input) : SV_Target {
    float4 out_col = input.col * texture0.Sample(sampler0, input.uv);
    return out_col;
  };
";

pub(crate) struct ShaderProgram {
    vtx_shader: ID3D11VertexShader,
    pix_shader: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    sampler: ID3D11SamplerState,
    rasterizer_state: ID3D11RasterizerState,
    blend_state: ID3D11BlendState,
    depth_stencil_state: ID3D11DepthStencilState,
}

impl ShaderProgram {
    pub(crate) fn new(dasc: &DeviceAndSwapChain) -> Result<ShaderProgram, Error> {
        let mut vs_blob: Option<ID3DBlob> = None;
        let mut ps_blob: Option<ID3DBlob> = None;

        unsafe {
            D3DCompile(
                VERTEX_SHADER_SRC.as_ptr() as _,
                VERTEX_SHADER_SRC.len(),
                None,
                null_mut(),
                None,
                PCSTR("main\0".as_ptr() as _),
                PCSTR("vs_4_0\0".as_ptr() as _),
                0,
                0,
                &mut vs_blob,
                &mut None,
            )?
        };

        unsafe {
            D3DCompile(
                PIXEL_SHADER_SRC.as_ptr() as _,
                PIXEL_SHADER_SRC.len(),
                None,
                null_mut(),
                None,
                PCSTR("main\0".as_ptr() as _),
                PCSTR("ps_4_0\0".as_ptr() as _),
                0,
                0,
                &mut ps_blob as *mut _ as _,
                &mut None,
            )?
        };

        let vtx_shader = unsafe {
            let vs_blob = vs_blob.as_ref().unwrap();
            let ptr = vs_blob.GetBufferPointer();
            let size = vs_blob.GetBufferSize();
            dasc.dev().CreateVertexShader(std::slice::from_raw_parts(ptr as _, size), None)?
        };

        let pix_shader = unsafe {
            let ps_blob = ps_blob.as_ref().unwrap();
            let ptr = ps_blob.GetBufferPointer();
            let size = ps_blob.GetBufferSize();
            dasc.dev().CreatePixelShader(std::slice::from_raw_parts(ptr as _, size), None)?
        };

        let layout = unsafe {
            let vs_blob = vs_blob.as_ref().unwrap();
            let ptr = vs_blob.GetBufferPointer();
            let size = vs_blob.GetBufferSize();
            dasc.dev().CreateInputLayout(
                &[
                    D3D11_INPUT_ELEMENT_DESC {
                        SemanticName: PCSTR("POSITION\0".as_ptr() as _),
                        SemanticIndex: 0,
                        Format: DXGI_FORMAT_R32G32_FLOAT,
                        InputSlot: 0,
                        AlignedByteOffset: 0,
                        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                        InstanceDataStepRate: 0,
                    },
                    D3D11_INPUT_ELEMENT_DESC {
                        SemanticName: PCSTR("TEXCOORD\0".as_ptr() as _),
                        SemanticIndex: 0,
                        Format: DXGI_FORMAT_R32G32_FLOAT,
                        InputSlot: 0,
                        AlignedByteOffset: 8,
                        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                        InstanceDataStepRate: 0,
                    },
                    D3D11_INPUT_ELEMENT_DESC {
                        SemanticName: PCSTR("COLOR\0".as_ptr() as _),
                        SemanticIndex: 0,
                        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                        InputSlot: 0,
                        AlignedByteOffset: 16,
                        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                        InstanceDataStepRate: 0,
                    },
                ],
                std::slice::from_raw_parts(ptr as _, size),
            )?
        };

        let sampler = unsafe {
            dasc.dev().CreateSamplerState(&D3D11_SAMPLER_DESC {
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
            })?
        };

        let blend_state = unsafe {
            dasc.dev().CreateBlendState(&D3D11_BLEND_DESC {
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
            } as *const _)?
        };

        let rasterizer_state = unsafe {
            dasc.dev().CreateRasterizerState(&D3D11_RASTERIZER_DESC {
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
            })?
        };

        let depth_stencil_state = unsafe {
            dasc.dev().CreateDepthStencilState(&D3D11_DEPTH_STENCIL_DESC {
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
            })?
        };

        Ok(ShaderProgram {
            vtx_shader,
            pix_shader,
            layout,
            sampler,
            blend_state,
            depth_stencil_state,
            rasterizer_state,
        })
    }

    pub(crate) unsafe fn set_state(&self, dasc: &DeviceAndSwapChain) {
        dasc.dev_ctx().VSSetShader(&self.vtx_shader, &[]);
        dasc.dev_ctx().PSSetShader(&self.pix_shader, &[]);
        dasc.dev_ctx().IASetInputLayout(&self.layout);
        dasc.dev_ctx().PSSetSamplers(0, &[Some(self.sampler.clone())]);
        dasc.dev_ctx().OMSetBlendState(&self.blend_state, &[0f32; 4] as _, 0xFFFFFFFF);
        dasc.dev_ctx().OMSetDepthStencilState(&self.depth_stencil_state, 0);
        dasc.dev_ctx().RSSetState(&self.rasterizer_state);
    }
}
