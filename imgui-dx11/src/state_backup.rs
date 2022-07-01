use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11BlendState, ID3D11Buffer, ID3D11ClassInstance, ID3D11DepthStencilState,
    ID3D11DeviceContext, ID3D11InputLayout, ID3D11PixelShader, ID3D11RasterizerState,
    ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11VertexShader, D3D11_VIEWPORT,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT;

const D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE: usize = 16;

pub struct StateBackup {
    scissor_rects_count: u32,
    viewports_count: u32,
    scissor_rects: [RECT; D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE],
    viewports: [D3D11_VIEWPORT; D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE],
    rasterizer_state: Option<ID3D11RasterizerState>,
    blend_state: Option<ID3D11BlendState>,
    blend_factor: [f32; 4],
    sample_mask: u32,
    stencil_ref: u32,
    depth_stencil_state: Option<ID3D11DepthStencilState>,
    ps_shader_resource: [Option<ID3D11ShaderResourceView>; 1],
    ps_sampler: Option<ID3D11SamplerState>,
    pixel_shader: Option<ID3D11PixelShader>,
    vertex_shader: Option<ID3D11VertexShader>,
    ps_instances_count: u32,
    vs_instances_count: u32,
    ps_instances: Vec<Option<ID3D11ClassInstance>>,
    vs_instances: Vec<Option<ID3D11ClassInstance>>,
    primitive_topology: D3D_PRIMITIVE_TOPOLOGY,
    index_buffer: Option<ID3D11Buffer>,
    vertex_buffer: Option<ID3D11Buffer>,
    vertex_constant_buffer: [Option<ID3D11Buffer>; 1],
    index_buffer_offset: u32,
    vertex_buffer_stride: u32,
    vertex_buffer_offset: u32,
    index_buffer_format: DXGI_FORMAT,
    input_layout: Option<ID3D11InputLayout>,
}

impl Default for StateBackup {
    fn default() -> Self {
        Self {
            ps_instances: vec![None; 256],
            vs_instances: vec![None; 256],
            scissor_rects_count: Default::default(),
            viewports_count: Default::default(),
            scissor_rects: Default::default(),
            viewports: Default::default(),
            rasterizer_state: Default::default(),
            blend_state: Default::default(),
            blend_factor: Default::default(),
            sample_mask: Default::default(),
            stencil_ref: Default::default(),
            depth_stencil_state: Default::default(),
            ps_shader_resource: Default::default(),
            ps_sampler: Default::default(),
            pixel_shader: Default::default(),
            vertex_shader: Default::default(),
            ps_instances_count: Default::default(),
            vs_instances_count: Default::default(),
            primitive_topology: Default::default(),
            index_buffer: Default::default(),
            vertex_buffer: Default::default(),
            vertex_constant_buffer: Default::default(),
            index_buffer_offset: Default::default(),
            vertex_buffer_stride: Default::default(),
            vertex_buffer_offset: Default::default(),
            index_buffer_format: Default::default(),
            input_layout: Default::default(),
        }
    }
}

impl StateBackup {
    pub fn backup(ctx: ID3D11DeviceContext) -> StateBackup {
        let mut r: StateBackup = StateBackup {
            scissor_rects_count: D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE as _,
            viewports_count: D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE as _,
            ..Default::default()
        };
        unsafe {
            ctx.RSGetScissorRects(
                &mut r.scissor_rects_count,
                &mut r.scissor_rects as *mut _ as *mut _,
            );
            ctx.RSGetViewports(&mut r.viewports_count, &mut r.viewports as *mut _ as *mut _);
            ctx.RSGetState(&mut r.rasterizer_state);
            ctx.OMGetBlendState(&mut r.blend_state, &mut r.blend_factor as _, &mut r.sample_mask);
            ctx.OMGetDepthStencilState(&mut r.depth_stencil_state, &mut r.stencil_ref);
            ctx.PSGetShaderResources(0, &mut r.ps_shader_resource);
            r.ps_instances_count = 256;
            r.vs_instances_count = 256;
            ctx.PSGetShader(&mut r.pixel_shader, &mut r.ps_instances[0], &mut r.ps_instances_count);
            ctx.VSGetShader(
                &mut r.vertex_shader,
                &mut r.vs_instances[0],
                &mut r.vs_instances_count,
            );
            ctx.VSGetConstantBuffers(0, &mut r.vertex_constant_buffer);
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

    pub fn restore(self, ctx: ID3D11DeviceContext) {
        unsafe {
            ctx.RSSetScissorRects(&self.scissor_rects);
            ctx.RSSetViewports(&self.viewports);
            ctx.RSSetState(self.rasterizer_state);
            // self.rasterizer_state
            //     .as_ref()
            //     .map(|r| r.Release())
            //     .unwrap_or(0);
            ctx.OMSetBlendState(self.blend_state, &self.blend_factor as _, self.sample_mask);
            // self.blend_state.as_ref().map(|r| r.Release()).unwrap_or(0);
            ctx.OMSetDepthStencilState(self.depth_stencil_state, self.stencil_ref);
            // self.depth_stencil_state
            //     .as_ref()
            //     .map(|r| r.Release())
            //     .unwrap_or(0);
            ctx.PSSetShaderResources(0, &self.ps_shader_resource);
            // self.ps_shader_resource
            //     .as_ref()
            //     .map(|r| r.Release())
            //     .unwrap_or(0);
            ctx.PSSetSamplers(0, &[self.ps_sampler]);
            // self.ps_sampler.as_ref().map(|r| r.Release()).unwrap_or(0);
            ctx.PSSetShader(self.pixel_shader, &self.ps_instances);
            // self.pixel_shader.as_ref().map(|r| r.Release()).unwrap_or(0);
            // for i in 0..self.ps_instances_count {
            //     self.ps_instances[i as usize]
            //         .as_ref()
            //         .map(|r| r.Release())
            //         .unwrap_or(0);
            // }
            ctx.VSSetShader(self.vertex_shader, &self.vs_instances);
            // self.vertex_shader
            //     .as_ref()
            //     .map(|r| r.Release())
            //     .unwrap_or(0);
            // for i in 0..self.vs_instances_count {
            //     self.vs_instances[i as usize]
            //         .as_ref()
            //         .map(|r| r.Release())
            //         .unwrap_or(0);
            // }
            ctx.IASetPrimitiveTopology(self.primitive_topology);
            ctx.IASetIndexBuffer(
                self.index_buffer,
                self.index_buffer_format,
                self.index_buffer_offset,
            );
            // self.index_buffer.as_ref().map(|r| r.Release()).unwrap_or(0);
            ctx.IASetVertexBuffers(
                0,
                1,
                &self.vertex_buffer,
                &self.vertex_buffer_stride,
                &self.vertex_buffer_offset,
            );
            // self.vertex_buffer
            //     .as_ref()
            //     .map(|r| r.Release())
            //     .unwrap_or(0);
            ctx.IASetInputLayout(self.input_layout);
            // self.input_layout.as_ref().map(|r| r.Release()).unwrap_or(0);
        }
    }
}
