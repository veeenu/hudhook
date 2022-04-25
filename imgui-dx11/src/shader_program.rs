use crate::common::check_hresult;
use crate::device_and_swapchain::*;

use std::ptr::{null, null_mut, NonNull};

use winapi::shared::dxgiformat::*;
use winapi::um::d3d11::*;
use winapi::um::d3dcommon::ID3D10Blob;
use winapi::um::d3dcompiler::D3DCompile;

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
    vtx_shader: NonNull<ID3D11VertexShader>,
    pix_shader: NonNull<ID3D11PixelShader>,
    layout: NonNull<ID3D11InputLayout>,
    sampler: NonNull<ID3D11SamplerState>,
    rasterizer_state: NonNull<ID3D11RasterizerState>,
    blend_state: NonNull<ID3D11BlendState>,
    depth_stencil_state: NonNull<ID3D11DepthStencilState>,
}

impl ShaderProgram {
    pub(crate) fn new(dasc: &DeviceAndSwapChain) -> ShaderProgram {
        let mut vs_blob: *mut ID3D10Blob = null_mut(); // TODO release?
        let mut ps_blob: *mut ID3D10Blob = null_mut(); // TODO release?
        let mut vtx_shader: *mut ID3D11VertexShader = null_mut();
        let mut pix_shader: *mut ID3D11PixelShader = null_mut();
        let mut layout: *mut ID3D11InputLayout = null_mut();
        let mut sampler: *mut ID3D11SamplerState = null_mut();
        let mut rasterizer_state: *mut ID3D11RasterizerState = null_mut();
        let mut blend_state: *mut ID3D11BlendState = null_mut();
        let mut depth_stencil_state: *mut ID3D11DepthStencilState = null_mut();

        check_hresult(unsafe {
            D3DCompile(
                VERTEX_SHADER_SRC.as_ptr() as _,
                VERTEX_SHADER_SRC.len(),
                null_mut(),
                null_mut(),
                null_mut(),
                "main\0".as_ptr() as _,
                "vs_4_0\0".as_ptr() as _,
                0,
                0,
                &mut vs_blob as *mut _ as _,
                null_mut(),
            )
        });

        check_hresult(unsafe {
            D3DCompile(
                PIXEL_SHADER_SRC.as_ptr() as _,
                PIXEL_SHADER_SRC.len(),
                null_mut(),
                null_mut(),
                null_mut(),
                "main\0".as_ptr() as _,
                "ps_4_0\0".as_ptr() as _,
                0,
                0,
                &mut ps_blob as *mut _ as _,
                null_mut(),
            )
        });

        check_hresult(unsafe {
            dasc.dev().CreateVertexShader(
                vs_blob.as_ref().unwrap().GetBufferPointer(),
                vs_blob.as_ref().unwrap().GetBufferSize(),
                null_mut(),
                &mut vtx_shader as *mut *mut _ as _,
            )
        });

        check_hresult(unsafe {
            dasc.dev().CreatePixelShader(
                ps_blob.as_ref().unwrap().GetBufferPointer(),
                ps_blob.as_ref().unwrap().GetBufferSize(),
                null_mut(),
                &mut pix_shader as *mut *mut _ as _,
            )
        });

        check_hresult(unsafe {
            dasc.dev().CreateInputLayout(
                &[
                    D3D11_INPUT_ELEMENT_DESC {
                        SemanticName: "POSITION\0".as_ptr() as _,
                        SemanticIndex: 0,
                        Format: DXGI_FORMAT_R32G32_FLOAT,
                        InputSlot: 0,
                        AlignedByteOffset: 0,
                        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                        InstanceDataStepRate: 0,
                    },
                    D3D11_INPUT_ELEMENT_DESC {
                        SemanticName: "TEXCOORD\0".as_ptr() as _,
                        SemanticIndex: 0,
                        Format: DXGI_FORMAT_R32G32_FLOAT,
                        InputSlot: 0,
                        AlignedByteOffset: 8,
                        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                        InstanceDataStepRate: 0,
                    },
                    D3D11_INPUT_ELEMENT_DESC {
                        SemanticName: "COLOR\0".as_ptr() as _,
                        SemanticIndex: 0,
                        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                        InputSlot: 0,
                        AlignedByteOffset: 16,
                        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                        InstanceDataStepRate: 0,
                    },
                ] as *const _,
                3,
                vs_blob.as_ref().expect("Null VS blob").GetBufferPointer(),
                vs_blob.as_ref().expect("Null VS blob").GetBufferSize(),
                &mut layout as *mut *mut _ as _,
            )
        });

        check_hresult(unsafe {
            dasc.dev().CreateSamplerState(
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
                } as *const _,
                &mut sampler as *mut _,
            )
        });

        check_hresult(unsafe {
            dasc.dev().CreateBlendState(
                &D3D11_BLEND_DESC {
                    AlphaToCoverageEnable: 0,
                    IndependentBlendEnable: 0,
                    RenderTarget: [
                        D3D11_RENDER_TARGET_BLEND_DESC {
                            BlendEnable: 1,
                            SrcBlend: D3D11_BLEND_SRC_ALPHA,
                            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
                            BlendOp: D3D11_BLEND_OP_ADD,
                            SrcBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
                            DestBlendAlpha: D3D11_BLEND_ZERO,
                            BlendOpAlpha: D3D11_BLEND_OP_ADD,
                            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL as u8,
                        },
                        std::mem::zeroed(),
                        std::mem::zeroed(),
                        std::mem::zeroed(),
                        std::mem::zeroed(),
                        std::mem::zeroed(),
                        std::mem::zeroed(),
                        std::mem::zeroed(),
                    ],
                } as *const _,
                &mut blend_state as *mut _,
            )
        });

        check_hresult(unsafe {
            dasc.dev().CreateRasterizerState(
                &D3D11_RASTERIZER_DESC {
                    FillMode: D3D11_FILL_SOLID,
                    CullMode: D3D11_CULL_NONE,
                    ScissorEnable: 1,
                    DepthClipEnable: 1,
                    DepthBias: 0,
                    DepthBiasClamp: 0.,
                    SlopeScaledDepthBias: 0.,
                    MultisampleEnable: 0,
                    AntialiasedLineEnable: 0,
                    FrontCounterClockwise: 0,
                },
                &mut rasterizer_state as *mut _,
            )
        });

        check_hresult(unsafe {
            dasc.dev().CreateDepthStencilState(
                &D3D11_DEPTH_STENCIL_DESC {
                    DepthEnable: 0,
                    DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL,
                    DepthFunc: D3D11_COMPARISON_ALWAYS,
                    StencilEnable: 0,
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
                &mut depth_stencil_state as *mut _,
            )
        });

        ShaderProgram {
            vtx_shader: NonNull::new(vtx_shader).expect("Null vertex shader"),
            pix_shader: NonNull::new(pix_shader).expect("Null pixel shader"),
            layout: NonNull::new(layout).expect("Null input layout"),
            sampler: NonNull::new(sampler).expect("Null sampler"),
            blend_state: NonNull::new(blend_state).expect("Null blend state"),
            depth_stencil_state: NonNull::new(depth_stencil_state)
                .expect("Null depth stencil state"),
            rasterizer_state: NonNull::new(rasterizer_state).expect("Null rasterizer state"),
        }
    }

    pub(crate) unsafe fn set_state(&self, dasc: &DeviceAndSwapChain) {
        dasc.dev_ctx()
            .VSSetShader(self.vtx_shader.as_ptr(), null(), 0);
        dasc.dev_ctx()
            .PSSetShader(self.pix_shader.as_ptr(), null(), 0);
        dasc.dev_ctx().IASetInputLayout(self.layout.as_ptr());
        dasc.dev_ctx().PSSetSamplers(0, 1, &self.sampler.as_ptr());
        dasc.dev_ctx()
            .OMSetBlendState(self.blend_state.as_ptr(), &[0f32; 4], 0xFFFFFFFF);
        dasc.dev_ctx()
            .OMSetDepthStencilState(self.depth_stencil_state.as_ptr(), 0);
        dasc.dev_ctx().RSSetState(self.rasterizer_state.as_ptr());
    }
}

impl Drop for ShaderProgram {
    fn drop(&mut self) {
        unsafe {
            self.layout.as_ref().Release();
            self.vtx_shader.as_ref().Release();
            self.pix_shader.as_ref().Release();

            self.sampler.as_ref().Release();
            self.blend_state.as_ref().Release();
            self.depth_stencil_state.as_ref().Release();
            self.rasterizer_state.as_ref().Release();
        }
    }
}
