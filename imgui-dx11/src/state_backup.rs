use winapi::shared::dxgiformat::*;
use winapi::um::d3d11::*;

const D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE: usize = 16;

pub struct StateBackup {
    scissor_rects_count: u32,
    viewports_count: u32,
    scissor_rects: [D3D11_RECT; D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE],
    viewports: [D3D11_VIEWPORT; D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE],
    rasterizer_state: *mut ID3D11RasterizerState,
    blend_state: *mut ID3D11BlendState,
    blend_factor: [f32; 4],
    sample_mask: u32,
    stencil_ref: u32,
    depth_stencil_state: *mut ID3D11DepthStencilState,
    ps_shader_resource: *mut ID3D11ShaderResourceView,
    ps_sampler: *mut ID3D11SamplerState,
    pixel_shader: *mut ID3D11PixelShader,
    vertex_shader: *mut ID3D11VertexShader,
    ps_instances_count: u32,
    vs_instances_count: u32,
    ps_instances: [*mut ID3D11ClassInstance; 256],
    vs_instances: [*mut ID3D11ClassInstance; 256],
    primitive_topology: D3D11_PRIMITIVE_TOPOLOGY,
    index_buffer: *mut ID3D11Buffer,
    vertex_buffer: *mut ID3D11Buffer,
    vertex_contstant_buffer: *mut ID3D11Buffer,
    index_buffer_offset: u32,
    vertex_buffer_stride: u32,
    vertex_buffer_offset: u32,
    index_buffer_format: DXGI_FORMAT,
    input_layout: *mut ID3D11InputLayout,
}

impl StateBackup {
    pub fn backup(ctx: &ID3D11DeviceContext) -> StateBackup {
        let mut r: StateBackup = unsafe { std::mem::zeroed() };
        r.scissor_rects_count = D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE as _;
        r.viewports_count = D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE as _;
        unsafe {
            ctx.RSGetScissorRects(
                &mut r.scissor_rects_count,
                &mut r.scissor_rects as *mut _ as *mut _,
            );
            ctx.RSGetViewports(&mut r.viewports_count, &mut r.viewports as *mut _ as *mut _);
            ctx.RSGetState(&mut r.rasterizer_state);
            ctx.OMGetBlendState(&mut r.blend_state, &mut r.blend_factor, &mut r.sample_mask);
            ctx.OMGetDepthStencilState(&mut r.depth_stencil_state, &mut r.stencil_ref);
            ctx.PSGetShaderResources(0, 1, &mut r.ps_shader_resource);
            r.ps_instances_count = 256;
            r.vs_instances_count = 256;
            ctx.PSGetShader(
                &mut r.pixel_shader,
                &mut r.ps_instances as *mut _ as *mut *mut _,
                &mut r.ps_instances_count,
            );
            ctx.VSGetShader(
                &mut r.vertex_shader,
                &mut r.vs_instances as *mut _ as *mut *mut _,
                &mut r.vs_instances_count,
            );
            ctx.VSGetConstantBuffers(0, 1, &mut r.vertex_contstant_buffer);
            ctx.IAGetPrimitiveTopology(&mut r.primitive_topology);
            ctx.IAGetIndexBuffer(
                &mut r.index_buffer,
                &mut r.index_buffer_format,
                &mut r.index_buffer_offset,
            );
            ctx.IAGetVertexBuffers(
                0,
                1,
                &mut r.vertex_buffer,
                &mut r.vertex_buffer_stride,
                &mut r.vertex_buffer_offset,
            );
            ctx.IAGetInputLayout(&mut r.input_layout);
        }

        r
    }

    pub fn restore(self, ctx: &ID3D11DeviceContext) {
        unsafe {
            ctx.RSSetScissorRects(self.scissor_rects_count, &self.scissor_rects as *const _);
            ctx.RSSetViewports(self.viewports_count, &self.viewports as *const _);
            ctx.RSSetState(self.rasterizer_state);
            self.rasterizer_state
                .as_ref()
                .map(|r| r.Release())
                .unwrap_or(0);
            ctx.OMSetBlendState(self.blend_state, &self.blend_factor, self.sample_mask);
            self.blend_state.as_ref().map(|r| r.Release()).unwrap_or(0);
            ctx.OMSetDepthStencilState(self.depth_stencil_state, self.stencil_ref);
            self.depth_stencil_state
                .as_ref()
                .map(|r| r.Release())
                .unwrap_or(0);
            ctx.PSSetShaderResources(0, 1, &self.ps_shader_resource);
            self.ps_shader_resource
                .as_ref()
                .map(|r| r.Release())
                .unwrap_or(0);
            ctx.PSSetSamplers(0, 1, &self.ps_sampler);
            self.ps_sampler.as_ref().map(|r| r.Release()).unwrap_or(0);
            ctx.PSSetShader(
                self.pixel_shader,
                &self.ps_instances as *const _,
                self.ps_instances_count,
            );
            self.pixel_shader.as_ref().map(|r| r.Release()).unwrap_or(0);
            for i in 0..self.ps_instances_count {
                self.ps_instances[i as usize]
                    .as_ref()
                    .map(|r| r.Release())
                    .unwrap_or(0);
            }
            ctx.VSSetShader(
                self.vertex_shader,
                &self.vs_instances as *const _,
                self.vs_instances_count,
            );
            self.vertex_shader
                .as_ref()
                .map(|r| r.Release())
                .unwrap_or(0);
            for i in 0..self.vs_instances_count {
                self.vs_instances[i as usize]
                    .as_ref()
                    .map(|r| r.Release())
                    .unwrap_or(0);
            }
            ctx.IASetPrimitiveTopology(self.primitive_topology);
            ctx.IASetIndexBuffer(
                self.index_buffer,
                self.index_buffer_format,
                self.index_buffer_offset,
            );
            self.index_buffer.as_ref().map(|r| r.Release()).unwrap_or(0);
            ctx.IASetVertexBuffers(
                0,
                1,
                &self.vertex_buffer,
                &self.vertex_buffer_stride,
                &self.vertex_buffer_offset,
            );
            self.vertex_buffer
                .as_ref()
                .map(|r| r.Release())
                .unwrap_or(0);
            ctx.IASetInputLayout(self.input_layout);
            self.input_layout.as_ref().map(|r| r.Release()).unwrap_or(0);
        }
    }
}
