// A good lot of this code is taken from glium/examples/image.rs
// For now, we only want a window capable of receiving keyboard inputs as a basis for future work
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};

use glium::{glutin, Display, Surface};
use glium::glutin::event::{Event, StartCause, WindowEvent};
use glium::glutin::event_loop::{ControlFlow, EventLoop};
use glium::index::PrimitiveType;

use mio_extras::channel::Sender;

use crate::atlas::RectSize;
use crate::config::*;
use crate::draw::*;
use crate::rasterizer::*;
use crate::term::*;
use crate::window_event::*;

pub const DEFAULT_BG: (f32, f32, f32, f32) = (0.0, 0.0, 0.0, 0.5);

pub fn window(config: Config) {    
    let events_loop = EventLoop::new();
    let window_builder = glutin::window::WindowBuilder::new()
        .with_inner_size(glutin::dpi::LogicalSize::new(1280.0, 720.0))
        .with_title("mou ikkai")
        .with_transparent(config.transparent);
    let context_builder = glutin::ContextBuilder::new();
    
    let display = glium::Display::new(window_builder, context_builder, &events_loop).unwrap();
    
    let vertex_buffer = {
        #[derive(Copy, Clone)]
        struct Vertex {
            position: [f32; 2],
            tex_coords: [f32; 2],
        }

        implement_vertex!(Vertex, position, tex_coords);

        glium::VertexBuffer::new(
            &display,
            &[
                Vertex {
                    position: [-1.0, -1.0],
                    tex_coords: [-1.0, 0.0],
                },
                Vertex {
                    position: [-1.0, 1.0],
                    tex_coords: [-1.0, -1.0],
                },
                Vertex {
                    position: [1.0, 1.0],
                    tex_coords: [0.0, -1.0],
                },
                Vertex {
                    position: [1.0, -1.0],
                    tex_coords: [0.0, 0.0],
                },
            ],
        )
        .unwrap()
    };

    let index_buffer =
        glium::IndexBuffer::new(&display, PrimitiveType::TriangleStrip, &[1 as u16, 2, 0, 3])
            .unwrap();

    let program = program!(&display,
    140 => {
        vertex: "
                       #version 140
                       uniform mat4 matrix;
                       in vec2 position;
                       in vec2 tex_coords;
                       out vec2 v_tex_coords;
                       void main() {
                           gl_Position = matrix * vec4(position, 0.0, 1.0);
                           v_tex_coords = tex_coords;
                       }
                   ",

        fragment: &format!("
                       #version 140
                       uniform sampler2D tex;
                       in vec2 v_tex_coords;
                       out vec4 f_color;
                       void main() {{
                           f_color = vec4(texture(tex, v_tex_coords).xyz, {});
                       }}
                   ", DEFAULT_BG.3),
        outputs_srgb: true
    })
    .unwrap();

    let mut display_background = false;

    let mut drawer = Drawer::new(&display, config.clone());
    let rasterizer = Arc::new(RwLock::new(Rasterizer::new(config.clone(), get_display_size(&display))));
    let cell_size = rasterizer.read().unwrap().cell_size;
    let delta_cell_height = rasterizer.read().unwrap().delta_cell_height;

    let mut manager = TermManager::new(config.clone(), rasterizer.clone());
    let mut dimensions = get_display_size(&display); 
    let mut lines = manager.get_lines_from_active_force(0, 20);
    let mut first_draw = true;
    
    
    let mut old = SystemTime::now();
    let mut t: u128 = 0;
    
    let mut old_display_cursor = false;
    
    let mut display_cursor_t_base = 0u128;
    
    let mut frame: Vec<u8> = vec![0; (dimensions.width * dimensions.height) as usize];
    
    let rasterizer = rasterizer.clone();
    start_loop(events_loop, move |events| {
        if manager.cleanup_exited_terminals() {
            return Action::Stop;
        }
        
        t += get_time_diff(&mut old);
        let mut need_refresh = false;
        
        // when something have been refreshed on the screen, that means we need to update the base
        // t for the cursor, and this way the cursor stay lit when inputting data
        if manager.is_active_updated() {
            display_cursor_t_base = t;
        }
        
        let display_cursor = new_cursor_state(t - display_cursor_t_base);
        need_refresh = need_refresh || (old_display_cursor ^ display_cursor);
        old_display_cursor = display_cursor;
        
        if first_draw {
            first_draw = false;
            need_refresh = true;
        }
        
        if check_updated_display_size(&display, dimensions) {
            need_refresh = true;
            dimensions = get_display_size(&display);
            drawer.update_dimensions(&display);
            rasterizer.write().unwrap().update_dimensions(&display);
            manager.dimensions_updated();
        }
        
        let maybe_new = manager.get_lines_from_active(0, 40);
        if let Some(new_lines) = maybe_new {
            lines = new_lines;
            need_refresh = true;
        }
        
        if let Some(new_frame) = manager.get_youtube_frame_from_active() {
            frame = new_frame;
            need_refresh = true;
            display_background = true;
        } else {
            display_background = false;
        }
        
        if need_refresh {            
            // drawing a frame
            let mut target = display.draw();
            
            target.clear_color(DEFAULT_BG.0, DEFAULT_BG.1, DEFAULT_BG.2, DEFAULT_BG.3);
            
            if display_background {
                let glium_image =
                    glium::texture::RawImage2d::from_raw_rgb(frame.clone(), (dimensions.width, dimensions.height));

                let opengl_texture =
                    glium::texture::Texture2d::new(&display, glium_image).unwrap();
                
                
                let uniforms = uniform! {
                    matrix: [
                        [1.0, 0.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                        [0.0, 0.0, 0.0, 1.0f32]
                    ],
                    tex: &opengl_texture
                };
                
                target
                .draw(
                    &vertex_buffer,
                    &index_buffer,
                    &program,
                    &uniforms,
                    &Default::default(),
                )
                .unwrap();
            }
            
            drawer.render_lines(&lines, display_cursor, cell_size, delta_cell_height, &display, &mut target);
            
            target.finish().unwrap();
        }
        

        let mut action = Action::Continue;        
        for event in events {
            if let Event::WindowEvent { event, .. } = event {
                match event {
                    WindowEvent::CloseRequested => action = Action::Stop,
                    WindowEvent::ReceivedCharacter(input) => {
                        manager.send_event(TermikuWindowEvent::CharacterInput(*input))
                    }
                    WindowEvent::KeyboardInput { input, .. } => {
                        // println!("{:?}", input);
                        if let Some(event) = handle_keyboard_input(input) {
                            manager.send_event(event);
                        }
                    }
                    _ => (),
                };
            }
        }

        action
    });
}

pub enum Action {
    Stop,
    Continue,
}

fn start_loop<F>(event_loop: EventLoop<()>, mut callback: F) -> !
where
    F: 'static + FnMut(&Vec<Event<()>>) -> Action,
{
    let mut events_buffer = Vec::new();
    let mut next_frame_time = Instant::now();
    event_loop.run(move |event, _, control_flow| {
        let run_callback = match event {
            Event::NewEvents(cause) => match cause {
                StartCause::ResumeTimeReached { .. } | StartCause::Init => true,
                _ => false,
            },
            _ => {
                events_buffer.push(event);
                false
            }
        };

        let action = if run_callback {
            let action = callback(&events_buffer);
            next_frame_time = Instant::now() + Duration::from_nanos(16_666_667);

            events_buffer.clear();
            action
        } else {
            Action::Continue
        };

        match action {
            Action::Continue => {
                *control_flow = ControlFlow::WaitUntil(next_frame_time);
            }
            Action::Stop => *control_flow = ControlFlow::Exit,
        }
    })
}

fn send_char_to_process(process: &Sender<char>, character: char) {
    process.send(character).unwrap();
}

fn get_display_size(display: &Display) -> RectSize {
    let (width, height) = display.get_framebuffer_dimensions();
    
    RectSize {
        width,
        height,
    }
}

fn check_updated_display_size(display: &Display, old: RectSize) -> bool {
    let (width, height) = display.get_framebuffer_dimensions();
    old.width != width || old.height != height
}

fn get_time_diff(old: &mut SystemTime) -> u128 {
    let now = SystemTime::now();
    let diff = match now.duration_since(old.clone()) {
        Ok(duration) => duration.as_millis(),
        Err(_) => 0
    };
    std::mem::replace(old, now);
    diff
}

fn new_cursor_state(t: u128) -> bool {
    (t % 1000) <= 500
}
