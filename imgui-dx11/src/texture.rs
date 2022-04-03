use crate::common::check_hresult;
use crate::device_and_swapchain::DeviceAndSwapChain;

use std::ptr::{null_mut, NonNull};

use winapi::shared::dxgiformat::*;
use winapi::shared::dxgitype::DXGI_SAMPLE_DESC;
use winapi::um::d3d11::*;
use winapi::um::d3dcommon::D3D11_SRV_DIMENSION_TEXTURE2D;

pub(crate) struct Texture {
    tex: NonNull<ID3D11Texture2D>,
    tex_view: NonNull<ID3D11ShaderResourceView>,
    font_sampler: NonNull<ID3D11SamplerState>,
}

impl Texture {
    // TODO FontAtlasTexture may be too specific?
    pub(crate) fn new(dasc: &DeviceAndSwapChain, fonts: &mut imgui::FontAtlasRefMut) -> Texture {
        let texture = fonts.build_rgba32_texture();
        let mut tex: *mut ID3D11Texture2D = null_mut();
        let mut tex_view = null_mut();
        let mut font_sampler = null_mut();
        let data = texture.data.to_vec();

        check_hresult(unsafe {
            dasc.dev().CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: texture.width as _,
                    Height: texture.height as _,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE,
                    CPUAccessFlags: 0,
                    MiscFlags: 0,
                } as *const _,
                &D3D11_SUBRESOURCE_DATA {
                    pSysMem: data.as_ptr() as _,
                    SysMemPitch: texture.width * 4,
                    SysMemSlicePitch: 0,
                } as *const _,
                &mut tex as *mut *mut _,
            )
        });

        let mut srv_desc_u: D3D11_SHADER_RESOURCE_VIEW_DESC_u = unsafe { std::mem::zeroed() };
        unsafe { srv_desc_u.Texture2D_mut() }.MipLevels = 1;

        check_hresult(unsafe {
            dasc.dev().CreateShaderResourceView(
                tex as _,
                &D3D11_SHADER_RESOURCE_VIEW_DESC {
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    ViewDimension: D3D11_SRV_DIMENSION_TEXTURE2D,
                    u: srv_desc_u,
                } as *const _,
                &mut tex_view as *mut *mut _,
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
                },
                &mut font_sampler as *mut *mut _,
            )
        });

        fonts.tex_id = imgui::TextureId::from(tex_view);

        Texture {
            tex: NonNull::new(tex).expect("Null texture"),
            tex_view: NonNull::new(tex_view).expect("Null texture view"),
            font_sampler: NonNull::new(font_sampler).expect("Null font sampler"),
        }
    }

    pub(crate) fn tex_view(&self) -> *mut ID3D11ShaderResourceView {
        self.tex_view.as_ptr()
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        unsafe {
            self.tex.as_ref().Release();
            self.tex_view.as_ref().Release();
            self.font_sampler.as_ref().Release();
        }
    }
}
