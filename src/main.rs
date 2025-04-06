#![allow(clippy::needless_range_loop)]

use crate::{
    animation::{Animation, AnimationReq},
    cursor_renderer::CursorRenderer,
    glyph_cache::GlyphCache,
    glyph_renderer::GlyphRenderer,
    mat::Transform,
    mesh_renderer::MeshRenderer,
};

use glfw::{fail_on_errors, Context};
use glow::{HasContext, NativeTexture};

use chrono::NaiveTime;

use mat::Vec3;
use mesh_renderer::{GpuMesh, UploadMeshError};
use obj_parser::ObjParseError;
use screen_tex_postprocess::ScreenTexPostprocessor;
use thiserror::Error;

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

mod animation;
mod cursor_renderer;
mod ease;
mod gl_util;
mod glyph_cache;
mod glyph_renderer;
mod mat;
mod mesh_renderer;
mod obj_parser;
mod screen_tex_postprocess;

#[derive(Error, Debug)]
#[error("{0}")]
pub struct GlError(String);

const WINDOW_WIDTH: u32 = 1920 / 2;
const WINDOW_HEIGHT: u32 = 1080 / 2;
const WINDOW_ASPECT: f32 = WINDOW_WIDTH as f32 / WINDOW_HEIGHT as f32;

struct Args {
    start_time: NaiveTime,
    topic: String,
}

impl Args {
    fn parse<It: Iterator<Item = String>>(mut args: It) -> Args {
        let mut start_time = None;
        let mut topic = None;
        let process_name = args.next().unwrap_or_else(|| "prog".to_string());

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--start-time" => {
                    start_time = args.next().map(|v| v.parse());
                }
                "--topic" => {
                    topic = args.next();
                }
                _ => {
                    Self::help(&process_name);
                }
            }
        }

        let start_time = match start_time {
            Some(Ok(start_time)) => start_time,
            Some(Err(e)) => {
                println!("Failed to parse start time: {e}");
                Self::help(&process_name);
            }
            None => {
                println!("Start time not provided");
                Self::help(&process_name);
            }
        };

        let topic = match topic {
            Some(v) => v,
            None => {
                println!("Topic not provided");
                Self::help(&process_name);
            }
        };

        Args { start_time, topic }
    }

    fn help(process_name: &str) -> ! {
        println!(
            "\
                 A pre-stream screen...\n\
                 \n\
                 Usage:\n\
                 {process_name} [args]\n\
                 \n\
                 Arguments:\n\
                 --start-time: when stream starts\n\
                 --topic: what are we working on today\n\
                 "
        );
        std::process::exit(1);
    }
}

fn stream_starting_string(start_time: NaiveTime, now: NaiveTime, topic: &str) -> String {
    let remaining = start_time - now;
    let program = std::env::args().next().unwrap();
    format!(
        "\
        $ ./{}\n\
        \n\
        Today's topic: {}\n\
        Stream starting at {}\n\
            Current time: {}\n\
            {:02}:{:02}:{:02} 'till stream starts",
        program,
        topic,
        start_time.format("%H:%M:%S"),
        now.format("%H:%M:%S"),
        remaining.num_hours(),
        remaining.num_minutes() % 60,
        remaining.num_seconds() % 60,
    )
}

fn reset_animation(
    start_time: NaiveTime,
    topic: &str,
    current: String,
) -> (Animation, VecDeque<AnimationReq>) {
    let new_s = stream_starting_string(start_time, chrono::Local::now().time(), topic);
    let reqs = animation::construct_animation_requests(&current, &new_s);
    (Animation::None(current), reqs)
}

fn init_gl(window: &mut glfw::PWindow) -> glow::Context {
    unsafe {
        let gl = glow::Context::from_loader_function(|s| window.get_proc_address(s) as *const _);

        let r = 29.0f32 / 255.0f32;
        let g = 31.0f32 / 255.0f32;
        let b = 33.0f32 / 255.0f32;
        gl.clear_color(r, g, b, 1.0);

        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::LESS);
        gl.enable(glow::BLEND);

        gl
    }
}

struct App<'a> {
    args: &'a Args,
    gl: &'a glow::Context,
    glyph_renderer: GlyphRenderer<'a>,
    cursor_renderer: CursorRenderer<'a>,
    screen_tex_postprocessor: ScreenTexPostprocessor<'a>,
    mesh_renderer: &'a MeshRenderer<'a>,
    current_animation: Animation,
    animation_queue: VecDeque<AnimationReq>,
    cursor_visible: bool,
    cursor_flip_time: Instant,
    cursor_blink_duration: Duration,
    last_update: Instant,
    time: f32,
    light_dir: Vec3,
    view_matrix: Transform,
    monitor: GpuMesh<'a>,
    screen: GpuMesh<'a>,
    table: GpuMesh<'a>,
    walls: GpuMesh<'a>,
    window_width: i32,
    window_height: i32,
}

impl App<'_> {
    fn new<'a>(
        gl: &'a glow::Context,
        args: &'a Args,
        glyph_cache: &'a mut GlyphCache,
        mesh_renderer: &'a MeshRenderer<'a>,
    ) -> Result<App<'a>, MainError> {
        let glyph_renderer =
            GlyphRenderer::new(gl, glyph_cache).map_err(MainError::CreateGlyphRenderer)?;
        let cursor_renderer = CursorRenderer::new(gl).map_err(MainError::CreateCursorRenderer)?;

        let (current_animation, animation_queue) =
            reset_animation(args.start_time, &args.topic, "".to_string());
        let cursor_visible = false;

        let cursor_blink_duration: Duration = Duration::from_secs_f32(0.5);
        let cursor_flip_time = Instant::now() + cursor_blink_duration;

        let screen_tex_postprocessor = ScreenTexPostprocessor::new(gl)
            .map_err(MainError::CreateScreenTexPostProcessorError)?;
        let monitor = obj_parser::Mesh::from_obj_file(include_bytes!("../monitor.obj").as_slice())
            .map_err(MainError::LoadMonitor)?;
        let screen = obj_parser::Mesh::from_obj_file(include_bytes!("../screen.obj").as_slice())
            .map_err(MainError::LoadScreen)?;
        let table = obj_parser::Mesh::from_obj_file(include_bytes!("../table.obj").as_slice())
            .map_err(MainError::LoadTable)?;
        let walls = obj_parser::Mesh::from_obj_file(include_bytes!("../walls.obj").as_slice())
            .map_err(MainError::LoadWalls)?;

        let monitor_tex =
            load_texture_from_png(gl, include_bytes!("../monitor_texture.png").as_slice());
        let screen_tex =
            load_texture_from_png(gl, include_bytes!("../screen_textuire.png").as_slice());
        let table_tex =
            load_texture_from_png(gl, include_bytes!("../table_texture.png").as_slice());
        let walls_tex = load_texture_from_png(gl, include_bytes!("../wall_texture.png").as_slice());

        let monitor = mesh_renderer
            .upload_mesh(&monitor, monitor_tex)
            .map_err(MainError::UploadMonitor)?;
        let screen = mesh_renderer
            .upload_mesh(&screen, screen_tex)
            .map_err(MainError::UploadScreen)?;
        let table = mesh_renderer
            .upload_mesh(&table, table_tex)
            .map_err(MainError::UploadTable)?;
        let walls = mesh_renderer
            .upload_mesh(&walls, walls_tex)
            .map_err(MainError::UploadWalls)?;

        Ok(App {
            args,
            gl,
            glyph_renderer,
            cursor_renderer,
            mesh_renderer,
            screen_tex_postprocessor,
            current_animation,
            animation_queue,
            cursor_visible,
            cursor_flip_time,
            cursor_blink_duration,
            time: 0.0,
            last_update: Instant::now(),
            light_dir: [0.0f32, 0.0f32, 0.0f32].into(),
            view_matrix: Transform::identity(),
            monitor,
            screen,
            table,
            walls,
            window_width: WINDOW_WIDTH as i32,
            window_height: WINDOW_HEIGHT as i32,
        })
    }

    fn light_transform(&self) -> Transform {
        Transform::scale(0.75, 0.5, 1.0 / 10.0)
            * Transform::look_at(
                [0.0, 0.0, 0.0].into(),
                self.light_dir,
                [0.0, 1.0, 0.0].into(),
            )
            .inverted()
    }

    fn view_pos_to_light_pos(&self) -> Transform {
        self.light_transform() * self.view_matrix.inverted()
    }

    fn update(&mut self, now: Instant) {
        let time_since_last = (now - self.last_update).as_secs_f32();
        if self.current_animation.finished(now) {
            let animation =
                std::mem::replace(&mut self.current_animation, Animation::None("".to_string()));
            let s = animation.into_finished_string();

            self.current_animation = match self.animation_queue.pop_front() {
                Some(req) => animation::apply_animation_req(req, s, now),
                None => {
                    (self.current_animation, self.animation_queue) =
                        reset_animation(self.args.start_time, &self.args.topic, s);
                    return;
                }
            }
        }

        self.current_animation.update(now);

        self.time += time_since_last;
        let z_offs = f32::sin(self.time / 8.0) * 0.05;
        let y_offs = f32::cos(self.time / 8.0) * 0.05;
        let aspect = self.window_width as f32 / self.window_height as f32;
        self.view_matrix = Transform::scale(1.0 / aspect, 1.0, 1.0)
            * Transform::perspective(50.0f32.to_radians(), 0.1, 10.0)
            * Transform::look_at(
                [0.6, 0.20 + y_offs, -0.05 + z_offs].into(),
                [0.16, 0.045, 0.0].into(),
                [0.0, 1.0, 0.0].into(),
            )
            .inverted();
        self.light_dir = [-0.3, -1.0, -0.6].into();
        self.mesh_renderer.set_light_color(&[0.8, 0.8, 0.5]);
        self.last_update = now;
    }

    fn render_objects(&self) {
        let monitor_transform = Transform::from_translation(0.0, 0.04, 0.0)
            * Transform::scale(1.5, 1.5 * 4.0 / 5.0, 1.5);
        self.mesh_renderer
            .render(&self.table, &Transform::identity());
        self.mesh_renderer.render(&self.monitor, &monitor_transform);
        self.mesh_renderer.render(&self.screen, &monitor_transform);
        self.mesh_renderer
            .render(&self.walls, &Transform::scale(1.0, 1.0, -1.0));
    }

    fn render_light_depth(&self) -> NativeTexture {
        unsafe {
            let (tex, fb) = gl_util::setup_depth_texture_render(self.gl, 4096, 4096).unwrap();

            self.gl.clear(glow::DEPTH_BUFFER_BIT);
            self.mesh_renderer
                .set_camera_transform(&self.light_transform());
            self.render_objects();

            self.gl.delete_framebuffer(fb);
            tex
        }
    }

    fn render_text_to_texture(&mut self, now: Instant) -> NativeTexture {
        unsafe {
            let (tex, fb) = gl_util::setup_color_texture_render(self.gl, 1024, 1024).unwrap();
            self.gl.clear_color(0.0, 0.0, 0.0, 1.0);
            self.gl
                .clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);

            let s = self.current_animation.as_str();

            let mut cursor_pos_x = 0.05;
            let mut cursor_pos_y = 0.7;
            let cursor_update =
                self.glyph_renderer
                    .render_str(s, cursor_pos_x, cursor_pos_y, WINDOW_ASPECT);

            cursor_pos_x += cursor_update.0;
            cursor_pos_y += cursor_update.1;

            if self.cursor_flip_time < now {
                self.cursor_flip_time += self.cursor_blink_duration;
                self.cursor_visible = !self.cursor_visible;
            }

            if self.cursor_visible {
                let cursor_height = self.glyph_renderer.line_height() * 0.6;
                let cursor_width = cursor_height / 2.0;
                self.cursor_renderer.render(
                    cursor_pos_x,
                    cursor_pos_y,
                    cursor_width,
                    cursor_height,
                    WINDOW_ASPECT,
                );
            }

            let (tex2, fb2) = gl_util::setup_color_texture_render(self.gl, 1024, 1024).unwrap();
            self.screen_tex_postprocessor
                .render(tex, self.time, WINDOW_ASPECT);

            self.gl.delete_framebuffer(fb);
            self.gl.delete_framebuffer(fb2);
            self.gl.delete_texture(tex);
            tex2
        }
    }

    fn render(&mut self, now: Instant) {
        unsafe {
            let tex = self.render_light_depth();
            let screen_tex = self.render_text_to_texture(now);

            self.gl
                .clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            self.gl
                .viewport(0, 0, self.window_width, self.window_height);

            self.mesh_renderer.set_camera_transform(&self.view_matrix);
            self.mesh_renderer
                .set_view_to_light_transform(&self.view_pos_to_light_pos());
            self.mesh_renderer.set_light_dir(&self.light_dir);
            self.mesh_renderer.set_light_texture(tex);
            self.screen.tex = screen_tex;
            self.render_objects();

            self.gl.delete_texture(tex);
            self.gl.delete_texture(screen_tex);
        }
    }

    fn set_window_size(&mut self, width: i32, height: i32) {
        self.window_width = width;
        self.window_height = height;
    }
}

#[derive(Error, Debug)]
enum MainError {
    #[error("failed to init glfw")]
    InitGlfw(#[from] glfw::InitError),
    #[error("failed to create glfw window")]
    CreateGlfwWindow,
    #[error("failed to create glyph cache")]
    CreateGlyphCache(#[from] glyph_cache::GlyphCacheCreationError),
    #[error("failed to create glyph renderer")]
    CreateGlyphRenderer(GlError),
    #[error("failed to create cursor renderer")]
    CreateCursorRenderer(GlError),
    #[error("failed to create screen text postprocessor")]
    CreateScreenTexPostProcessorError(GlError),
    #[error("failed to create mesh renderer")]
    CreateMeshRenderer(GlError),
    #[error("failed to load table obj")]
    LoadTable(ObjParseError),
    #[error("failed to load walls obj")]
    LoadWalls(ObjParseError),
    #[error("failed to load monitor obj")]
    LoadMonitor(ObjParseError),
    #[error("failed to load screen obj")]
    LoadScreen(ObjParseError),
    #[error("failed to upload table to gpu")]
    UploadTable(UploadMeshError),
    #[error("failed to upload walls to gpu")]
    UploadWalls(UploadMeshError),
    #[error("failed to upload monitor to gpu")]
    UploadMonitor(UploadMeshError),
    #[error("failed to upload screen to gpu")]
    UploadScreen(UploadMeshError),
    #[error("failed to get character")]
    GetCharacter(#[from] glyph_cache::GetCharacterError),
}

fn load_texture_from_png<R: std::io::Read>(gl: &glow::Context, f: R) -> NativeTexture {
    let mut png_reader = png::Decoder::new(f).read_info().unwrap();

    let mut img_data = vec![0; png_reader.output_buffer_size()];
    let img_info = png_reader.next_frame(&mut img_data).unwrap();

    unsafe {
        let tex = gl_util::create_tex_default_params(gl).unwrap();

        let color_format = match img_info.color_type {
            png::ColorType::Grayscale => glow::RED,
            png::ColorType::Rgb => glow::RGB,
            png::ColorType::Indexed => {
                panic!("Indexed colors unsupported");
            }
            png::ColorType::GrayscaleAlpha => {
                panic!("Grayscale alpha unsupported");
            }
            png::ColorType::Rgba => glow::RGBA,
        };

        let bit_depth = match img_info.bit_depth {
            png::BitDepth::One | png::BitDepth::Two | png::BitDepth::Four => {
                panic!("Unsupported bit depth: {:?}", img_info.bit_depth);
            }
            png::BitDepth::Eight => glow::UNSIGNED_BYTE,
            png::BitDepth::Sixteen => glow::UNSIGNED_SHORT,
        };

        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            img_info.width.try_into().unwrap(),
            img_info.height.try_into().unwrap(),
            0,
            color_format,
            bit_depth,
            Some(&img_data),
        );
        gl.bind_texture(glow::TEXTURE_2D, None);

        tex
    }
}

fn main() -> Result<(), MainError> {
    let args = Args::parse(std::env::args());

    let mut glfw = glfw::init(fail_on_errors!())?;

    // Request specific OpenGL version and core profile
    glfw.window_hint(glfw::WindowHint::ContextVersionMajor(4));
    glfw.window_hint(glfw::WindowHint::ContextVersionMinor(1));
    glfw.window_hint(glfw::WindowHint::OpenGlProfile(
        glfw::OpenGlProfileHint::Core,
    ));
    glfw.window_hint(glfw::WindowHint::OpenGlForwardCompat(true)); // Required on macOS

    let (mut window, events) = glfw
        .create_window(
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            "Stream starting...",
            glfw::WindowMode::Windowed,
        )
        .ok_or(MainError::CreateGlfwWindow)?;

    // Set the aspect ratio to 16:9
    window.set_aspect_ratio(16, 9);

    window.make_current();
    window.set_key_polling(true);
    window.set_framebuffer_size_polling(true);

    // Get the initial window size
    let (width, height) = window.get_framebuffer_size();

    const PIXEL_SIZE: u32 = 256;
    let mut glyph_cache = GlyphCache::new(PIXEL_SIZE)?;
    let gl = init_gl(&mut window);

    let mesh_renderer = MeshRenderer::new(&gl).map_err(MainError::CreateMeshRenderer)?;
    let mut app = App::new(&gl, &args, &mut glyph_cache, &mesh_renderer)?;

    // Initialize the app's window size
    app.set_window_size(width, height);

    while !window.should_close() {
        let now = Instant::now();
        app.update(now);
        app.render(now);

        window.swap_buffers();

        glfw.poll_events();
        for (_, event) in glfw::flush_messages(&events) {
            match event {
                glfw::WindowEvent::FramebufferSize(width, height) => {
                    app.set_window_size(width, height);
                }
                _ => {}
            }
        }
    }

    Ok(())
}
