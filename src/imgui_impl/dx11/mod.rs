use std::marker::PhantomData;
use std::ptr::{null, null_mut, NonNull};

use imgui;
use imgui::internal::RawWrapper;

use log::*;

use winapi::shared::dxgi::*;
use winapi::shared::dxgiformat::*;
use winapi::shared::dxgitype::*;
use winapi::um::d3d11::*;
use winapi::um::d3dcommon::*;
use winapi::Interface;

mod shaders;
mod state_backup;

use crate::util::*;
use shaders::*;
use state_backup::StateBackup;

pub struct DeviceObjects {
  font_sampler: *mut ID3D11SamplerState,
  texture_view: *mut ID3D11ShaderResourceView,
  blend_state: *mut ID3D11BlendState,
  depth_stencil_state: *mut ID3D11DepthStencilState,
  rasterizer_state: *mut ID3D11RasterizerState,
  pixel_shader: PixelShader,
  vertex_shader: VertexShader,
}

impl DeviceObjects {
  fn new(device: &mut ID3D11Device, ctx: &mut imgui::Context) -> Result<DeviceObjects> {
    let vertex_shader = VertexShader::new(device)?;
    let pixel_shader = PixelShader::new(device)?;

    let blend_state = {
      let desc: D3D11_BLEND_DESC = D3D11_BLEND_DESC {
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
          unsafe { std::mem::zeroed() },
          unsafe { std::mem::zeroed() },
          unsafe { std::mem::zeroed() },
          unsafe { std::mem::zeroed() },
          unsafe { std::mem::zeroed() },
          unsafe { std::mem::zeroed() },
          unsafe { std::mem::zeroed() },
        ],
      };

      let mut blend_state = null_mut();
      match unsafe { device.CreateBlendState(&desc as *const _, &mut blend_state as *mut *mut _) } {
        0 | 1 => Ok(blend_state),
        e => Err(format!("CreateBlendState error: {:x}", e)),
      }
    }?;

    let rasterizer_state = {
      let desc: D3D11_RASTERIZER_DESC = D3D11_RASTERIZER_DESC {
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
      };

      let mut rasterizer_state = null_mut();
      match unsafe {
        device.CreateRasterizerState(&desc as *const _, &mut rasterizer_state as *mut *mut _)
      } {
        0 | 1 => Ok(rasterizer_state),
        e => Err(format!("CreateRasterizerState error: {:?}", e)),
      }
    }?;

    let depth_stencil_state = {
      let ff = D3D11_DEPTH_STENCILOP_DESC {
        StencilFailOp: D3D11_STENCIL_OP_KEEP,
        StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
        StencilPassOp: D3D11_STENCIL_OP_KEEP,
        StencilFunc: D3D11_COMPARISON_ALWAYS,
      };
      let desc = D3D11_DEPTH_STENCIL_DESC {
        DepthEnable: 0,
        DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL,
        DepthFunc: D3D11_COMPARISON_ALWAYS,
        StencilEnable: 0,
        StencilReadMask: 0,
        StencilWriteMask: 0,
        FrontFace: ff,
        BackFace: ff.clone(),
      };

      let mut depth_stencil_state = null_mut();
      match unsafe {
        device.CreateDepthStencilState(&desc as *const _, &mut depth_stencil_state as *mut *mut _)
      } {
        0 | 1 => Ok(depth_stencil_state),
        e => Err(format!("CreateDepthStencilState error: {:?}", e)),
      }
    }?;

    let texture_view = {
      let mut fonts = ctx.fonts();
      let tex = fonts.build_rgba32_texture();

      let desc = D3D11_TEXTURE2D_DESC {
        Width: tex.width,
        Height: tex.height,
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
      };

      let mut d3dtex: *mut ID3D11Texture2D = null_mut();
      let sub_resource = D3D11_SUBRESOURCE_DATA {
        pSysMem: tex.data as *const _ as *const _,
        SysMemPitch: tex.width * 4,
        SysMemSlicePitch: 0,
      };

      unsafe {
        device.CreateTexture2D(
          &desc as *const _,
          &sub_resource as *const _,
          &mut d3dtex as *mut *mut _,
        );
      }

      let mut srv_desc_u: D3D11_SHADER_RESOURCE_VIEW_DESC_u = unsafe { std::mem::zeroed() };
      let mut srv_tex = unsafe { srv_desc_u.Texture2D_mut() };
      srv_tex.MipLevels = desc.MipLevels;
      srv_tex.MostDetailedMip = 0;
      let srv_desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        ViewDimension: D3D11_SRV_DIMENSION_TEXTURE2D,
        u: srv_desc_u,
      };

      let mut texture_view = null_mut();
      match unsafe {
        device.CreateShaderResourceView(
          std::mem::transmute(d3dtex),
          &srv_desc as *const _,
          &mut texture_view as *mut *mut _,
        )
      } {
        0 | 1 => {
          unsafe {
            (*d3dtex).Release();
          }
          fonts.tex_id = imgui::TextureId::from(texture_view);

          Ok(texture_view)
        }
        e => Err(Error(format!("CreateShaderResource error: {:x}", e))),
      }
    }?;

    let font_sampler = {
      let desc = D3D11_SAMPLER_DESC {
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
      };

      let mut font_sampler = null_mut();
      match unsafe {
        device.CreateSamplerState(&desc as *const _, &mut font_sampler as *mut *mut _)
      } {
        0 | 1 => Ok(font_sampler),
        e => Err(format!("CreateSamplerState error: {:x}", e)),
      }
    }?;

    Ok(DeviceObjects {
      vertex_shader,
      pixel_shader,
      blend_state,
      rasterizer_state,
      depth_stencil_state,
      font_sampler,
      texture_view,
    })
  }
}

impl Drop for DeviceObjects {
  #[inline]
  fn drop(&mut self) {
    unsafe {
      (*self.font_sampler).Release();
      (*self.texture_view).Release();
      (*self.blend_state).Release();
      (*self.depth_stencil_state).Release();
      (*self.rasterizer_state).Release();
    }
  }
}

//
// Render buffer data
//

struct RenderBufferData {
  vertex_buffer: *mut ID3D11Buffer,
  index_buffer: *mut ID3D11Buffer,
  vertex_buffer_size: usize,
  index_buffer_size: usize,
}

struct MappedSubresource<T, S>(
  Box<D3D11_MAPPED_SUBRESOURCE>,
  *mut S,
  NonNull<ID3D11DeviceContext>,
  PhantomData<T>,
);

impl<T, S> MappedSubresource<T, S> {
  fn map(ptr: *mut S, device_ctx: NonNull<ID3D11DeviceContext>) -> Result<MappedSubresource<T, S>> {
    let mut res: Box<D3D11_MAPPED_SUBRESOURCE> = Box::new(unsafe { std::mem::zeroed() });
    debug!("Mapping {:p} onto {:p}", ptr, res.as_ref());
    match unsafe {
      device_ctx
        .as_ref()
        .Map(ptr as _, 0, D3D11_MAP_WRITE_DISCARD, 0, res.as_mut()) //&mut res as *mut _)
    } {
      0 => Ok(()),
      i => Err(Error(format!("ID3D11DeviceContext::Map error: {}", i))),
    }?;

    Ok(MappedSubresource(res, ptr, device_ctx, PhantomData))
  }

  fn get_ptr(&self) -> *mut T {
    self.0.pData as *mut T
  }
}

impl<T, S> Drop for MappedSubresource<T, S> {
  fn drop(&mut self) {
    unsafe { self.2.as_ref().Unmap(self.1 as *mut _, 0) };
  }
}

impl RenderBufferData {
  fn new() -> RenderBufferData {
    RenderBufferData {
      vertex_buffer: null_mut(),
      index_buffer: null_mut(),
      vertex_buffer_size: 0,
      index_buffer_size: 0,
    }
  }

  fn check_sizes(
    &mut self,
    device: &mut ID3D11Device,
    vertex_buffer_size: usize,
    index_buffer_size: usize,
  ) -> Result<()> {
    // Mutate the buffers by allocating more memory if their size is not sufficient anymore
    if self.vertex_buffer_size < vertex_buffer_size
      || (self.vertex_buffer_size == 0 && vertex_buffer_size == 0)
    {
      unsafe {
        self
          .vertex_buffer
          .as_ref()
          .map(|e| e.Release())
          .unwrap_or(0)
      };
      self.vertex_buffer_size = vertex_buffer_size + 5000;
      self.vertex_buffer = RenderBufferData::create_vertex_buffer(device, self.vertex_buffer_size)?;
      debug!("Created vertex buffer {:p}", self.vertex_buffer);
    }

    if self.index_buffer_size < index_buffer_size
      || (self.index_buffer_size == 0 && index_buffer_size == 0)
    {
      unsafe { self.index_buffer.as_ref().map(|e| e.Release()).unwrap_or(0) };
      self.index_buffer_size = index_buffer_size + 5000;
      self.index_buffer = RenderBufferData::create_index_buffer(device, self.index_buffer_size)?;
      debug!("Created index buffer {:p}", self.index_buffer);
    }

    Ok(())
  }

  fn map_resources<'a>(
    &'a self,
    device_ctx: NonNull<ID3D11DeviceContext>,
  ) -> Result<(
    MappedSubresource<imgui::DrawVert, ID3D11Buffer>,
    MappedSubresource<imgui::DrawIdx, ID3D11Buffer>,
  )> {
    let msr_vert = MappedSubresource::map(self.vertex_buffer, device_ctx)?;
    let msr_idx = MappedSubresource::map(self.index_buffer, device_ctx)?;
    Ok((msr_vert, msr_idx))
  }

  fn create_vertex_buffer(device: &mut ID3D11Device, size: usize) -> Result<*mut ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
      Usage: D3D11_USAGE_DYNAMIC,
      ByteWidth: (size * std::mem::size_of::<imgui::DrawVert>()) as u32,
      BindFlags: D3D11_BIND_VERTEX_BUFFER,
      CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
      MiscFlags: 0,
      StructureByteStride: 0,
    };

    let mut vertex_buffer = null_mut();
    match unsafe {
      device.CreateBuffer(
        &desc as *const _,
        null_mut(),
        &mut vertex_buffer as *mut *mut _,
      )
    } {
      i if i != 0 => Err(format!("CreateBuffer error: {:x}", i).into()),
      _ => Ok(vertex_buffer),
    }
  }

  fn create_index_buffer(device: &mut ID3D11Device, size: usize) -> Result<*mut ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
      Usage: D3D11_USAGE_DYNAMIC,
      ByteWidth: (size * std::mem::size_of::<imgui::DrawIdx>()) as u32,
      BindFlags: D3D11_BIND_INDEX_BUFFER,
      CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
      MiscFlags: 0,
      StructureByteStride: 0,
    };

    let mut index_buffer = null_mut();
    match unsafe {
      device.CreateBuffer(
        &desc as *const _,
        null_mut(),
        &mut index_buffer as *mut *mut _,
      )
    } {
      i if i != 0 => Err(format!("CreateBuffer error: {:x}", i).into()),
      _ => Ok(index_buffer),
    }
  }
}

//
// Renderer
//

pub struct Renderer {
  device: NonNull<ID3D11Device>,
  device_ctx: NonNull<ID3D11DeviceContext>,
  //dxgi_factory: NonNull<IDXGIFactory>,
  device_objects: DeviceObjects,
  render_buffer_data: RenderBufferData,
}

impl Renderer {
  pub fn new(
    device: *mut ID3D11Device,
    device_ctx: *mut ID3D11DeviceContext,
    ctx: &mut imgui::Context,
  ) -> Result<Renderer> {
    ctx.set_renderer_name(imgui::ImString::from(String::from("imgui_impl_dx11_rs")));
    let io = ctx.io_mut();
    io.backend_flags |= imgui::BackendFlags::RENDERER_HAS_VTX_OFFSET;

    let mut device = NonNull::new(device).ok_or_else(|| (format!("Null device")))?;
    let device_ctx = NonNull::new(device_ctx).ok_or_else(|| (format!("Null device context")))?;
    let device_objects = DeviceObjects::new(unsafe { device.as_mut() }, ctx)?;

    // let dxgi_factory = {
    {
      let dxgi_device = {
        let mut dxgi_device: *mut IDXGIDevice = null_mut();
        match unsafe {
          device.as_ref().QueryInterface(
            &IDXGIDevice::uuidof(),
            &mut dxgi_device as *mut _ as *mut *mut _,
          )
        } {
          0 => Ok(NonNull::new(dxgi_device).unwrap()),
          e => Err(Error(format!("QueryInterface error: {:x}", e))),
        }
      }?;

      let dxgi_adapter = {
        let mut dxgi_adapter: *mut IDXGIAdapter = null_mut();
        match unsafe {
          dxgi_device.as_ref().GetParent(
            &IDXGIAdapter::uuidof(),
            &mut dxgi_adapter as *mut _ as *mut *mut _,
          )
        } {
          0 => Ok(NonNull::new(dxgi_adapter).unwrap()),
          e => Err(Error(format!("DXGI Device GetParent error: {:x}", e))),
        }
      }?;

      let dxgi_factory: NonNull<IDXGIFactory> = {
        let mut dxgi_factory: *mut IDXGIFactory = null_mut();
        match unsafe {
          dxgi_adapter.as_ref().GetParent(
            &IDXGIFactory::uuidof(),
            &mut dxgi_factory as *mut _ as *mut *mut _,
          )
        } {
          0 => Ok(NonNull::new(dxgi_factory).unwrap()),
          e => Err(Error(format!("DXGI Adapter GetParent error: {:x}", e))),
        }
      }?;

      unsafe {
        dxgi_device.as_ref().Release();
        dxgi_adapter.as_ref().Release();
      }

      unsafe { device.as_ref().AddRef() };
      unsafe { device_ctx.as_ref().AddRef() };

      Ok::<NonNull<IDXGIFactory>, Error>(dxgi_factory)
    }?;

    let render_buffer_data = RenderBufferData::new();

    Ok(Renderer {
      device,
      device_ctx,
      device_objects,
      /* dxgi_factory, */ render_buffer_data,
    })
  }

  pub fn render(&mut self, draw_data: &imgui::DrawData) -> Result<()> {
    debug!("Rendering draw data");
    if draw_data.display_size[0] <= 0. && draw_data.display_size[1] <= 0. {
      return Err(
        format!(
          "Insufficient display size {} x {}",
          draw_data.display_size[0], draw_data.display_size[1]
        )
        .into(),
      );
    }

    debug!("Checking sizes");
    self.render_buffer_data.check_sizes(
      unsafe { self.device.as_mut() },
      draw_data.total_vtx_count as _,
      draw_data.total_idx_count as _,
    )?;

    debug!("Mapping subresources for vertex and index buffers");
    let (vertex_pdata, index_pdata) = self.render_buffer_data.map_resources(self.device_ctx)?;

    debug!("Copying buffers");
    for (offset, cl) in draw_data.draw_lists().enumerate() {
      let vertex_buffer = cl.vtx_buffer();
      let index_buffer = cl.idx_buffer();
      unsafe {
        std::ptr::copy_nonoverlapping(
          vertex_buffer.as_ptr(),
          vertex_pdata.get_ptr().offset(offset as _),
          vertex_buffer.len(),
        );
        std::ptr::copy_nonoverlapping(
          index_buffer.as_ptr(),
          index_pdata.get_ptr().offset(offset as _),
          index_buffer.len(),
        );
      }
    }

    debug!("Dropping buffers");
    drop(vertex_pdata);
    drop(index_pdata);

    debug!("Mapping subresource");
    let context_pdata = MappedSubresource::<VERTEX_CONSTANT_BUFFER, ID3D11Buffer>::map(
      self.device_objects.vertex_shader.constant_buffer,
      self.device_ctx,
    )?;

    let cbpdata = context_pdata.get_ptr();
    let l = draw_data.display_pos[0];
    let r = draw_data.display_pos[0] + draw_data.display_size[0];
    let t = draw_data.display_pos[1];
    let b = draw_data.display_pos[1] + draw_data.display_size[1];
    let mvp = VERTEX_CONSTANT_BUFFER([
      [2. / (r - l), 0., 0., 0.],
      [0., 2. / (t - b), 0., 0.],
      [0., 0., 0.5, 0.],
      [(r + l) / (l - r), (t + b) / (b - t), 0.5, 1.0],
    ]);

    debug!("Copying context");
    unsafe {
      std::ptr::copy_nonoverlapping(&mvp.0 as *const _, &mut (*cbpdata).0 as *mut _, 1);
    }

    drop(context_pdata);

    debug!("Backing up state");
    let state_backup = StateBackup::backup(unsafe { self.device_ctx.as_ref() });

    let mut goffs_idx = 0;
    let mut goffs_vtx = 0;

    self.setup_render_state(draw_data);

    debug!("Drawing");
    for cl in draw_data.draw_lists() {
      for cmd in cl.commands() {
        match cmd {
          imgui::DrawCmd::Elements { count, cmd_params } => {
            let r = D3D11_RECT {
              left: (cmd_params.clip_rect[0] - draw_data.display_pos[0]) as i32,
              top: (cmd_params.clip_rect[1] - draw_data.display_pos[1]) as i32,
              right: (cmd_params.clip_rect[2] - draw_data.display_pos[0]) as i32,
              bottom: (cmd_params.clip_rect[3] - draw_data.display_pos[1]) as i32,
            };
            unsafe {
              self
                .device_ctx
                .as_ref()
                .RSSetScissorRects(1, &r as *const _)
            };

            let mut tex_srv = unsafe {
              std::mem::transmute::<_, *mut ID3D11ShaderResourceView>(cmd_params.texture_id)
            };
            unsafe {
              self
                .device_ctx
                .as_ref()
                .PSSetShaderResources(0, 1, &mut tex_srv as *mut _);
              self.device_ctx.as_ref().DrawIndexed(
                count as u32,
                (cmd_params.idx_offset + goffs_idx) as _,
                (cmd_params.vtx_offset + goffs_vtx) as _,
              );
            }
          }
          imgui::DrawCmd::ResetRenderState => {
            self.setup_render_state(draw_data);
          }
          imgui::DrawCmd::RawCallback { callback, raw_cmd } => unsafe {
            callback(cl.raw() as *const _, raw_cmd);
          },
        }
      }

      goffs_idx += cl.idx_buffer().len();
      goffs_vtx += cl.vtx_buffer().len();
    }

    debug!("Restoring backup");
    state_backup.restore(unsafe { self.device_ctx.as_ref() });

    Ok(())
  }

  fn setup_render_state(&self, draw_data: &imgui::DrawData) {
    let mut vp: D3D11_VIEWPORT = unsafe { std::mem::zeroed() };
    vp.Width = draw_data.display_size[0];
    vp.Height = draw_data.display_size[1];
    vp.MinDepth = 0.;
    vp.MaxDepth = 1.;
    vp.TopLeftX = 0.;
    vp.TopLeftY = 0.;

    unsafe {
      self
        .device_ctx
        .as_ref()
        .RSSetViewports(1, &mut vp as *mut _)
    };

    let stride = std::mem::size_of::<imgui::DrawVert>() as u32;
    let offs: u32 = 0;

    unsafe {
      self
        .device_ctx
        .as_ref()
        .IASetInputLayout(self.device_objects.vertex_shader.input_layout);
      self.device_ctx.as_ref().IASetVertexBuffers(
        0,
        1,
        &self.render_buffer_data.vertex_buffer,
        &stride,
        &offs,
      );
      self.device_ctx.as_ref().IASetIndexBuffer(
        self.render_buffer_data.index_buffer,
        if std::mem::size_of::<imgui::DrawIdx>() == 2 {
          DXGI_FORMAT_R16_UINT
        } else {
          DXGI_FORMAT_R32_UINT
        },
        0,
      );
      self
        .device_ctx
        .as_ref()
        .IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
      self.device_ctx.as_ref().VSSetShader(
        self.device_objects.vertex_shader.vertex_shader,
        null(),
        0,
      );
      self.device_ctx.as_ref().VSSetConstantBuffers(
        0,
        1,
        &self.device_objects.vertex_shader.constant_buffer,
      );
      self.device_ctx.as_ref().PSSetShader(
        self.device_objects.pixel_shader.pixel_shader,
        null(),
        0,
      );
      self
        .device_ctx
        .as_ref()
        .PSSetSamplers(0, 1, &self.device_objects.font_sampler);

      let blend_factor = [0f32, 0f32, 0f32, 0f32];
      self.device_ctx.as_ref().OMSetBlendState(
        self.device_objects.blend_state,
        &blend_factor,
        0xffffffff,
      );
      self
        .device_ctx
        .as_ref()
        .OMSetDepthStencilState(self.device_objects.depth_stencil_state, 0);
      self
        .device_ctx
        .as_ref()
        .RSSetState(self.device_objects.rasterizer_state);
    };
  }
}

impl Drop for Renderer {
  fn drop(&mut self) {
    unsafe { self.device.as_mut().Release() };
  }
}

