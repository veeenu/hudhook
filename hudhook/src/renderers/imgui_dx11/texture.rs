use log::trace;
use windows::core::Error;
use windows::Win32::Graphics::Direct3D::D3D11_SRV_DIMENSION_TEXTURE2D;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D, D3D11_BIND_SHADER_RESOURCE,
    D3D11_COMPARISON_ALWAYS, D3D11_CPU_ACCESS_FLAG, D3D11_FILTER_MIN_MAG_MIP_LINEAR,
    D3D11_RESOURCE_MISC_FLAG, D3D11_SAMPLER_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC,
    D3D11_SHADER_RESOURCE_VIEW_DESC_0, D3D11_SUBRESOURCE_DATA, D3D11_TEX2D_SRV,
    D3D11_TEXTURE2D_DESC, D3D11_TEXTURE_ADDRESS_WRAP, D3D11_USAGE_DEFAULT,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};

use super::device_and_swapchain::DeviceAndSwapChain;

pub(crate) struct Texture {
    _tex: ID3D11Texture2D,
    tex_view: ID3D11ShaderResourceView,
    _font_sampler: ID3D11SamplerState,
}

impl Texture {
    // TODO FontAtlasTexture may be too specific?
    pub(crate) fn new(
        dasc: &DeviceAndSwapChain,
        fonts: &mut imgui::FontAtlasRefMut,
    ) -> Result<Texture, Error> {
        let texture = fonts.build_rgba32_texture();
        let data = texture.data.to_vec();

        let tex = unsafe {
            dasc.dev().CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: texture.width as _,
                    Height: texture.height as _,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE,
                    CPUAccessFlags: D3D11_CPU_ACCESS_FLAG(0),
                    MiscFlags: D3D11_RESOURCE_MISC_FLAG(0),
                } as *const _,
                &D3D11_SUBRESOURCE_DATA {
                    pSysMem: data.as_ptr() as _,
                    SysMemPitch: texture.width * 4,
                    SysMemSlicePitch: 0,
                } as *const _,
            )?
        };

        let tex_view = unsafe {
            dasc.dev().CreateShaderResourceView(&tex, &D3D11_SHADER_RESOURCE_VIEW_DESC {
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                ViewDimension: D3D11_SRV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_SRV { MostDetailedMip: 0, MipLevels: 1 },
                },
            } as *const _)?
        };

        let _font_sampler = unsafe {
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

        fonts.tex_id = imgui::TextureId::from(&tex_view as *const _ as usize);
        trace!("Texture view: {:x} id: {:x}", &tex_view as *const _ as usize, fonts.tex_id.id());

        Ok(Texture { _tex: tex, tex_view, _font_sampler })
    }

    pub(crate) fn tex_view(&self) -> ID3D11ShaderResourceView {
        self.tex_view.clone()
    }
}
