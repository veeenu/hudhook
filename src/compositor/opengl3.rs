use std::cell::RefCell;
use std::ffi::CString;
use std::os::raw::c_void;
use std::{mem, ptr};

use gl::types::{GLchar, GLint, GLuint};
use once_cell::sync::OnceCell;
use windows::core::{s, Result, PCSTR};
use windows::Win32::Foundation::{FARPROC, HINSTANCE};
use windows::Win32::Graphics::Direct3D12::ID3D12Resource;
use windows::Win32::Graphics::OpenGL::wglGetProcAddress;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

use crate::renderer::RenderEngine;

mod gl {
    #![cfg_attr(
        feature = "cargo-clippy",
        allow(
            clippy::unreadable_literal,
            clippy::too_many_arguments,
            clippy::unused_unit,
            clippy::upper_case_acronyms,
            clippy::manual_non_exhaustive
        )
    )]

    include!(concat!(env!("OUT_DIR"), "/gl_bindings.rs"));
}

unsafe fn load_func(function_string: CString) -> *const c_void {
    static OPENGL3_LIB: OnceCell<HINSTANCE> = OnceCell::new();
    let module = OPENGL3_LIB
        .get_or_init(|| LoadLibraryA(s!("opengl32.dll\0")).expect("LoadLibraryA").into());

    if let Some(wgl_proc_address) = wglGetProcAddress(PCSTR(function_string.as_ptr() as _)) {
        wgl_proc_address as _
    } else {
        let proc_address: FARPROC = GetProcAddress(*module, PCSTR(function_string.as_ptr() as _));
        proc_address.unwrap() as _
    }
}

pub struct Compositor {
    gl: gl::Gl,
    program: GLuint,
    texture_loc: GLuint,
    ebo: GLuint,
    texture: GLuint,
    texture_data: RefCell<Vec<u8>>,
}

impl Compositor {
    pub fn new() -> Result<Self> {
        const VS: &[u8] = b"
          #version 130

          in vec2 Position;
          in vec2 UV;
          out vec2 Frag_UV;
          void main()
          {
              const vec2 positions[4] = vec2[](
                  vec2(-1, 1),
                  vec2(+1, 1),
                  vec2(-1, -1),
                  vec2(+1, -1)
              );
              const vec2 coords[4] = vec2[](
                  vec2(0, 0),
                  vec2(1, 0),
                  vec2(0, 1),
                  vec2(1, 1)
              );
              Frag_UV = coords[gl_VertexID];
              gl_Position = vec4(positions[gl_VertexID], 0.0, 1.0);
          }
        \0";

        const FS: &[u8] = b"
          #version 130

          uniform sampler2D Texture;
          in vec2 Frag_UV;
          out vec4 Out_Color;
          void main()
          {
            Out_Color = texture(Texture, Frag_UV.st);
          }
        \0";

        const ELEMENTS: [u16; 6] = [0, 1, 2, 1, 3, 2];

        tracing::trace!("Loading");
        let gl = gl::Gl::load_with(|s| unsafe { load_func(CString::new(s).unwrap()) });
        tracing::trace!("Loaded");

        unsafe {
            let program = gl.CreateProgram();
            let vertex_shader = gl.CreateShader(gl::VERTEX_SHADER);
            let fragment_shader = gl.CreateShader(gl::FRAGMENT_SHADER);
            let vertex_source = [VS.as_ptr() as *const GLchar];
            let fragment_source = [FS.as_ptr() as *const GLchar];
            let vertex_source_len = [VS.len() as i32];
            let fragment_source_len = [FS.len() as i32];
            gl.ShaderSource(vertex_shader, 1, vertex_source.as_ptr(), vertex_source_len.as_ptr());
            gl.ShaderSource(
                fragment_shader,
                1,
                fragment_source.as_ptr(),
                fragment_source_len.as_ptr(),
            );
            gl.CompileShader(vertex_shader);
            gl.CompileShader(fragment_shader);
            gl.AttachShader(program, vertex_shader);
            gl.AttachShader(program, fragment_shader);
            gl.LinkProgram(program);
            gl.DeleteShader(vertex_shader);
            gl.DeleteShader(fragment_shader);

            let texture_loc = gl.GetUniformLocation(program, b"Texture\0".as_ptr() as _) as GLuint;

            let ebo = out_param(|x| gl.GenBuffers(1, x));
            let texture = out_param(|x| gl.GenTextures(1, x));

            gl.BindBuffer(gl::ELEMENT_ARRAY_BUFFER, ebo);
            gl.BufferData(
                gl::ELEMENT_ARRAY_BUFFER,
                mem::size_of_val(&ELEMENTS) as _,
                ELEMENTS.as_ptr() as _,
                gl::STREAM_DRAW,
            );

            let mut bound_texture = 0;
            gl.GetIntegerv(gl::TEXTURE_BINDING_2D, &mut bound_texture);

            gl.ActiveTexture(gl::TEXTURE0);
            gl.BindTexture(gl::TEXTURE_2D, texture);
            gl.TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as _);
            gl.TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as _);
            gl.BindTexture(gl::TEXTURE_2D, bound_texture as _);

            Ok(Compositor {
                gl,
                program,
                ebo,
                texture,
                texture_loc,
                texture_data: RefCell::new(Vec::new()),
            })
        }
    }

    pub fn composite(&self, engine: &RenderEngine, source: ID3D12Resource) -> Result<()> {
        let gl = &self.gl;

        unsafe {
            let desc = source.GetDesc();

            let backup = backup(gl);

            gl.Enable(gl::BLEND);
            gl.BlendEquation(gl::FUNC_ADD);
            gl.BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            gl.Disable(gl::CULL_FACE);
            gl.Disable(gl::DEPTH_TEST);
            gl.Enable(gl::SCISSOR_TEST);
            gl.PolygonMode(gl::FRONT_AND_BACK, gl::FILL);

            gl.Viewport(0, 0, desc.Width as _, desc.Height as _);
            gl.UseProgram(self.program);
            if gl.BindSampler.is_loaded() {
                gl.BindSampler(0, 0);
            }
            gl.BindBuffer(gl::ELEMENT_ARRAY_BUFFER, self.ebo);

            gl.ActiveTexture(gl::TEXTURE0);
            gl.TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as _);
            gl.TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as _);
            gl.BindTexture(gl::TEXTURE_2D, self.texture);
            gl.Uniform1i(self.texture_loc as _, 0);

            let mut texture_data = self.texture_data.borrow_mut();
            texture_data.resize(desc.Width as usize * desc.Height as usize * 4, 0);
            engine.copy_texture(source, texture_data.as_mut_ptr())?;

            gl.TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA as _,
                desc.Width as _,
                desc.Height as _,
                0,
                gl::BGRA,
                gl::UNSIGNED_BYTE,
                texture_data.as_ptr() as _,
            );

            gl.Scissor(0, 0, desc.Width as _, desc.Height as _);
            gl.DrawElements(gl::TRIANGLES, 6, gl::UNSIGNED_SHORT, ptr::null());

            restore(gl, backup);
        }

        Ok(())
    }
}

struct Backup {
    last_active_texture: i32,
    last_program: i32,
    last_texture: i32,
    last_sampler: i32,
    last_array_buffer: i32,
    last_element_array_buffer: i32,
    last_vertex_array: i32,
    last_polygon_mode: [i32; 2],
    last_viewport: [i32; 4],
    last_scissor_box: [i32; 4],
    last_blend_src_rgb: i32,
    last_blend_dst_rgb: i32,
    last_blend_src_alpha: i32,
    last_blend_dst_alpha: i32,
    last_blend_equation_rgb: i32,
    last_blend_equation_alpha: i32,
    last_enable_blend: bool,
    last_enable_cull_face: bool,
    last_enable_depth_test: bool,
    last_enable_scissor_test: bool,
}

unsafe fn backup(gl: &gl::Gl) -> Backup {
    let last_active_texture = out_param(|x| gl.GetIntegerv(gl::ACTIVE_TEXTURE, x));
    gl.ActiveTexture(gl::TEXTURE0);
    let last_program = out_param(|x| gl.GetIntegerv(gl::CURRENT_PROGRAM, x));
    let last_texture = out_param(|x| gl.GetIntegerv(gl::TEXTURE_BINDING_2D, x));
    let last_sampler = if gl.BindSampler.is_loaded() {
        out_param(|x| gl.GetIntegerv(gl::SAMPLER_BINDING, x))
    } else {
        0
    };
    let last_array_buffer = out_param(|x| gl.GetIntegerv(gl::ARRAY_BUFFER_BINDING, x));
    let last_element_array_buffer =
        out_param(|x| gl.GetIntegerv(gl::ELEMENT_ARRAY_BUFFER_BINDING, x));
    let last_vertex_array = out_param(|x| gl.GetIntegerv(gl::VERTEX_ARRAY_BINDING, x));
    let last_polygon_mode =
        out_param(|x: &mut [GLint; 2]| gl.GetIntegerv(gl::POLYGON_MODE, x.as_mut_ptr()));
    let last_viewport =
        out_param(|x: &mut [GLint; 4]| gl.GetIntegerv(gl::VIEWPORT, x.as_mut_ptr()));
    let last_scissor_box =
        out_param(|x: &mut [GLint; 4]| gl.GetIntegerv(gl::SCISSOR_BOX, x.as_mut_ptr()));
    let last_blend_src_rgb = out_param(|x| gl.GetIntegerv(gl::BLEND_SRC_RGB, x));
    let last_blend_dst_rgb = out_param(|x| gl.GetIntegerv(gl::BLEND_DST_RGB, x));
    let last_blend_src_alpha = out_param(|x| gl.GetIntegerv(gl::BLEND_SRC_ALPHA, x));
    let last_blend_dst_alpha = out_param(|x| gl.GetIntegerv(gl::BLEND_DST_ALPHA, x));
    let last_blend_equation_rgb = out_param(|x| gl.GetIntegerv(gl::BLEND_EQUATION_RGB, x));
    let last_blend_equation_alpha = out_param(|x| gl.GetIntegerv(gl::BLEND_EQUATION_ALPHA, x));
    let last_enable_blend = gl.IsEnabled(gl::BLEND) == gl::TRUE;
    let last_enable_cull_face = gl.IsEnabled(gl::CULL_FACE) == gl::TRUE;
    let last_enable_depth_test = gl.IsEnabled(gl::DEPTH_TEST) == gl::TRUE;
    let last_enable_scissor_test = gl.IsEnabled(gl::SCISSOR_TEST) == gl::TRUE;

    Backup {
        last_active_texture,
        last_program,
        last_texture,
        last_sampler,
        last_array_buffer,
        last_element_array_buffer,
        last_vertex_array,
        last_polygon_mode,
        last_viewport,
        last_scissor_box,
        last_blend_src_rgb,
        last_blend_dst_rgb,
        last_blend_src_alpha,
        last_blend_dst_alpha,
        last_blend_equation_rgb,
        last_blend_equation_alpha,
        last_enable_blend,
        last_enable_cull_face,
        last_enable_depth_test,
        last_enable_scissor_test,
    }
}

unsafe fn restore(gl: &gl::Gl, backup: Backup) {
    let Backup {
        last_active_texture,
        last_program,
        last_texture,
        last_sampler,
        last_array_buffer,
        last_element_array_buffer,
        last_vertex_array,
        last_polygon_mode,
        last_viewport,
        last_scissor_box,
        last_blend_src_rgb,
        last_blend_dst_rgb,
        last_blend_src_alpha,
        last_blend_dst_alpha,
        last_blend_equation_rgb,
        last_blend_equation_alpha,
        last_enable_blend,
        last_enable_cull_face,
        last_enable_depth_test,
        last_enable_scissor_test,
    } = backup;
    gl.UseProgram(last_program as _);
    gl.BindTexture(gl::TEXTURE_2D, last_texture as _);
    if gl.BindSampler.is_loaded() {
        gl.BindSampler(0, last_sampler as _);
    }
    gl.ActiveTexture(last_active_texture as _);
    gl.BindVertexArray(last_vertex_array as _);
    gl.BindBuffer(gl::ARRAY_BUFFER, last_array_buffer as _);
    gl.BindBuffer(gl::ELEMENT_ARRAY_BUFFER, last_element_array_buffer as _);
    gl.BlendEquationSeparate(last_blend_equation_rgb as _, last_blend_equation_alpha as _);
    gl.BlendFuncSeparate(
        last_blend_src_rgb as _,
        last_blend_dst_rgb as _,
        last_blend_src_alpha as _,
        last_blend_dst_alpha as _,
    );
    if last_enable_blend {
        gl.Enable(gl::BLEND)
    } else {
        gl.Disable(gl::BLEND)
    };
    if last_enable_cull_face {
        gl.Enable(gl::CULL_FACE)
    } else {
        gl.Disable(gl::CULL_FACE)
    };
    if last_enable_depth_test {
        gl.Enable(gl::DEPTH_TEST)
    } else {
        gl.Disable(gl::DEPTH_TEST)
    };
    if last_enable_scissor_test {
        gl.Enable(gl::SCISSOR_TEST)
    } else {
        gl.Disable(gl::SCISSOR_TEST)
    };
    gl.PolygonMode(gl::FRONT_AND_BACK, last_polygon_mode[0] as _);
    gl.Viewport(
        last_viewport[0] as _,
        last_viewport[1] as _,
        last_viewport[2] as _,
        last_viewport[3] as _,
    );
    gl.Scissor(
        last_scissor_box[0] as _,
        last_scissor_box[1] as _,
        last_scissor_box[2] as _,
        last_scissor_box[3] as _,
    );
}

fn out_param<T: Default, F>(f: F) -> T
where
    F: FnOnce(&mut T),
{
    let mut val = Default::default();
    f(&mut val);
    val
}
