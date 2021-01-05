use std::ffi::CStr;
use std::ptr::null_mut;

use winapi::shared::dxgiformat::*;
use winapi::um::d3d11::*;
use winapi::um::d3dcommon::*;
use winapi::um::d3dcompiler::*;

use crate::util::*;

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
    output.pos = mul( ProjectionMatrix, float4(input.pos.xy, 0.f, 1.f));
    output.col = input.col;
    output.uv  = input.uv;
    return output;
  }";

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

//
// Vertex shader implementation
//

pub struct VertexShader {
  pub vertex_shader: *mut ID3D11VertexShader,
  vertex_shader_blob: *mut ID3D10Blob,
  pub constant_buffer: *mut ID3D11Buffer,
  pub input_layout: *mut ID3D11InputLayout,
}

impl VertexShader {
  pub fn new(device: &mut ID3D11Device) -> Result<VertexShader> {
    let mut vertex_shader_blob: *mut ID3D10Blob = null_mut();
    let mut vertex_shader = null_mut();
    let mut input_layout = null_mut();
    let mut constant_buffer: *mut ID3D11Buffer = null_mut();

    match unsafe {
      D3DCompile(
        reckless_string(VERTEX_SHADER_SRC).as_ptr() as _,
        VERTEX_SHADER_SRC.len(),
        null_mut(),
        null_mut(),
        null_mut(),
        reckless_string("main").as_ptr() as _,
        reckless_string("vs_4_0").as_ptr(),
        0,
        0,
        &mut vertex_shader_blob as *mut _,
        null_mut(),
      )
    } {
      0 | 1 => { /* OK */ }
      e => return Err(format!("D3DCompile Vertex Shader: {:x}", e).into()),
    }

    let vertex_shader_blob_ref = ptr_as_ref(vertex_shader_blob)?;

    if unsafe {
      device.CreateVertexShader(
        vertex_shader_blob_ref.GetBufferPointer(),
        vertex_shader_blob_ref.GetBufferSize(),
        null_mut(),
        &mut vertex_shader as *mut _,
      )
    } != 0
    {
      return Err("CreateVertexShader error".to_string().into());
    }

    let local_layout = [
      D3D11_INPUT_ELEMENT_DESC {
        SemanticName: CStr::from_bytes_with_nul(b"POSITION\0").unwrap().as_ptr(),
        SemanticIndex: 0,
        Format: DXGI_FORMAT_R32G32_FLOAT,
        InputSlot: 0,
        AlignedByteOffset: 0,
        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
        InstanceDataStepRate: 0,
      },
      D3D11_INPUT_ELEMENT_DESC {
        SemanticName: CStr::from_bytes_with_nul(b"TEXCOORD\0").unwrap().as_ptr(),
        SemanticIndex: 0,
        Format: DXGI_FORMAT_R32G32_FLOAT,
        InputSlot: 0,
        AlignedByteOffset: 8,
        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
        InstanceDataStepRate: 0,
      },
      D3D11_INPUT_ELEMENT_DESC {
        SemanticName: CStr::from_bytes_with_nul(b"COLOR\0").unwrap().as_ptr(),
        SemanticIndex: 0,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        InputSlot: 0,
        AlignedByteOffset: 16,
        InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
        InstanceDataStepRate: 0,
      },
    ];

    match unsafe {
      device.CreateInputLayout(
        local_layout.as_ptr(),
        3,
        vertex_shader_blob_ref.GetBufferPointer(),
        vertex_shader_blob_ref.GetBufferSize(),
        &mut input_layout as *mut _,
      )
    } {
      0 => {}
      e => return Err(format!("CreateInputLayout error: {:x}", e).into()),
    };

    let desc = D3D11_BUFFER_DESC {
      ByteWidth: std::mem::size_of::<VERTEX_CONSTANT_BUFFER>() as u32,
      Usage: D3D11_USAGE_DYNAMIC,
      BindFlags: D3D11_BIND_CONSTANT_BUFFER,
      CPUAccessFlags: D3D11_CPU_ACCESS_WRITE,
      MiscFlags: 0,
      StructureByteStride: 0,
    };

    unsafe {
      device.CreateBuffer(
        &desc as *const _,
        null_mut(),
        &mut constant_buffer as *mut _,
      );
    }

    Ok(VertexShader {
      vertex_shader,
      vertex_shader_blob,
      constant_buffer,
      input_layout,
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
  pub pixel_shader: *mut ID3D11PixelShader,
  pixel_shader_blob: *mut ID3D10Blob,
}

impl PixelShader {
  pub fn new(device: &mut ID3D11Device) -> Result<PixelShader> {
    let mut pixel_shader_blob: *mut ID3D10Blob = null_mut();
    let mut pixel_shader = null_mut();

    match unsafe {
      D3DCompile(
        reckless_string(PIXEL_SHADER_SRC).as_ptr() as _,
        PIXEL_SHADER_SRC.len(),
        null_mut(),
        null_mut(),
        null_mut(),
        reckless_string("main").as_ptr(),
        reckless_string("ps_4_0").as_ptr(),
        0,
        0,
        &mut pixel_shader_blob as *mut _,
        null_mut(),
      )
    } {
      0 | 1 => { /* OK */ }
      e => return Err(format!("D3DCompile Pixel Shader: {:x}", e).into()),
    }

    let pixel_shader_blob_ref = ptr_as_ref(pixel_shader_blob)?;

    if unsafe {
      device.CreatePixelShader(
        pixel_shader_blob_ref.GetBufferPointer(),
        pixel_shader_blob_ref.GetBufferSize(),
        null_mut(),
        &mut pixel_shader as *mut _,
      )
    } != 0
    {
      return Err("CreatePixelShader error".to_string().into());
    }

    Ok(PixelShader {
      pixel_shader,
      pixel_shader_blob,
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
