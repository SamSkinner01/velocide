//! Window creation and the main loop (winit `ApplicationHandler`).
//!
//! ```ignore
//! struct MyGame { /* ... */ }
//! impl engine::app::Game for MyGame {
//!     fn init(ctx: &mut EngineContext) -> anyhow::Result<Self> { /* create meshes/textures */ }
//!     fn frame(&mut self, ctx: &mut EngineContext) {
//!         // read ctx.input, step the simulation, then:
//!         ctx.renderer.render(&frame_desc, None).ok();
//!     }
//! }
//! engine::app::run::<MyGame>(AppConfig::default())
//! ```
//!
//! The loop redraws continuously. Each redraw: poll gamepads → `Game::frame` → reset per-frame
//! input. Window events go to `Game::window_event` first (e.g. for the debug UI); unless it
//! returns `true` (consumed) they also update [`InputState`].

use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

use crate::gfx::RetroSettings;
use crate::input::{GamepadAxis, GamepadButton, GamepadEvent, InputState};
use crate::render::Renderer;

/// Longest frame time reported to the game (a debugger pause shouldn't explode the sim).
const MAX_FRAME_DT: f32 = 0.25;

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub title: String,
    /// Initial inner size in logical pixels.
    pub width: u32,
    pub height: u32,
    pub retro: RetroSettings,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            title: "VELOCIDE".to_owned(),
            width: 1280,
            height: 720,
            retro: RetroSettings::default(),
        }
    }
}

/// Everything the game can reach from `init` / `frame`.
pub struct EngineContext {
    pub window: Arc<Window>,
    pub renderer: Renderer,
    pub input: InputState,
    /// Seconds since the previous frame (clamped to 0.25 s).
    pub frame_dt: f32,
    /// Seconds since startup.
    pub time: f64,
    quit_requested: bool,
    cursor_grabbed: bool,
    fullscreen: bool,
}

impl EngineContext {
    /// Locks (or, where unsupported, confines) and hides the mouse cursor for mouselook.
    pub fn set_cursor_grab(&mut self, grab: bool) {
        self.cursor_grabbed = grab;
        apply_cursor_grab(&self.window, grab);
    }

    pub fn cursor_grabbed(&self) -> bool {
        self.cursor_grabbed
    }

    /// Borderless fullscreen on the current monitor.
    pub fn set_fullscreen(&mut self, fullscreen: bool) {
        self.fullscreen = fullscreen;
        self.window
            .set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
    }

    pub fn is_fullscreen(&self) -> bool {
        self.fullscreen
    }

    /// Exits the main loop after this frame (`Game::exit` is still called).
    pub fn quit(&mut self) {
        self.quit_requested = true;
    }
}

fn apply_cursor_grab(window: &Window, grab: bool) {
    if grab {
        let locked = window.set_cursor_grab(CursorGrabMode::Locked);
        if locked.is_err()
            && let Err(e) = window.set_cursor_grab(CursorGrabMode::Confined)
        {
            log::warn!("cursor grab not supported: {e}");
        }
    } else if let Err(e) = window.set_cursor_grab(CursorGrabMode::None) {
        log::warn!("releasing cursor grab failed: {e}");
    }
    window.set_cursor_visible(!grab);
}

pub trait Game: Sized + 'static {
    fn init(ctx: &mut EngineContext) -> anyhow::Result<Self>;
    /// Called once per displayed frame. The game renders by calling `ctx.renderer.render`.
    fn frame(&mut self, ctx: &mut EngineContext);
    /// Raw window events, before the input state sees them. Return `true` to consume the event.
    fn window_event(&mut self, _ctx: &mut EngineContext, _event: &WindowEvent) -> bool {
        false
    }
    /// Called once when the loop ends.
    fn exit(&mut self, _ctx: &mut EngineContext) {}
}

struct Running<G> {
    ctx: EngineContext,
    game: G,
}

struct App<G: Game> {
    config: AppConfig,
    running: Option<Running<G>>,
    gilrs: Option<gilrs::Gilrs>,
    start: Instant,
    last_frame: Instant,
    error: Option<anyhow::Error>,
}

/// Opens the window and runs the game until it quits or the window is closed.
pub fn run<G: Game>(config: AppConfig) -> anyhow::Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let gilrs = match gilrs::Gilrs::new() {
        Ok(g) => Some(g),
        Err(e) => {
            log::warn!("gamepad support unavailable: {e}");
            None
        }
    };
    let now = Instant::now();
    let mut app = App::<G> {
        config,
        running: None,
        gilrs,
        start: now,
        last_frame: now,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

impl<G: Game> App<G> {
    fn start(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        let attributes = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.config.width,
                self.config.height,
            ))
            .with_min_inner_size(PhysicalSize::new(160, 120));
        let window = Arc::new(event_loop.create_window(attributes)?);
        let renderer = Renderer::new(window.clone(), &self.config.retro)?;
        let mut input = InputState::new();
        if let Some(gilrs) = &self.gilrs {
            for _ in gilrs.gamepads() {
                input.handle_gamepad_event(GamepadEvent::Connected);
            }
        }
        let mut ctx = EngineContext {
            window,
            renderer,
            input,
            frame_dt: 0.0,
            time: 0.0,
            quit_requested: false,
            cursor_grabbed: false,
            fullscreen: false,
        };
        let game = G::init(&mut ctx)?;
        ctx.window.request_redraw();
        self.last_frame = Instant::now();
        self.running = Some(Running { ctx, game });
        Ok(())
    }

    fn poll_gamepads(&mut self) {
        let (Some(gilrs), Some(running)) = (&mut self.gilrs, &mut self.running) else {
            return;
        };
        while let Some(event) = gilrs.next_event() {
            let converted = match event.event {
                gilrs::EventType::ButtonPressed(b, _) => {
                    GamepadButton::from_gilrs(b).map(|b| GamepadEvent::Button(b, true))
                }
                gilrs::EventType::ButtonReleased(b, _) => {
                    GamepadButton::from_gilrs(b).map(|b| GamepadEvent::Button(b, false))
                }
                // Analog triggers report their value through ButtonChanged.
                gilrs::EventType::ButtonChanged(gilrs::Button::LeftTrigger2, v, _) => {
                    Some(GamepadEvent::Axis(GamepadAxis::LeftTrigger, v))
                }
                gilrs::EventType::ButtonChanged(gilrs::Button::RightTrigger2, v, _) => {
                    Some(GamepadEvent::Axis(GamepadAxis::RightTrigger, v))
                }
                gilrs::EventType::AxisChanged(axis, v, _) => {
                    GamepadAxis::from_gilrs(axis).map(|a| GamepadEvent::Axis(a, v))
                }
                gilrs::EventType::Connected => Some(GamepadEvent::Connected),
                gilrs::EventType::Disconnected => Some(GamepadEvent::Disconnected),
                _ => None,
            };
            if let Some(e) = converted {
                running.ctx.input.handle_gamepad_event(e);
            }
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        self.poll_gamepads();
        let Some(running) = &mut self.running else {
            return;
        };
        let now = Instant::now();
        running.ctx.frame_dt = (now - self.last_frame).as_secs_f32().min(MAX_FRAME_DT);
        running.ctx.time = (now - self.start).as_secs_f64();
        self.last_frame = now;

        running.game.frame(&mut running.ctx);
        running.ctx.input.end_frame();

        if running.ctx.quit_requested {
            event_loop.exit();
        } else {
            running.ctx.window.request_redraw();
        }
    }
}

impl<G: Game> ApplicationHandler for App<G> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() || self.error.is_some() {
            return;
        }
        if let Err(e) = self.start(event_loop) {
            self.error = Some(e);
            event_loop.exit();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if matches!(event, WindowEvent::RedrawRequested) {
            self.redraw(event_loop);
            return;
        }
        let Some(running) = &mut self.running else {
            return;
        };
        let ctx = &mut running.ctx;
        match &event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::Resized(size) => ctx.renderer.resize(size.width, size.height),
            WindowEvent::ScaleFactorChanged { .. } => {
                let size = ctx.window.inner_size();
                ctx.renderer.resize(size.width, size.height);
            }
            WindowEvent::Focused(true) if ctx.cursor_grabbed => {
                // Compositors drop pointer locks when focus leaves; take it back.
                apply_cursor_grab(&ctx.window, true);
            }
            _ => {}
        }
        if !running.game.window_event(&mut running.ctx, &event) {
            running.ctx.input.handle_window_event(&event);
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let (DeviceEvent::MouseMotion { delta }, Some(running)) = (event, &mut self.running) {
            running.ctx.input.handle_mouse_motion(delta);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(running) = &self.running {
            running.ctx.window.request_redraw();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(running) = &mut self.running {
            running.game.exit(&mut running.ctx);
        }
    }
}
