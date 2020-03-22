use imgui;

use std::ptr::{null, null_mut, NonNull};
use std::ffi::{CString, CStr};

use winapi::um::d3d11::*;
use winapi::um::d3dcompiler::*;
use winapi::um::d3dcommon::*;
use winapi::shared::dxgi::*;
use winapi::shared::dxgiformat::*;
use winapi::shared::dxgitype::*;

use winapi::Interface;
use imgui::internal::RawWrapper;

const VERTEX_SHADER_SRC: &'static str = r"
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
    output.pos = mul( ProjectionMatrix, float4(input.pos.xy, 0.f, 1.f));
    output.col = input.col;
    output.uv  = input.uv;
    return output;
  }";

const PIXEL_SHADER_SRC: &'static str = r"
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

pub struct Error(String);

impl From<String> for Error {
  fn from(s: String) -> Error {
    Error(s)
  }
}

type Result<T> = std::result::Result<T, Error>;

//
// Vertex shader implementation
//

pub struct VertexShader {
  vertex_shader: *mut ID3D11VertexShader,
  vertex_shader_blob: *mut ID3D10Blob,
  constant_buffer: *mut ID3D11Buffer,
  input_layout: *mut ID3D11InputLayout,
}
#[repr(C)]
struct VERTEX_CONSTANT_BUFFER {
  mvp: [[f32; 4]; 4]
}

impl VertexShader {
  fn new(device: &mut ID3D11Device) -> Result<VertexShader> {
    let mut vertex_shader_blob: *mut ID3D10Blob = null_mut();
    let mut vertex_shader = null_mut();
    let mut input_layout = null_mut();
    let mut constant_buffer: *mut ID3D11Buffer = null_mut();
    
    match unsafe {
      D3DCompile(
        reckless_string(VERTEX_SHADER_SRC) as _,
        VERTEX_SHADER_SRC.len(),
        null_mut(), null_mut(), null_mut(),
        reckless_string("main") as _,
        reckless_string("vs_4_0"),
        0, 0,
        &mut vertex_shader_blob as *mut _,
        null_mut()
      )
    } {
      0 | 1 => { /* OK */ }
      e => return Err(format!("D3DCompile: {:x}", e).into())
    }

    let vertex_shader_blob_ref = ptr_as_ref(vertex_shader_blob)?;

    if unsafe {
      device.CreateVertexShader(
        vertex_shader_blob_ref.GetBufferPointer(),
        vertex_shader_blob_ref.GetBufferSize(),
        null_mut(),
        &mut vertex_shader as *mut _
      )
    } != 0 {
      return Err(format!("CreateVertexShader error").into())
    }

    let local_layout = [
      D3D11_INPUT_ELEMENT_DESC {
        SemanticName: CStr::from_bytes_with_nul(b"POSITION\0").unwrap().as_ptr(),
        SemanticIndex: 0,
        Format: DXGI_FORMAT_R32G32_FLOAT,
        InputSlot: 0,
        AlignedByteOffset: 0,
        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
        InstanceDataStepRate: 0
      },
      D3D11_INPUT_ELEMENT_DESC {
        SemanticName: CStr::from_bytes_with_nul(b"TEXCOORD\0").unwrap().as_ptr(),
        SemanticIndex: 0,
        Format: DXGI_FORMAT_R32G32_FLOAT,
        InputSlot: 0,
        AlignedByteOffset: 8,
        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
        InstanceDataStepRate: 0
      },
      D3D11_INPUT_ELEMENT_DESC {
        SemanticName: CStr::from_bytes_with_nul(b"COLOR\0").unwrap().as_ptr(),
        SemanticIndex: 0,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        InputSlot: 0,
        AlignedByteOffset: 16,
        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
        InstanceDataStepRate: 0
      }
    ];

    match unsafe {
      device.CreateInputLayout(
        local_layout.as_ptr(), 3,
        vertex_shader_blob_ref.GetBufferPointer(),
        vertex_shader_blob_ref.GetBufferSize(),
        &mut input_layout as *mut _
      )
    } {
      0 => {},
      e => return Err(format!("CreateInputLayout error: {:x}", e).into())
    };

    let desc = D3D11_BUFFER_DESC {
      ByteWidth: std::mem::size_of::<VERTEX_CONSTANT_BUFFER>() as u32,
      Usage: D3D11_USAGE_DYNAMIC,
      BindFlags: D3D11_BIND_CONSTANT_BUFFER,
      CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
      MiscFlags: 0,
      StructureByteStride: 0
    };

    unsafe {
      device.CreateBuffer(&desc as *const _, null_mut(), &mut constant_buffer as *mut _);
    }

    Ok(VertexShader {
      vertex_shader,
      vertex_shader_blob,
      constant_buffer,
      input_layout
    })
  }
}

impl Drop for VertexShader {
  fn drop(&mut self) {
    unsafe {
      (*self.constant_buffer).Release();
      (*self.input_layout).Release();
      (*self.vertex_shader).Release();
      (*self.vertex_shader_blob).Release();
    }
  }
}

//
// Pixel shader implementation
//

pub struct PixelShader {
  pixel_shader: *mut ID3D11PixelShader,
  pixel_shader_blob: *mut ID3D10Blob,
}

impl PixelShader {
  fn new(device: &mut ID3D11Device) -> Result<PixelShader> {
    let mut pixel_shader_blob: *mut ID3D10Blob = null_mut();
    let mut pixel_shader = null_mut();

    match unsafe {
      D3DCompile(
        reckless_string(PIXEL_SHADER_SRC) as _,
        PIXEL_SHADER_SRC.len(),
        null_mut(), null_mut(), null_mut(),
        reckless_string("main"),
        reckless_string("ps_4_0"),
        0, 0,
        &mut pixel_shader_blob as *mut _,
        null_mut()
      )
    } {
      0 | 1 => { /* OK */ },
      e => return Err(format!("D3DCompile Pixel Shader: {:x}", e).into())
    }

    let pixel_shader_blob_ref = ptr_as_ref(pixel_shader_blob)?;

    if unsafe {
      device.CreatePixelShader(
        pixel_shader_blob_ref.GetBufferPointer(),
        pixel_shader_blob_ref.GetBufferSize(),
        null_mut(),
        &mut pixel_shader as *mut _
      )
    } != 0 {
      return Err(format!("CreatePixelShader error").into());
    }

    Ok(PixelShader {
      pixel_shader, pixel_shader_blob
    })
  }
}

impl Drop for PixelShader {
  fn drop(&mut self) {
    unsafe {
      (*self.pixel_shader).Release();
      (*self.pixel_shader_blob).Release();
    }
  }
}

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
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL as u8
          },
          unsafe { std::mem::zeroed() },
          unsafe { std::mem::zeroed() },
          unsafe { std::mem::zeroed() },
          unsafe { std::mem::zeroed() },
          unsafe { std::mem::zeroed() },
          unsafe { std::mem::zeroed() },
          unsafe { std::mem::zeroed() }
        ]
      };

      let mut blend_state = null_mut();
      match unsafe {
        device.CreateBlendState(&desc as *const _, &mut blend_state as *mut *mut _)
      } {
        0 | 1 => Ok(blend_state),
        e => Err(format!("CreateBlendState error: {:x}", e))
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
        FrontCounterClockwise: 0
      };

      let mut rasterizer_state = null_mut();
      match unsafe {
        device.CreateRasterizerState(&desc as *const _, &mut rasterizer_state as *mut *mut _)
      } {
        0 | 1 => Ok(rasterizer_state),
        e => Err(format!("CreateRasterizerState error: {:?}", e))
      }
    }?;

    let depth_stencil_state = {
      let ff = D3D11_DEPTH_STENCILOP_DESC {
        StencilFailOp: D3D11_STENCIL_OP_KEEP,
        StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
        StencilPassOp: D3D11_STENCIL_OP_KEEP,
        StencilFunc: D3D11_COMPARISON_ALWAYS
      };
      let desc = D3D11_DEPTH_STENCIL_DESC {
        DepthEnable: 0,
        DepthWriteMask: D3D11_DEPTH_WRITE_MASK_ALL,
        DepthFunc: D3D11_COMPARISON_ALWAYS,
        StencilEnable: 0,
        StencilReadMask: 0,
        StencilWriteMask: 0,
        FrontFace: ff,
        BackFace: ff.clone()
      };

      let mut depth_stencil_state = null_mut();
      match unsafe {
        device.CreateDepthStencilState(&desc as *const _, &mut depth_stencil_state as *mut *mut _)
      } {
        0 | 1 => Ok(depth_stencil_state),
        e => Err(format!("CreateDepthStencilState error: {:?}", e))
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
          Quality: 0
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE,
        CPUAccessFlags: 0,
        MiscFlags: 0
      };

      let mut d3dtex: *mut ID3D11Texture2D = null_mut();
      let sub_resource = D3D11_SUBRESOURCE_DATA {
        pSysMem: tex.data as *const _ as *const _,
        SysMemPitch: tex.width * 4,
        SysMemSlicePitch: 0
      };
      
      unsafe {
        device.CreateTexture2D(&desc as *const _, &sub_resource as *const _, &mut d3dtex as *mut *mut _);
      }

      let mut srv_desc_u: D3D11_SHADER_RESOURCE_VIEW_DESC_u = unsafe { std::mem::zeroed() };
      let mut srv_tex = unsafe { srv_desc_u.Texture2D_mut() };
      srv_tex.MipLevels = desc.MipLevels;
      srv_tex.MostDetailedMip = 0;
      let srv_desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        ViewDimension: D3D11_SRV_DIMENSION_TEXTURE2D,
        u: srv_desc_u
      };

      let mut texture_view = null_mut();
      match unsafe { 
        device.CreateShaderResourceView(std::mem::transmute(d3dtex), &srv_desc as *const _, &mut texture_view as *mut *mut _)
      } {
        0 | 1 => {
          unsafe { (*d3dtex).Release(); }
          fonts.tex_id = imgui::TextureId::from(texture_view);
          
          Ok(texture_view)
        },
        e => Err(Error(format!("CreateShaderResource error: {:x}", e)))
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
        MaxAnisotropy: 0
      };

      let mut font_sampler = null_mut();
      match unsafe {
        device.CreateSamplerState(&desc as *const _, &mut font_sampler as *mut *mut _)
      } {
        0 | 1 => Ok(font_sampler),
        e => Err(format!("CreateSamplerState error: {:x}", e))
      }
    }?;

    Ok(DeviceObjects {
      vertex_shader, pixel_shader,
      blend_state, rasterizer_state, depth_stencil_state,
      font_sampler, texture_view
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

struct MappedSubresource<'a, T>(*mut T, &'a ID3D11DeviceContext);

impl<'a, T> MappedSubresource<'a, T> {
  fn map<Source>(
    ptr: *mut Source, device_ctx: &ID3D11DeviceContext
  ) -> Result<MappedSubresource<T>> {
    let mut res: D3D11_MAPPED_SUBRESOURCE = unsafe { std::mem::zeroed() };
    match unsafe {
      device_ctx.Map(
        std::mem::transmute(ptr),
        0, D3D11_MAP_WRITE_DISCARD,
        0, &mut res as *mut _
      )
    } {
      0 => Ok(()),
      i => Err(Error(format!("ID3D11DeviceContext::Map error: {}", i)))
    }?;

    let mut output: *mut T = unsafe { std::mem::transmute(res.pData) };

    Ok(MappedSubresource(output, device_ctx))
  }
}

impl<'a, T> Drop for MappedSubresource<'a, T> {
  fn drop(&mut self) {
    unsafe { self.1.Unmap(std::mem::transmute(self.0), 0) };
  }
}

impl RenderBufferData {

  fn new() -> RenderBufferData {
    RenderBufferData {
      vertex_buffer: null_mut(),
      index_buffer: null_mut(),
      vertex_buffer_size: 0,
      index_buffer_size: 0
    }
  }

  fn check_sizes(
    &mut self, device: &mut ID3D11Device,
    vertex_buffer_size: usize, index_buffer_size: usize) -> Result<()>
  {
    // Mutate the buffers by allocating more memory if their size is not sufficient anymore
    if self.vertex_buffer_size < vertex_buffer_size {
      unsafe { self.vertex_buffer.as_ref().map(|e| e.Release()) };
      self.vertex_buffer_size = vertex_buffer_size + 5000;
      self.vertex_buffer = RenderBufferData::create_vertex_buffer(device, self.vertex_buffer_size)?;
    }

    if self.index_buffer_size < index_buffer_size {
      unsafe { self.index_buffer.as_ref().map(|e| e.Release()) };
      self.index_buffer_size = index_buffer_size + 5000;
      self.index_buffer = RenderBufferData::create_index_buffer(device, self.index_buffer_size)?;
    }

    Ok(())
  }

  fn map_resources<'a>(
    &'a self, device_ctx: &'a ID3D11DeviceContext
  ) -> Result<(MappedSubresource<imgui::DrawVert>, MappedSubresource<imgui::DrawIdx>)> {
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
      StructureByteStride: 0
    };

    let mut vertex_buffer = null_mut();
    match unsafe {
      device.CreateBuffer(&desc as *const _, null_mut(), &mut vertex_buffer as *mut *mut _)
    } {
      i if i < 0 => Err(format!("CreateBuffer error: {}", i).into()),
      _ => Ok(vertex_buffer)
    }
  }

  fn create_index_buffer(device: &mut ID3D11Device, size: usize) -> Result<*mut ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
      Usage: D3D11_USAGE_DYNAMIC,
      ByteWidth: (size * std::mem::size_of::<imgui::DrawIdx>()) as u32,
      BindFlags: D3D11_BIND_INDEX_BUFFER,
      CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
      MiscFlags: 0,
      StructureByteStride: 0
    };

    let mut index_buffer = null_mut();
    match unsafe {
      device.CreateBuffer(&desc as *const _, null_mut(), &mut index_buffer as *mut *mut _)
    } {
      i if i < 0 => Err(format!("CreateBuffer error: {}", i).into()),
      _ => Ok(index_buffer)
    }
  }
}

//
// Renderer
//

pub struct Renderer {
  device: NonNull<ID3D11Device>,
  device_ctx: NonNull<ID3D11DeviceContext>,
  dxgi_factory: NonNull<IDXGIFactory>,
  device_objects: DeviceObjects,
  render_buffer_data: RenderBufferData
}

impl Renderer {
  fn new(
    device: *mut ID3D11Device,
    device_ctx: *mut ID3D11DeviceContext,
    ctx: &mut imgui::Context
  ) -> Result<Renderer> {
    ctx.set_renderer_name(imgui::ImString::from(String::from("imgui_impl_dx11_rs")));
    let io = ctx.io_mut();
    io.backend_flags |= imgui::BackendFlags::RENDERER_HAS_VTX_OFFSET;

    let mut device = NonNull::new(device).ok_or_else(|| (format!("Null device")))?;
    let mut device_ctx = NonNull::new(device_ctx).ok_or_else(|| (format!("Null device context")))?;
    let device_objects = DeviceObjects::new(unsafe { device.as_mut() }, ctx)?;

    let dxgi_factory = {
      let dxgi_device = {
        let mut dxgi_device: *mut IDXGIDevice = null_mut();
        match unsafe { device.as_ref().QueryInterface(&IDXGIDevice::uuidof(), &mut dxgi_device as *mut _ as *mut *mut _) } {
          0 => Ok(NonNull::new(dxgi_device).unwrap()),
          e => Err(Error(format!("QueryInterface error: {:x}", e)))
        }
      }?;

      let dxgi_adapter = {
        let mut dxgi_adapter: *mut IDXGIAdapter = null_mut();
        match unsafe { dxgi_device.as_ref().GetParent(&IDXGIAdapter::uuidof(), &mut dxgi_adapter as *mut _ as *mut *mut _) } {
          0 => Ok(NonNull::new(dxgi_adapter).unwrap()),
          e => Err(Error(format!("DXGI Device GetParent error: {:x}", e)))
        }
      }?;

      let dxgi_factory: NonNull<IDXGIFactory> = {
        let mut dxgi_factory: *mut IDXGIFactory = null_mut();
        match unsafe { dxgi_adapter.as_ref().GetParent(&IDXGIAdapter::uuidof(), &mut dxgi_factory as *mut _ as *mut *mut _) } {
          0 => Ok(NonNull::new(dxgi_factory).unwrap()),
          e => Err(Error(format!("DXGI Adapter GetParent error: {:x}", e)))
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

    Ok(Renderer { device, device_ctx, device_objects, dxgi_factory, render_buffer_data })
  }

  fn render(&mut self, draw_data: &imgui::DrawData) -> Result<()> {
    if draw_data.display_size[0] <= 0. && draw_data.display_size[1] <= 0. {
      return Err(
        format!(
          "Insufficient display size {} x {}",
          draw_data.display_size[0], draw_data.display_size[1]
        ).into()
      );
    }

    self.render_buffer_data.check_sizes(
      unsafe { self.device.as_mut() },
      draw_data.total_vtx_count as _,
      draw_data.total_idx_count as _
    )?;

    let (vertex_pdata, index_pdata) =
      self.render_buffer_data.map_resources(unsafe {
        self.device_ctx.as_ref()
      })?;

    for (offset, cl) in draw_data.draw_lists().enumerate() {
      let vertex_buffer = cl.vtx_buffer();
      let index_buffer = cl.idx_buffer();
      unsafe {
        std::ptr::copy_nonoverlapping(
          vertex_buffer.as_ptr(),
          vertex_pdata.0.offset(offset as _),
          vertex_buffer.len()
        );
        std::ptr::copy_nonoverlapping(
          index_buffer.as_ptr(),
          index_pdata.0.offset(offset as _),
          index_buffer.len()
        );
      }
    }

    drop(vertex_pdata);
    drop(index_pdata);

    let context_pdata = MappedSubresource(
      self.device_objects.vertex_shader.constant_buffer,
      unsafe { self.device_ctx.as_ref() } 
    );

    let cbpdata: *mut VERTEX_CONSTANT_BUFFER = unsafe {
      std::mem::transmute(context_pdata.0)
    };
    let l = draw_data.display_pos[0];
    let r = draw_data.display_pos[0] + draw_data.display_size[0];
    let t = draw_data.display_pos[1];
    let b = draw_data.display_pos[1] + draw_data.display_size[1];
    let mvp = VERTEX_CONSTANT_BUFFER {
      mvp: [
        [ 2. / (r - l), 0., 0., 0. ],
        [ 0., 2. / (t - b), 0., 0. ],
        [ 0., 0., 0.5, 0. ],
        [ (r + l) / (l - r), (t + b) / (b - t), 0.5, 1.0 ]
      ]
    };

    unsafe {
      std::ptr::copy_nonoverlapping(
        &mvp.mvp as *const _,
        &mut (*cbpdata).mvp as *mut _,
        1
      );
    }

    drop(context_pdata);

    Ok(())
  }
}

impl Drop for Renderer {
  fn drop(&mut self) {
    unsafe { self.device.as_mut().Release() };
  }
}

//
// A reckless implementation of a conversion from 
// a string to raw C char data. Pls only use with
// static const strings.
//

unsafe fn reckless_string(s: &str) -> *const i8 {
  CString::new(s).unwrap().as_ptr()
}

//
// Convert pointer to ref, emit error if null
//

fn ptr_as_ref<'a, T>(ptr: *const T) -> Result<&'a T> {
  match unsafe { ptr.as_ref() } {
    Some(t) => Ok(t),
    None => Err(format!("Null pointer").into())
  }
}