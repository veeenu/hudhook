// Based on https://github.com/michaelfairley/rust-imgui-opengl-renderer/

use std::ffi::{c_void, CString};
use std::mem;

use gl::types::*;
use imgui::internal::RawWrapper;
use imgui::{Context, DrawCmd, DrawData, DrawIdx, DrawVert, TextureId};
use memoffset::offset_of;
use once_cell::sync::OnceCell;
use tracing::error;
use windows::core::{s, Error, Result, HRESULT, PCSTR};
use windows::Win32::Foundation::{FARPROC, HINSTANCE};
use windows::Win32::Graphics::OpenGL::*;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

use crate::renderer::RenderEngine;
use crate::{util, RenderContext};

mod gl {
    #![allow(
        clippy::unreadable_literal,
        clippy::too_many_arguments,
        clippy::unused_unit,
        clippy::upper_case_acronyms,
        clippy::manual_non_exhaustive,

        // We support stable but lint on nightly. The following lint isn't available on stable,
        // so allow `unknown_lints`.
        unknown_lints,
        clippy::missing_transmute_annotations
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

pub struct OpenGl3RenderEngine {
    gl: gl::Gl,
    program: GLuint,

    projection_loc: GLuint,
    position_loc: GLuint,
    color_loc: GLuint,
    uv_loc: GLuint,
    texture_loc: GLuint,

    vao: GLuint,

    vertex_buffer: GLuint,
    index_buffer: GLuint,
    projection_buffer: [[f32; 4]; 4],

    texture_heap: TextureHeap,
}

impl OpenGl3RenderEngine {
    pub fn new(ctx: &mut Context) -> Result<Self> {
        let gl = gl::Gl::load_with(|s| unsafe { load_func(CString::new(s).unwrap()) });

        let (program, projection_loc, position_loc, color_loc, uv_loc, texture_loc) =
            unsafe { create_shader_program(&gl) };

        let vertex_buffer = util::out_param(|x| unsafe { gl.GenBuffers(1, x) });
        let index_buffer = util::out_param(|x| unsafe { gl.GenBuffers(1, x) });
        let projection_buffer = Default::default();

        let vao = util::out_param(|x| unsafe { gl.GenVertexArrays(1, x) });

        let texture_heap = TextureHeap::new();

        ctx.set_ini_filename(None);
        ctx.set_renderer_name(String::from(concat!("hudhook-opengl3@", env!("CARGO_PKG_VERSION"))));

        Ok(Self {
            gl,
            program,
            projection_loc,
            position_loc,
            color_loc,
            uv_loc,
            texture_loc,
            vao,
            vertex_buffer,
            index_buffer,
            projection_buffer,
            texture_heap,
        })
    }
}

impl RenderContext for OpenGl3RenderEngine {
    fn load_texture(&mut self, data: &[u8], width: u32, height: u32) -> Result<TextureId> {
        unsafe { self.texture_heap.create_texture(&self.gl, data, width, height) }
    }

    fn replace_texture(
        &mut self,
        texture_id: TextureId,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<()> {
        unsafe { self.texture_heap.update_texture(&self.gl, texture_id, data, width, height) }
    }
}

impl RenderEngine for OpenGl3RenderEngine {
    type RenderTarget = ();

    fn render(&mut self, draw_data: &DrawData, _render_target: Self::RenderTarget) -> Result<()> {
        unsafe {
            let state_backup = StateBackup::backup(&self.gl);
            self.render_draw_data(draw_data)?;
            state_backup.restore(&self.gl);
        }
        Ok(())
    }

    fn setup_fonts(&mut self, ctx: &mut Context) -> Result<()> {
        let fonts = ctx.fonts();
        let fonts_texture = fonts.build_rgba32_texture();
        fonts.tex_id =
            self.load_texture(fonts_texture.data, fonts_texture.width, fonts_texture.height)?;
        Ok(())
    }
}

impl OpenGl3RenderEngine {
    unsafe fn render_draw_data(&mut self, draw_data: &DrawData) -> Result<()> {
        let [clip_offset_x, clip_offset_y] = draw_data.display_pos;
        let [clip_scale_w, clip_scale_h] = draw_data.framebuffer_scale;
        let fb_height = clip_scale_h * draw_data.display_size[1];

        self.projection_buffer = {
            let [l, t, r, b] = [
                draw_data.display_pos[0],
                draw_data.display_pos[1],
                draw_data.display_pos[0] + draw_data.display_size[0],
                draw_data.display_pos[1] + draw_data.display_size[1],
            ];

            [[2. / (r - l), 0., 0., 0.], [0., 2. / (t - b), 0., 0.], [0., 0., 0.5, 0.], [
                (r + l) / (l - r),
                (t + b) / (b - t),
                0.5,
                1.0,
            ]]
        };

        self.setup_render_state(draw_data);

        for cl in draw_data.draw_lists() {
            for cmd in cl.commands() {
                match cmd {
                    DrawCmd::Elements { count, cmd_params } => {
                        let [cx, cy, cz, cw] = cmd_params.clip_rect;

                        let clip_min_x = (cx - clip_offset_x) * clip_scale_w;
                        let clip_min_y = (cy - clip_offset_y) * clip_scale_h;
                        let clip_max_x = (cz - clip_offset_x) * clip_scale_w;
                        let clip_max_y = (cw - clip_offset_y) * clip_scale_h;

                        if clip_max_x <= clip_min_x || clip_max_y <= clip_min_y {
                            continue;
                        }

                        self.gl.Scissor(
                            clip_min_x as i32,
                            (fb_height - clip_max_y) as i32,
                            (clip_max_x - clip_min_x) as i32,
                            (clip_max_y - clip_min_y) as i32,
                        );
                        self.gl.ActiveTexture(gl::TEXTURE0);
                        self.gl.BindTexture(
                            gl::TEXTURE_2D,
                            self.texture_heap.get(cmd_params.texture_id).gl_texture,
                        );

                        self.gl.BufferData(
                            gl::ARRAY_BUFFER,
                            std::mem::size_of_val(cl.vtx_buffer()) as isize,
                            cl.vtx_buffer().as_ptr() as *const c_void,
                            gl::STREAM_DRAW,
                        );
                        self.gl.BufferData(
                            gl::ELEMENT_ARRAY_BUFFER,
                            std::mem::size_of_val(cl.idx_buffer()) as isize,
                            cl.idx_buffer().as_ptr() as *const c_void,
                            gl::STREAM_DRAW,
                        );

                        self.gl.DrawElements(
                            gl::TRIANGLES,
                            count as GLint,
                            if mem::size_of::<DrawIdx>() == 2 {
                                gl::UNSIGNED_SHORT
                            } else {
                                gl::UNSIGNED_INT
                            },
                            (cmd_params.idx_offset * mem::size_of::<DrawIdx>()) as *const c_void,
                        );
                    },
                    DrawCmd::ResetRenderState => {
                        self.setup_render_state(draw_data);
                    },
                    DrawCmd::RawCallback { callback, raw_cmd } => callback(cl.raw(), raw_cmd),
                }
            }
        }

        Ok(())
    }

    unsafe fn setup_render_state(&mut self, draw_data: &DrawData) {
        self.gl.Enable(gl::BLEND);
        self.gl.BlendEquation(gl::FUNC_ADD);
        self.gl.BlendFuncSeparate(
            gl::SRC_ALPHA,
            gl::ONE_MINUS_SRC_ALPHA,
            gl::ONE,
            gl::ONE_MINUS_SRC_ALPHA,
        );
        self.gl.Disable(gl::CULL_FACE);
        self.gl.Disable(gl::DEPTH_TEST);
        self.gl.Disable(gl::STENCIL_TEST);
        self.gl.Enable(gl::SCISSOR_TEST);
        self.gl.PolygonMode(gl::FRONT_AND_BACK, gl::FILL);

        self.gl.Viewport(
            0,
            0,
            (draw_data.framebuffer_scale[0] * draw_data.display_size[0]) as GLint,
            (draw_data.framebuffer_scale[1] * draw_data.display_size[1]) as GLint,
        );

        self.gl.UseProgram(self.program);
        self.gl.Uniform1i(self.texture_loc as GLint, 0);
        self.gl.UniformMatrix4fv(
            self.projection_loc as i32,
            1,
            gl::FALSE,
            self.projection_buffer.as_ptr() as *const f32,
        );
        if self.gl.BindSampler.is_loaded() {
            self.gl.BindSampler(0, 0);
        }

        self.gl.BindVertexArray(self.vao);
        self.gl.BindBuffer(gl::ARRAY_BUFFER, self.vertex_buffer);
        self.gl.BindBuffer(gl::ELEMENT_ARRAY_BUFFER, self.index_buffer);
        self.gl.EnableVertexAttribArray(self.position_loc);
        self.gl.EnableVertexAttribArray(self.uv_loc);
        self.gl.EnableVertexAttribArray(self.color_loc);
        self.gl.VertexAttribPointer(
            self.position_loc,
            2,
            gl::FLOAT,
            gl::FALSE,
            mem::size_of::<DrawVert>() as GLint,
            offset_of!(DrawVert, pos) as *const c_void,
        );
        self.gl.VertexAttribPointer(
            self.uv_loc,
            2,
            gl::FLOAT,
            gl::FALSE,
            mem::size_of::<DrawVert>() as GLint,
            offset_of!(DrawVert, uv) as *const c_void,
        );
        self.gl.VertexAttribPointer(
            self.color_loc,
            4,
            gl::UNSIGNED_BYTE,
            gl::TRUE,
            mem::size_of::<DrawVert>() as GLint,
            offset_of!(DrawVert, col) as *const c_void,
        );
    }
}

unsafe fn create_shader_program(gl: &gl::Gl) -> (GLuint, GLuint, GLuint, GLuint, GLuint, GLuint) {
    const VS: &[u8] = b"
    #version 130

    uniform mat4 projection_matrix;
    in vec2 position;
    in vec4 color;
    in vec2 uv;

    out vec2 frag_uv;
    out vec4 frag_color;

    void main() {
        frag_uv = uv;
        frag_color = color;
        gl_Position = projection_matrix * vec4(position.xy, 0.0, 1.0);
    }
    \0";

    const FS: &[u8] = b"
    #version 130

    uniform sampler2D tex;
    in vec2 frag_uv;
    in vec4 frag_color;
    out vec4 out_color;

    void main() {
        out_color = frag_color * texture(tex, frag_uv.st);
    }
    \0";

    let program = gl.CreateProgram();
    let vertex_shader = gl.CreateShader(gl::VERTEX_SHADER);
    let fragment_shader = gl.CreateShader(gl::FRAGMENT_SHADER);
    let vertex_source = [VS.as_ptr() as *const GLchar];
    let fragment_source = [FS.as_ptr() as *const GLchar];
    let vertex_source_len = [VS.len() as i32];
    let fragment_source_len = [FS.len() as i32];
    gl.ShaderSource(vertex_shader, 1, vertex_source.as_ptr(), vertex_source_len.as_ptr());
    gl.ShaderSource(fragment_shader, 1, fragment_source.as_ptr(), fragment_source_len.as_ptr());
    gl.CompileShader(vertex_shader);
    gl.CompileShader(fragment_shader);
    gl.AttachShader(program, vertex_shader);
    gl.AttachShader(program, fragment_shader);
    gl.LinkProgram(program);
    gl.DeleteShader(vertex_shader);
    gl.DeleteShader(fragment_shader);

    let projection_loc =
        gl.GetUniformLocation(program, b"projection_matrix\0".as_ptr() as _) as GLuint;
    let position_loc = gl.GetAttribLocation(program, b"position\0".as_ptr() as _) as GLuint;
    let color_loc = gl.GetAttribLocation(program, b"color\0".as_ptr() as _) as GLuint;
    let uv_loc = gl.GetAttribLocation(program, b"uv\0".as_ptr() as _) as GLuint;
    let texture_loc = gl.GetUniformLocation(program, b"tex\0".as_ptr() as _) as GLuint;

    (program, projection_loc, position_loc, color_loc, uv_loc, texture_loc)
}

struct TextureHeap {
    textures: Vec<Texture>,
}
struct Texture {
    gl_texture: GLuint,
    width: u32,
    height: u32,
}

impl TextureHeap {
    fn new() -> Self {
        Self { textures: Vec::new() }
    }

    fn get(&self, texture_id: TextureId) -> &Texture {
        &self.textures[texture_id.id()]
    }

    unsafe fn create_texture(
        &mut self,
        gl: &gl::Gl,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<TextureId> {
        let texture = util::out_param(|x| gl.GenTextures(1, x));

        let mut bound_texture = 0;
        gl.GetIntegerv(gl::TEXTURE_BINDING_2D, &mut bound_texture);

        gl.ActiveTexture(gl::TEXTURE0);
        gl.BindTexture(gl::TEXTURE_2D, texture);
        gl.TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as _);
        gl.TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as _);

        gl.TexImage2D(
            gl::TEXTURE_2D,
            0,
            gl::RGBA as GLint,
            width as GLint,
            height as GLint,
            0,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            data.as_ptr() as *const c_void,
        );
        gl.BindTexture(gl::TEXTURE_2D, bound_texture as _);

        let id = TextureId::from(self.textures.len());
        self.textures.push(Texture { gl_texture: texture, width, height });

        Ok(id)
    }

    unsafe fn update_texture(
        &mut self,
        gl: &gl::Gl,
        texture: TextureId,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Result<()> {
        let texture_info = self.get(texture);
        if texture_info.width != width || texture_info.height != height {
            error!(
                "image size {width}x{height} do not match expected {}x{}",
                texture_info.width, texture_info.height
            );
            return Err(Error::from_hresult(HRESULT(-1)));
        }

        let mut bound_texture = 0;
        gl.GetIntegerv(gl::TEXTURE_BINDING_2D, &mut bound_texture);

        gl.ActiveTexture(gl::TEXTURE0);
        gl.BindTexture(gl::TEXTURE_2D, texture_info.gl_texture);

        gl.TexSubImage2D(
            gl::TEXTURE_2D,
            0,
            0,
            0,
            width as GLint,
            height as GLint,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            data.as_ptr() as *const c_void,
        );

        gl.BindTexture(gl::TEXTURE_2D, bound_texture as _);

        Ok(())
    }
}

struct StateBackup {
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

impl StateBackup {
    unsafe fn backup(gl: &gl::Gl) -> StateBackup {
        let last_active_texture = util::out_param(|x| gl.GetIntegerv(gl::ACTIVE_TEXTURE, x));
        gl.ActiveTexture(gl::TEXTURE0);
        let last_program = util::out_param(|x| gl.GetIntegerv(gl::CURRENT_PROGRAM, x));
        let last_texture = util::out_param(|x| gl.GetIntegerv(gl::TEXTURE_BINDING_2D, x));
        let last_sampler = if gl.BindSampler.is_loaded() {
            util::out_param(|x| gl.GetIntegerv(gl::SAMPLER_BINDING, x))
        } else {
            0
        };
        let last_array_buffer = util::out_param(|x| gl.GetIntegerv(gl::ARRAY_BUFFER_BINDING, x));
        let last_element_array_buffer =
            util::out_param(|x| gl.GetIntegerv(gl::ELEMENT_ARRAY_BUFFER_BINDING, x));
        let last_vertex_array = util::out_param(|x| gl.GetIntegerv(gl::VERTEX_ARRAY_BINDING, x));
        let last_polygon_mode =
            util::out_param(|x: &mut [GLint; 2]| gl.GetIntegerv(gl::POLYGON_MODE, x.as_mut_ptr()));
        let last_viewport =
            util::out_param(|x: &mut [GLint; 4]| gl.GetIntegerv(gl::VIEWPORT, x.as_mut_ptr()));
        let last_scissor_box =
            util::out_param(|x: &mut [GLint; 4]| gl.GetIntegerv(gl::SCISSOR_BOX, x.as_mut_ptr()));
        let last_blend_src_rgb = util::out_param(|x| gl.GetIntegerv(gl::BLEND_SRC_RGB, x));
        let last_blend_dst_rgb = util::out_param(|x| gl.GetIntegerv(gl::BLEND_DST_RGB, x));
        let last_blend_src_alpha = util::out_param(|x| gl.GetIntegerv(gl::BLEND_SRC_ALPHA, x));
        let last_blend_dst_alpha = util::out_param(|x| gl.GetIntegerv(gl::BLEND_DST_ALPHA, x));
        let last_blend_equation_rgb =
            util::out_param(|x| gl.GetIntegerv(gl::BLEND_EQUATION_RGB, x));
        let last_blend_equation_alpha =
            util::out_param(|x| gl.GetIntegerv(gl::BLEND_EQUATION_ALPHA, x));
        let last_enable_blend = gl.IsEnabled(gl::BLEND) == gl::TRUE;
        let last_enable_cull_face = gl.IsEnabled(gl::CULL_FACE) == gl::TRUE;
        let last_enable_depth_test = gl.IsEnabled(gl::DEPTH_TEST) == gl::TRUE;
        let last_enable_scissor_test = gl.IsEnabled(gl::SCISSOR_TEST) == gl::TRUE;

        StateBackup {
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

    unsafe fn restore(self, gl: &gl::Gl) {
        let StateBackup {
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
        } = self;

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
}
