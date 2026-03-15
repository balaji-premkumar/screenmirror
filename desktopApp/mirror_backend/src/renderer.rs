use winit::{
    event::*,
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use std::sync::Arc;

pub struct Renderer {
    _instance: wgpu::Instance,
    _surface: wgpu::Surface<'static>,
    _device: wgpu::Device,
    _queue: wgpu::Queue,
    _config: wgpu::SurfaceConfiguration,
    pub size: winit::dpi::PhysicalSize<u32>,
}

impl Renderer {
    async fn new(window: Arc<winit::window::Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        
        let surface = instance.create_surface(window.clone()).unwrap();
        let adapter = instance.request_adapter(
            &wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            },
        ).await.unwrap();

        let (device, queue) = adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        ).await.unwrap();

        let caps = surface.get_capabilities(&adapter);
        
        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate
        } else if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            caps.present_modes[0] // fallback to Fifo or whatever is available
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: caps.formats[0],
            width: size.width,
            height: size.height,
            present_mode,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 1,
        };
        surface.configure(&device, &config);

        Self {
            _instance: instance,
            _surface: surface,
            _device: device,
            _queue: queue,
            _config: config,
            size,
        }
    }
}

pub fn start_native_preview() {
    std::thread::spawn(|| {
        let mut builder = winit::event_loop::EventLoopBuilder::new();
        #[cfg(target_os = "linux")]
        {
            use winit::platform::x11::EventLoopBuilderExtX11;
            use winit::platform::wayland::EventLoopBuilderExtWayland;
            EventLoopBuilderExtWayland::with_any_thread(&mut builder, true);
            EventLoopBuilderExtX11::with_any_thread(&mut builder, true);
        }
        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::EventLoopBuilderExtWindows;
            builder.with_any_thread(true);
        }
        let event_loop = builder.build().unwrap();
        let window = Arc::new(WindowBuilder::new()
            .with_title("Mirror High-Speed Preview")
            .build(&event_loop)
            .unwrap());

        let _renderer = pollster::block_on(Renderer::new(window.clone()));

        // winit 0.29+ new API
        event_loop.run(move |event, target| {
            target.set_control_flow(ControlFlow::Wait);

            match event {
                Event::WindowEvent { ref event, window_id } if window_id == window.id() => match event {
                    WindowEvent::CloseRequested => target.exit(),
                    WindowEvent::Resized(physical_size) => {
                        // Handle resize logic if needed
                    },
                    WindowEvent::RedrawRequested => {
                        // Render frame here
                    }
                    _ => {}
                },
                _ => {}
            }
        }).unwrap();
    });
}
