use std::ffi::CStr;

use ash::vk;
use ash::{
    extensions::{
        ext::DebugUtils,
        khr::{Surface, Swapchain},
    },
    vk::{
        make_api_version, AccessFlags, ApplicationInfo, AttachmentDescription, AttachmentLoadOp,
        AttachmentReference, AttachmentStoreOp, BlendFactor, BlendOp, ClearColorValue, ClearValue,
        ColorComponentFlags, ColorSpaceKHR, CommandBuffer, CommandBufferAllocateInfo,
        CommandBufferBeginInfo, CommandBufferLevel, CommandPool, CommandPoolCreateFlags,
        CommandPoolCreateInfo, ComponentMapping, ComponentSwizzle, CompositeAlphaFlagsKHR,
        CullModeFlags, DeviceCreateInfo, DeviceQueueCreateInfo, DynamicState, Extent2D, Fence,
        FenceCreateFlags, FenceCreateInfo, Format, Framebuffer, FramebufferCreateInfo, FrontFace,
        GraphicsPipelineCreateInfo, Image, ImageAspectFlags, ImageLayout, ImageSubresourceRange,
        ImageUsageFlags, ImageView, ImageViewCreateInfo, ImageViewType, InstanceCreateFlags,
        InstanceCreateInfo, LogicOp, Offset2D, PhysicalDevice, PhysicalDeviceFeatures,
        PhysicalDeviceType, Pipeline, PipelineBindPoint, PipelineCache,
        PipelineColorBlendAttachmentState, PipelineColorBlendStateCreateInfo,
        PipelineDynamicStateCreateInfo, PipelineInputAssemblyStateCreateInfo, PipelineLayout,
        PipelineLayoutCreateInfo, PipelineMultisampleStateCreateInfo,
        PipelineRasterizationStateCreateInfo, PipelineShaderStageCreateInfo, PipelineStageFlags,
        PipelineVertexInputStateCreateInfo, PipelineViewportStateCreateInfo, PolygonMode,
        PresentInfoKHR, PresentModeKHR, PrimitiveTopology, Queue, QueueFlags, Rect2D, RenderPass,
        RenderPassBeginInfo, RenderPassCreateInfo, SampleCountFlags, Semaphore, ShaderModule,
        ShaderModuleCreateInfo, ShaderStageFlags, SharingMode, SubmitInfo, SubpassContents,
        SubpassDependency, SubpassDescription, SurfaceCapabilitiesKHR, SurfaceFormatKHR,
        SurfaceKHR, SwapchainCreateInfoKHR, SwapchainKHR, Viewport, SUBPASS_EXTERNAL,
    },
    Device, Entry, Instance,
};
use once_cell::sync::Lazy;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use thiserror::Error;
use windows::Win32::Foundation::{HINSTANCE, HWND};

use crate::util;

#[derive(Error, Debug)]
pub enum Error {
    #[error("vulkan error")]
    Vk(#[from] vk::Result),
    #[error("error")]
    Other(String),
}

type Result<T> = std::result::Result<T, Error>;

static VERTEX_SHADER: Lazy<Vec<u32>> =
    Lazy::new(|| include_u32(include_bytes!("shaders/shader.vert.spv")));
static FRAGMENT_SHADER: Lazy<Vec<u32>> =
    Lazy::new(|| include_u32(include_bytes!("shaders/shader.frag.spv")));

fn include_u32(bytes: &[u8]) -> Vec<u32> {
    let spv: Vec<u32> = (0..bytes.len() / 4)
        .map(|i| i * 4)
        .map(|i| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]))
        .collect();

    assert_eq!(spv[0], 0x0723_0203);

    spv
}

pub struct Vulkan {
    instance: Instance,
    device: Device,

    width: i32,
    height: i32,

    swapchain: Swapchain,
    swapchain_khr: SwapchainKHR,
    swapchain_images: Vec<Image>,
    swapchain_image_views: Vec<ImageView>,
    swapchain_extent: Extent2D,

    surface: Surface,
    surface_khr: SurfaceKHR,

    graphics_queue: Queue,
    present_queue: Queue,

    vertex_shader: ShaderModule,
    fragment_shader: ShaderModule,

    pipeline_layout: PipelineLayout,
    render_pass: RenderPass,
    pipelines: Vec<Pipeline>,

    framebuffers: Vec<Framebuffer>,

    command_pool: CommandPool,
    command_buffers: Vec<CommandBuffer>,

    semaphore_image_available: Semaphore,
    semaphore_render_finished: Semaphore,
    fence_in_flight: Fence,
}

impl Vulkan {
    pub fn new(hwnd: HWND, hinstance: HINSTANCE) -> Result<Self> {
        let display_handle = RawDisplayHandle::Windows(WindowsDisplayHandle::empty());
        let mut window_handle = Win32WindowHandle::empty();
        window_handle.hwnd = hwnd.0 as _;
        window_handle.hinstance = hinstance.0 as _;
        let window_handle = RawWindowHandle::Win32(window_handle);

        let (width, height) = util::win_size(hwnd);

        let entry = Entry::linked();

        println!("Extension properties:");
        entry.enumerate_instance_extension_properties(None)?.into_iter().for_each(|ep| {
            println!("  - {ep:?}");
        });

        println!("\nLayer properties:");
        entry.enumerate_instance_layer_properties()?.into_iter().for_each(|lp| {
            println!("  - {lp:?}");
        });

        let app_info = ApplicationInfo::builder()
            .application_name(unsafe { CStr::from_bytes_with_nul_unchecked(b"Vulkan Tutorial\0") })
            .application_version(make_api_version(0, 0, 1, 0))
            .engine_name(unsafe { CStr::from_bytes_with_nul_unchecked(b"No Engine\0") })
            .engine_version(make_api_version(0, 0, 1, 0))
            .api_version(make_api_version(0, 1, 0, 0))
            .build();

        let layer_names = [b"VK_LAYER_KHRONOS_validation\0".as_ptr() as *const i8];

        let mut extension_names =
            ash_window::enumerate_required_extensions(display_handle).unwrap().to_vec();
        extension_names.push(DebugUtils::name().as_ptr());

        println!("Creating instance");

        let instance_create_info = InstanceCreateInfo::builder()
            .application_info(&app_info)
            .enabled_layer_names(&layer_names)
            .enabled_extension_names(&extension_names)
            .flags(if cfg!(any(target_os = "macos", target_os = "ios")) {
                InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR
            } else {
                InstanceCreateFlags::default()
            })
            .build();

        let instance = unsafe { entry.create_instance(&instance_create_info, None)? };

        let surface = Surface::new(&entry, &instance);

        let surface_khr = unsafe {
            ash_window::create_surface(&entry, &instance, display_handle, window_handle, None)
        }?;

        println!("Creating physical device and finding queue family index");

        let (physical_device, queue_families, swapchain_support) =
            unsafe { instance.enumerate_physical_devices()? }
                .into_iter()
                .find_map(|device| is_device_suitable(&instance, device, &surface, surface_khr))
                .ok_or_else(|| {
                    Error::Other("Could not find suitable physical device".to_string())
                })?;

        println!("Creating logical device");

        let device_queue_create_info_graphics = DeviceQueueCreateInfo::builder()
            .queue_family_index(queue_families.graphics_queue)
            .queue_priorities(&[1.0f32])
            .build();

        let device_queue_create_info_present = DeviceQueueCreateInfo::builder()
            .queue_family_index(queue_families.present_queue)
            .queue_priorities(&[1.0f32])
            .build();

        let device_queue_create_infos =
            if queue_families.present_queue == queue_families.graphics_queue {
                vec![device_queue_create_info_graphics]
            } else {
                vec![device_queue_create_info_graphics, device_queue_create_info_present]
            };

        println!("{device_queue_create_infos:?}");

        let device_features = PhysicalDeviceFeatures::builder().build();

        let device_extension_names = [Swapchain::name().as_ptr()];

        let device_create_info = DeviceCreateInfo::builder()
            .queue_create_infos(&device_queue_create_infos)
            .enabled_features(&device_features)
            .enabled_extension_names(&device_extension_names)
            .build();

        let device = unsafe { instance.create_device(physical_device, &device_create_info, None)? };

        println!("Retrieving device queues");

        let graphics_queue = unsafe { device.get_device_queue(queue_families.graphics_queue, 0) };
        let present_queue = unsafe { device.get_device_queue(queue_families.present_queue, 0) };

        println!("Creating swap chain");

        let swapchain_create_info = swapchain_support.create_info(&queue_families, width, height);

        let swapchain = Swapchain::new(&instance, &device);
        let swapchain_khr = unsafe { swapchain.create_swapchain(&swapchain_create_info, None)? };
        let swapchain_images = unsafe { swapchain.get_swapchain_images(swapchain_khr) }?;
        let swapchain_extent = swapchain_create_info.image_extent;

        let image_view_create_info = |image| {
            ImageViewCreateInfo::builder()
                .view_type(ImageViewType::TYPE_2D)
                .format(swapchain_create_info.image_format)
                .components(
                    ComponentMapping::builder()
                        .r(ComponentSwizzle::IDENTITY)
                        .g(ComponentSwizzle::IDENTITY)
                        .b(ComponentSwizzle::IDENTITY)
                        .a(ComponentSwizzle::IDENTITY)
                        .build(),
                )
                .subresource_range(
                    ImageSubresourceRange::builder()
                        .aspect_mask(ImageAspectFlags::COLOR)
                        .base_mip_level(0)
                        .level_count(1)
                        .base_array_layer(0)
                        .layer_count(1)
                        .build(),
                )
                .image(image)
                .build()
        };

        println!("Creating swap chain image views");

        let swapchain_image_views = swapchain_images
            .iter()
            .copied()
            .map(|image| unsafe {
                let image_view_create_info = image_view_create_info(image);
                device.create_image_view(&image_view_create_info, None).map_err(Error::from)
            })
            .collect::<Result<Vec<_>>>()?;

        println!("Creating shader modules");

        let shader_module_create_info_vert =
            ShaderModuleCreateInfo::builder().code(VERTEX_SHADER.as_ref()).build();

        let shader_module_create_info_frag =
            ShaderModuleCreateInfo::builder().code(FRAGMENT_SHADER.as_ref()).build();

        let vertex_shader =
            unsafe { device.create_shader_module(&shader_module_create_info_vert, None) }?;

        let fragment_shader =
            unsafe { device.create_shader_module(&shader_module_create_info_frag, None) }?;

        println!("Creating shader stages");

        let pipeline_shader_stage_create_info_vert = PipelineShaderStageCreateInfo::builder()
            .stage(ShaderStageFlags::VERTEX)
            .module(vertex_shader)
            .name(unsafe { CStr::from_bytes_with_nul_unchecked(b"main\0") })
            .build();

        let pipeline_shader_stage_create_info_frag = PipelineShaderStageCreateInfo::builder()
            .stage(ShaderStageFlags::FRAGMENT)
            .module(fragment_shader)
            .name(unsafe { CStr::from_bytes_with_nul_unchecked(b"main\0") })
            .build();

        println!("Creating pipeline state");

        let pipeline_dynamic_state_create_info = PipelineDynamicStateCreateInfo::builder()
            .dynamic_states(&[DynamicState::VIEWPORT, DynamicState::SCISSOR])
            .build();

        let pipeline_vertex_input_state_create_info = PipelineVertexInputStateCreateInfo::builder()
            .vertex_binding_descriptions(&[])
            .vertex_attribute_descriptions(&[])
            .build();

        let pipeline_input_assembly_state_create_info =
            PipelineInputAssemblyStateCreateInfo::builder()
                .topology(PrimitiveTopology::TRIANGLE_LIST)
                .primitive_restart_enable(false)
                .build();

        let viewport = Viewport::builder()
            .x(0.)
            .y(0.)
            .width(swapchain_extent.width as f32)
            .height(swapchain_extent.height as f32)
            .min_depth(0.)
            .max_depth(1.)
            .build();

        let scissor =
            Rect2D::builder().offset(Offset2D { x: 0, y: 0 }).extent(swapchain_extent).build();

        let pipeline_viewport_state_create_info = PipelineViewportStateCreateInfo::builder()
            .viewports(&[viewport])
            .scissors(&[scissor])
            .build();

        let pipeline_rasterization_state_create_info =
            PipelineRasterizationStateCreateInfo::builder()
                .depth_clamp_enable(false)
                .rasterizer_discard_enable(false)
                .polygon_mode(PolygonMode::FILL)
                .line_width(1.)
                .cull_mode(CullModeFlags::BACK)
                .front_face(FrontFace::CLOCKWISE)
                .depth_bias_enable(false)
                .build();

        let pipeline_color_blend_attachment_state = [PipelineColorBlendAttachmentState::builder()
            .color_write_mask(
                ColorComponentFlags::R
                    | ColorComponentFlags::G
                    | ColorComponentFlags::B
                    | ColorComponentFlags::A,
            )
            .blend_enable(true)
            .src_color_blend_factor(BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(BlendOp::ADD)
            .src_alpha_blend_factor(BlendFactor::ONE)
            .dst_alpha_blend_factor(BlendFactor::ZERO)
            .alpha_blend_op(BlendOp::ADD)
            .build()];

        let pipeline_color_blend_state_create_info = PipelineColorBlendStateCreateInfo::builder()
            .logic_op_enable(false)
            .logic_op(LogicOp::COPY)
            .attachments(&pipeline_color_blend_attachment_state)
            .blend_constants([0., 0., 0., 0.])
            .build();

        let pipeline_layout_create_info =
            PipelineLayoutCreateInfo::builder().set_layouts(&[]).push_constant_ranges(&[]).build();

        let pipeline_layout =
            unsafe { device.create_pipeline_layout(&pipeline_layout_create_info, None) }?;

        println!("Creating render pass");

        let color_attachment_descriptions = [AttachmentDescription::builder()
            .format(swapchain_create_info.image_format)
            .samples(SampleCountFlags::TYPE_1)
            .load_op(AttachmentLoadOp::CLEAR)
            .store_op(AttachmentStoreOp::STORE)
            .stencil_load_op(AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(AttachmentStoreOp::DONT_CARE)
            .initial_layout(ImageLayout::UNDEFINED)
            .final_layout(ImageLayout::PRESENT_SRC_KHR)
            .build()];

        let color_attachment_references = [AttachmentReference::builder()
            .attachment(0)
            .layout(ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .build()];

        let subpass_descriptions = [SubpassDescription::builder()
            .pipeline_bind_point(PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_attachment_references)
            .build()];

        let subpass_dependencies = [SubpassDependency::builder()
            .src_subpass(SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(AccessFlags::default())
            .dst_stage_mask(PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(AccessFlags::default())
            .build()];

        let render_pass_create_info = RenderPassCreateInfo::builder()
            .attachments(&color_attachment_descriptions)
            .subpasses(&subpass_descriptions)
            .dependencies(&subpass_dependencies)
            .build();

        let render_pass = unsafe { device.create_render_pass(&render_pass_create_info, None) }?;

        println!("Create graphics pipeline");

        let stages =
            [pipeline_shader_stage_create_info_vert, pipeline_shader_stage_create_info_frag];
        let pipeline_multisample_state_create_info = PipelineMultisampleStateCreateInfo::builder()
            .rasterization_samples(SampleCountFlags::TYPE_1)
            .build();

        let graphics_pipeline_create_infos = [GraphicsPipelineCreateInfo::builder()
            .stages(&stages)
            .dynamic_state(&pipeline_dynamic_state_create_info)
            .vertex_input_state(&pipeline_vertex_input_state_create_info)
            .viewport_state(&pipeline_viewport_state_create_info)
            .render_pass(render_pass)
            .layout(pipeline_layout)
            .input_assembly_state(&pipeline_input_assembly_state_create_info)
            .multisample_state(&pipeline_multisample_state_create_info)
            .color_blend_state(&pipeline_color_blend_state_create_info)
            .rasterization_state(&pipeline_rasterization_state_create_info)
            .build()];

        let pipelines = unsafe {
            device
                .create_graphics_pipelines(
                    PipelineCache::null(),
                    &graphics_pipeline_create_infos,
                    None,
                )
                .map_err(|(pipelines, e)| {
                    for pipeline in pipelines {
                        device.destroy_pipeline(pipeline, None);
                    }
                    e
                })
        }?;

        println!("Create framebuffers");

        let framebuffers = swapchain_image_views
            .iter()
            .map(|&swapchain_image_view| {
                let attachments = [swapchain_image_view];

                let framebuffer_create_info = FramebufferCreateInfo::builder()
                    .render_pass(render_pass)
                    .attachments(&attachments)
                    .width(swapchain_extent.width)
                    .height(swapchain_extent.height)
                    .layers(1)
                    .build();

                unsafe {
                    device.create_framebuffer(&framebuffer_create_info, None).map_err(|e| e.into())
                }
            })
            .collect::<Result<Vec<_>>>()?;

        println!("Create command pool");

        let command_pool_create_info = CommandPoolCreateInfo::builder()
            .flags(CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
            .queue_family_index(queue_families.graphics_queue)
            .build();

        let command_pool = unsafe { device.create_command_pool(&command_pool_create_info, None) }?;

        println!("Create command buffer");

        let command_buffer_allocate_info = CommandBufferAllocateInfo::builder()
            .command_pool(command_pool)
            .level(CommandBufferLevel::PRIMARY)
            .command_buffer_count(1)
            .build();

        let command_buffers =
            unsafe { device.allocate_command_buffers(&command_buffer_allocate_info) }?;

        let semaphore_image_available =
            unsafe { device.create_semaphore(&Default::default(), None)? };

        let semaphore_render_finished =
            unsafe { device.create_semaphore(&Default::default(), None)? };

        let fence_create_info =
            FenceCreateInfo::builder().flags(FenceCreateFlags::SIGNALED).build();
        let fence_in_flight = unsafe { device.create_fence(&fence_create_info, None)? };

        Ok(Self {
            instance,
            device,

            width,
            height,

            swapchain,
            swapchain_khr,
            swapchain_images,
            swapchain_image_views,
            swapchain_extent,

            surface,
            surface_khr,

            graphics_queue,
            present_queue,

            vertex_shader,
            fragment_shader,

            pipeline_layout,
            render_pass,
            pipelines,

            framebuffers,

            command_pool,
            command_buffers,
            semaphore_image_available,
            semaphore_render_finished,
            fence_in_flight,
        })
    }

    fn record_command_buffer(&self, command_buffer: CommandBuffer, index: u32) -> Result<()> {
        let command_buffer_begin_info = CommandBufferBeginInfo::builder().build();

        unsafe { self.device.begin_command_buffer(command_buffer, &command_buffer_begin_info)? };

        let clear_values = [ClearValue { color: ClearColorValue { float32: [0., 0., 0., 0.] } }];

        let render_pass_begin_info = RenderPassBeginInfo::builder()
            .render_pass(self.render_pass)
            .framebuffer(self.framebuffers[index as usize])
            .render_area(
                Rect2D::builder()
                    .offset(Offset2D { x: 0, y: 0 })
                    .extent(self.swapchain_extent)
                    .build(),
            )
            .clear_values(&clear_values)
            .build();

        unsafe {
            self.device.cmd_begin_render_pass(
                command_buffer,
                &render_pass_begin_info,
                SubpassContents::INLINE,
            );
            self.device.cmd_bind_pipeline(
                command_buffer,
                PipelineBindPoint::GRAPHICS,
                self.pipelines[0],
            );

            let viewports = [Viewport::builder()
                .x(0.)
                .y(0.)
                .width(self.swapchain_extent.width as _)
                .height(self.swapchain_extent.height as _)
                .min_depth(0.)
                .max_depth(1.)
                .build()];
            let scissors = [Rect2D::builder()
                .offset(Default::default())
                .extent(self.swapchain_extent)
                .build()];

            self.device.cmd_set_viewport(command_buffer, 0, &viewports);
            self.device.cmd_set_scissor(command_buffer, 0, &scissors);
            self.device.cmd_draw(command_buffer, 3, 1, 0, 0);
            self.device.cmd_end_render_pass(command_buffer);
            self.device.end_command_buffer(command_buffer)?;
        }

        Ok(())
    }

    pub fn draw(&self) -> Result<()> {
        unsafe {
            let fences = [self.fence_in_flight];
            self.device.wait_for_fences(&fences, true, u64::MAX)?
        };
        unsafe { self.device.reset_fences(&[self.fence_in_flight])? };
        let (image_index, _swapchain_is_suboptimal) = unsafe {
            self.swapchain.acquire_next_image(
                self.swapchain_khr,
                u64::MAX,
                self.semaphore_image_available,
                Fence::null(),
            )?
        };
        unsafe { self.device.reset_command_buffer(self.command_buffers[0], Default::default())? };
        // XXX: Why is this not unsafe?
        self.record_command_buffer(self.command_buffers[0], image_index)?;

        let wait_semaphores = [self.semaphore_image_available];
        let signal_semaphores = [self.semaphore_render_finished];
        let pipeline_stage_flags = [PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let submit_info = [SubmitInfo::builder()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&pipeline_stage_flags)
            .command_buffers(&self.command_buffers)
            .signal_semaphores(&signal_semaphores)
            .build()];
        unsafe {
            self.device.queue_submit(self.graphics_queue, &submit_info, self.fence_in_flight)?
        };

        let swapchains = [self.swapchain_khr];
        let image_indices = [image_index];

        let present_info = PresentInfoKHR::builder()
            .wait_semaphores(&signal_semaphores)
            .swapchains(&swapchains)
            .image_indices(&image_indices)
            .build();
        unsafe { self.swapchain.queue_present(self.present_queue, &present_info)? };

        Ok(())
    }
}

impl Drop for Vulkan {
    fn drop(&mut self) {
        unsafe {
            self.device.device_wait_idle().ok();
            self.device.destroy_semaphore(self.semaphore_image_available, None);
            self.device.destroy_semaphore(self.semaphore_render_finished, None);
            self.device.destroy_fence(self.fence_in_flight, None);
            self.device.free_command_buffers(self.command_pool, &self.command_buffers);
            self.device.destroy_command_pool(self.command_pool, None);
            self.framebuffers.drain(..).for_each(|framebuffer| {
                self.device.destroy_framebuffer(framebuffer, None);
            });
            self.device.destroy_pipeline_layout(self.pipeline_layout, None);
            self.pipelines
                .drain(..)
                .for_each(|pipeline| self.device.destroy_pipeline(pipeline, None));
            self.device.destroy_render_pass(self.render_pass, None);
            self.device.destroy_shader_module(self.vertex_shader, None);
            self.device.destroy_shader_module(self.fragment_shader, None);
            self.swapchain_image_views.drain(..).for_each(|swapchain_image_view| {
                self.device.destroy_image_view(swapchain_image_view, None);
            });
            self.swapchain.destroy_swapchain(self.swapchain_khr, None);
            self.surface.destroy_surface(self.surface_khr, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None)
        };
    }
}

struct QueueFamilies {
    graphics_queue: u32,
    present_queue: u32,
}

impl QueueFamilies {
    fn new(
        instance: &Instance,
        device: PhysicalDevice,
        surface: &Surface,
        surface_khr: SurfaceKHR,
    ) -> Option<Self> {
        let mut graphics_queue = None;
        let mut present_queue = None;

        for (queue_family_index, info) in
            unsafe { instance.get_physical_device_queue_family_properties(device) }
                .iter()
                .enumerate()
                .map(|(queue_family_index, info)| (queue_family_index as u32, info))
        {
            if info.queue_flags.contains(QueueFlags::GRAPHICS) {
                graphics_queue = Some(queue_family_index);
            }

            if unsafe {
                surface
                    .get_physical_device_surface_support(device, queue_family_index, surface_khr)
                    .unwrap_or(false)
            } {
                present_queue = Some(queue_family_index)
            }

            if present_queue.is_some() && graphics_queue.is_some() {
                break;
            }
        }

        Some(Self { present_queue: present_queue?, graphics_queue: graphics_queue? })
    }
}

struct SwapchainSupport {
    capabilities: SurfaceCapabilitiesKHR,
    formats: Vec<SurfaceFormatKHR>,
    present_modes: Vec<PresentModeKHR>,
    surface_khr: SurfaceKHR,
}

impl SwapchainSupport {
    fn new(device: PhysicalDevice, surface: &Surface, surface_khr: SurfaceKHR) -> Option<Self> {
        let capabilities =
            unsafe { surface.get_physical_device_surface_capabilities(device, surface_khr).ok()? };
        let formats =
            unsafe { surface.get_physical_device_surface_formats(device, surface_khr).ok()? };
        let present_modes =
            unsafe { surface.get_physical_device_surface_present_modes(device, surface_khr).ok()? };

        if !formats.is_empty() && !present_modes.is_empty() {
            Some(Self { capabilities, formats, present_modes, surface_khr })
        } else {
            None
        }
    }

    fn choose_format(&self) -> SurfaceFormatKHR {
        self.formats
            .iter()
            .copied()
            .find(|format| {
                println!("Format: {format:?}");
                format.format == Format::B8G8R8A8_UNORM
                    && format.color_space == ColorSpaceKHR::SRGB_NONLINEAR
            })
            .unwrap_or(self.formats.first().copied().unwrap())
    }

    fn choose_present_mode(&self) -> PresentModeKHR {
        self.present_modes
            .iter()
            .copied()
            .find(|&present_mode| present_mode == PresentModeKHR::MAILBOX)
            .unwrap_or(PresentModeKHR::FIFO)
    }

    fn choose_swap_extent(&self, width: u32, height: u32) -> Extent2D {
        if self.capabilities.current_extent.width == u32::MAX {
            Extent2D {
                width: u32::clamp(
                    width,
                    self.capabilities.min_image_extent.width,
                    self.capabilities.max_image_extent.width,
                ),
                height: u32::clamp(
                    height,
                    self.capabilities.min_image_extent.height,
                    self.capabilities.max_image_extent.height,
                ),
            }
        } else {
            self.capabilities.current_extent
        }
    }

    fn choose_image_count(&self) -> u32 {
        u32::clamp(
            self.capabilities.min_image_count + 1,
            self.capabilities.min_image_count,
            self.capabilities.max_image_count,
        )
    }

    fn create_info(
        &self,
        queue_families: &QueueFamilies,
        width: i32,
        height: i32,
    ) -> SwapchainCreateInfoKHR {
        // Look into VK_IMAGE_USAGE_TRANSFER_DST_BIT for compositing

        let is_same_queue = queue_families.present_queue == queue_families.graphics_queue;

        let format = self.choose_format();
        SwapchainCreateInfoKHR::builder()
            .min_image_count(self.choose_image_count())
            .present_mode(self.choose_present_mode())
            .image_extent(self.choose_swap_extent(width as _, height as _))
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_array_layers(1)
            .image_usage(ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(if !is_same_queue {
                SharingMode::CONCURRENT
            } else {
                SharingMode::EXCLUSIVE
            })
            .queue_family_indices(&[queue_families.present_queue, queue_families.graphics_queue])
            .pre_transform(self.capabilities.current_transform)
            .composite_alpha(CompositeAlphaFlagsKHR::OPAQUE)
            .surface(self.surface_khr)
            .clipped(true)
            .build()
    }
}

fn is_device_suitable(
    instance: &Instance,
    device: PhysicalDevice,
    surface: &Surface,
    surface_khr: SurfaceKHR,
) -> Option<(PhysicalDevice, QueueFamilies, SwapchainSupport)> {
    let properties = unsafe { instance.get_physical_device_properties(device) };
    // let features = unsafe { instance.get_physical_device_features(device) };

    if properties.device_type != PhysicalDeviceType::DISCRETE_GPU
        && properties.device_type != PhysicalDeviceType::INTEGRATED_GPU
    {
        eprintln!("{device:?} unsuitable: type {:?}", properties.device_type);
        return None;
    }

    if !check_device_extension_support(instance, device).unwrap_or(false) {
        return None;
    }

    let queue_families = QueueFamilies::new(instance, device, surface, surface_khr)?;
    let swapchain_support = SwapchainSupport::new(device, surface, surface_khr)?;

    Some((device, queue_families, swapchain_support))
}

fn check_device_extension_support(instance: &Instance, device: PhysicalDevice) -> Result<bool> {
    Ok(unsafe { instance.enumerate_device_extension_properties(device)? }.into_iter().any(
        |extension| unsafe {
            let ext_name = CStr::from_ptr(extension.extension_name.as_ptr());
            ext_name == Swapchain::name()
        },
    ))
}
