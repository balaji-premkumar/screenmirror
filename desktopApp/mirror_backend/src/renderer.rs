use crate::receiver::log_event;
use concurrent_queue::ConcurrentQueue;
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use sdl2::pixels::PixelFormatEnum;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;

pub struct PreviewFrame {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub format: u32, // 1 = NV12, 2 = I420
}

pub static PREVIEW_ACTIVE: AtomicBool = AtomicBool::new(false);
pub static PREVIEW_QUEUE: Lazy<Arc<ConcurrentQueue<PreviewFrame>>> =
    Lazy::new(|| Arc::new(ConcurrentQueue::bounded(2))); // Small buffer for lowest latency

pub static FREE_QUEUE: Lazy<Arc<ConcurrentQueue<Vec<u8>>>> =
    Lazy::new(|| {
        let q = Arc::new(ConcurrentQueue::bounded(3));
        for _ in 0..3 {
            let _ = q.push(Vec::with_capacity(8 * 1024 * 1024));
        }
        q
    });

pub fn start_native_preview(_project_root: &str) {
    if PREVIEW_ACTIVE.swap(true, Ordering::Relaxed) {
        return; // Already active
    }

    log_event(
        "INFO",
        "PREVIEW",
        "sdl2",
        "Launching native Rust preview thread with SDL2...",
    );

    let queue = PREVIEW_QUEUE.clone();

    std::thread::spawn(move || {
        let sdl_context = match sdl2::init() {
            Ok(ctx) => ctx,
            Err(e) => {
                log_event("ERROR", "PREVIEW", "sdl2", &format!("SDL init failed: {}", e));
                PREVIEW_ACTIVE.store(false, Ordering::Relaxed);
                return;
            }
        };

        let video_subsystem = match sdl_context.video() {
            Ok(vs) => vs,
            Err(e) => {
                log_event("ERROR", "PREVIEW", "sdl2", &format!("Video subsystem failed: {}", e));
                PREVIEW_ACTIVE.store(false, Ordering::Relaxed);
                return;
            }
        };

        let window = match video_subsystem.window("Mirror High-Speed Preview", 1280, 720)
            .position_centered()
            .resizable()
            .allow_highdpi()
            .build() {
                Ok(w) => w,
                Err(e) => {
                    log_event("ERROR", "PREVIEW", "sdl2", &format!("Window creation failed: {}", e));
                    PREVIEW_ACTIVE.store(false, Ordering::Relaxed);
                    return;
                }
            };

        let mut canvas = match window.into_canvas().accelerated().build() {
            Ok(c) => c,
            Err(e) => {
                log_event("ERROR", "PREVIEW", "sdl2", &format!("Canvas creation failed: {}", e));
                PREVIEW_ACTIVE.store(false, Ordering::Relaxed);
                return;
            }
        };

        let texture_creator = canvas.texture_creator();
        let mut event_pump = match sdl_context.event_pump() {
            Ok(ep) => ep,
            Err(e) => {
                log_event("ERROR", "PREVIEW", "sdl2", &format!("Event pump failed: {}", e));
                PREVIEW_ACTIVE.store(false, Ordering::Relaxed);
                return;
            }
        };

        log_event("SUCCESS", "PREVIEW", "sdl2", "SDL2 Native preview window created");

        let mut texture = None;
        let mut tex_width = 0;
        let mut tex_height = 0;
        let mut tex_format = 0;
        let mut is_fullscreen = false;

        'running: loop {
            if crate::TERMINATION_SIGNAL.load(Ordering::Relaxed) {
                log_event("INFO", "PREVIEW", "sdl2", "Preview thread receiving termination signal.");
                break 'running;
            }

            for event in event_pump.poll_iter() {
                match event {
                    Event::Quit { .. }
                    | Event::KeyDown { keycode: Some(Keycode::Escape), .. } => {
                        break 'running;
                    },
                    Event::KeyDown { keycode: Some(Keycode::F), .. } => {
                        is_fullscreen = !is_fullscreen;
                        let state = if is_fullscreen { sdl2::video::FullscreenType::Desktop } else { sdl2::video::FullscreenType::Off };
                        let _ = canvas.window_mut().set_fullscreen(state);
                    },
                    Event::KeyDown { keycode: Some(Keycode::M), .. } => {
                        let current = crate::audio::AUDIO_MUTED.load(Ordering::Relaxed);
                        crate::audio::AUDIO_MUTED.store(!current, Ordering::Relaxed);
                        log_event("INFO", "PREVIEW", "audio", if !current { "Audio Muted" } else { "Audio Unmuted" });
                    },
                    _ => {}
                }
            }

            let mut latest_frame: Option<PreviewFrame> = None;
            while let Ok(frame) = queue.pop() {
                if let Some(mut old_frame) = latest_frame.take() {
                    old_frame.data.clear();
                    let _ = FREE_QUEUE.push(old_frame.data);
                }
                latest_frame = Some(frame);
            }

            if let Some(mut frame) = latest_frame {
                if texture.is_none() || tex_width != frame.width || tex_height != frame.height || tex_format != frame.format {
                    let pf = if frame.format == 1 { 
                        PixelFormatEnum::NV12 
                    } else if frame.format == 2 {
                        PixelFormatEnum::IYUV
                    } else {
                        PixelFormatEnum::ABGR8888 // SDL2 BGRA is actually ABGR in memory for some reason?
                    };
                    texture = texture_creator.create_texture_streaming(pf, frame.width as u32, frame.height as u32).ok();
                    tex_width = frame.width;
                    tex_height = frame.height;
                    tex_format = frame.format;
                }

                if let Some(tex) = texture.as_mut() {
                    if frame.format == 0 {
                        // BGRA
                        let _ = tex.update(None, &frame.data, frame.width * 4);
                    } else if frame.format == 1 {
                        // NV12
                        let y_len = frame.width * frame.height;
                        let uv_len = y_len / 2;
                        if frame.data.len() >= y_len + uv_len {
                            let _ = tex.update(None, &frame.data, frame.width);
                        }
                    } else {
                        // IYUV
                        let y_len = frame.width * frame.height;
                        let u_len = y_len / 4;
                        let v_len = y_len / 4;
                        if frame.data.len() >= y_len + u_len + v_len {
                            let y_plane = &frame.data[..y_len];
                            let u_plane = &frame.data[y_len..y_len + u_len];
                            let v_plane = &frame.data[y_len + u_len..];
                            let _ = tex.update_yuv(None, y_plane, frame.width, u_plane, frame.width / 2, v_plane, frame.width / 2);
                        }
                    }
                    
                    let _ = canvas.clear();
                    
                    let (win_w, win_h) = canvas.output_size().unwrap_or((1280, 720));
                    let src_ratio = frame.width as f32 / frame.height as f32;
                    let dst_ratio = win_w as f32 / win_h as f32;
                    
                    let mut dst_w = win_w;
                    let mut dst_h = win_h;
                    
                    if src_ratio > dst_ratio {
                        // Source is wider than destination (pillarbox top/bottom)
                        dst_h = (win_w as f32 / src_ratio) as u32;
                    } else {
                        // Source is taller than destination (pillarbox left/right)
                        dst_w = (win_h as f32 * src_ratio) as u32;
                    }
                    
                    let dst_rect = sdl2::rect::Rect::new(
                        ((win_w - dst_w) / 2) as i32,
                        ((win_h - dst_h) / 2) as i32,
                        dst_w,
                        dst_h,
                    );
                    
                    let _ = canvas.copy(tex, None, dst_rect);
                    canvas.present();
                }
                
                // Return buffer to pool
                frame.data.clear();
                let _ = FREE_QUEUE.push(frame.data);
            } else {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        PREVIEW_ACTIVE.store(false, Ordering::Relaxed);
        crate::audio::AUDIO_MUTED.store(true, Ordering::Relaxed); // mute again when closed
        log_event("INFO", "PREVIEW", "sdl2", "Preview window closed");
    });
}

pub fn update_preview_window(mut data: Vec<u8>, width: usize, height: usize, format: u32) {
    if !PREVIEW_ACTIVE.load(Ordering::Relaxed) {
        data.clear();
        let _ = FREE_QUEUE.push(data); // return it to pool immediately
        return;
    }

    // Drain queue if full to avoid lag (always keep the freshest frame)
    while PREVIEW_QUEUE.is_full() {
        if let Ok(mut old) = PREVIEW_QUEUE.pop() {
            old.data.clear();
            let _ = FREE_QUEUE.push(old.data);
        }
    }

    let _ = PREVIEW_QUEUE.push(PreviewFrame {
        data,
        width,
        height,
        format,
    });
}