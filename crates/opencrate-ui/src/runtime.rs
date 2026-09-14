//! Event-driven native window host. No graphics driver is loaded.
//!
//! Workers and tray events are serviced even when the window cannot repaint.
//! Frame buffers and raster caches exist only while the window is drawable.

use crate::{
    preferences,
    software::{Painter, Textures},
    windows_startup, App,
};
use egui::{ViewportCommand, ViewportId, ViewportInfo};
use std::{
    error::Error,
    num::NonZeroU32,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    window::{Window, WindowId},
};

pub fn hardware_enabled() -> bool {
    #[cfg(feature = "diagnostics")]
    if crate::diagnostics::enabled() {
        return false;
    }
    true
}

pub fn icon_from_png(bytes: &[u8]) -> Result<egui::IconData, image::ImageError> {
    let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)?.into_rgba8();
    Ok(egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}

enum Event {
    Tray(tray_icon::menu::MenuEvent),
    Repaint { when: Instant, pass: u64 },
    AccessKit(egui_winit::accesskit_winit::Event),
}
impl From<egui_winit::accesskit_winit::Event> for Event {
    fn from(event: egui_winit::accesskit_winit::Event) -> Self {
        Self::AccessKit(event)
    }
}

#[derive(Default)]
struct Schedule(Option<Instant>);
impl Schedule {
    fn request(&mut self, when: Instant) {
        self.0 = Some(self.0.map_or(when, |due| due.min(when)));
    }
    fn take_due(&mut self, now: Instant) -> bool {
        if self.0.is_some_and(|due| due <= now) {
            self.0 = None;
            true
        } else {
            false
        }
    }
}

pub(crate) struct Running {
    app: App,
    painter: Option<Painter>,
    textures: Textures,
    input: egui_winit::State,
    ctx: egui::Context,
    window: Arc<Window>,
    info: ViewportInfo,
    occluded: bool,
    frame_time: Duration,
}

impl Running {
    #[cfg(feature = "diagnostics")]
    pub fn diagnostic_step(&mut self, step: usize) {
        use crate::{dashboard::Page, i18n::Language};
        match step {
            1 => self.info.events.push(egui::ViewportEvent::Close),
            2 => (self.app.diagnostic_tray_icon_handler)(tray_icon::TrayIconEvent::Click {
                id: self.app._tray.id().clone(),
                position: Default::default(),
                rect: Default::default(),
                button: tray_icon::MouseButton::Left,
                button_state: tray_icon::MouseButtonState::Up,
            }),
            3 => self.app.change_language(Language::Turkish, &self.ctx),
            4 => self.app.change_language(Language::Chinese, &self.ctx),
            5 => self.app.ui.select_page(Page::Fans),
            6 => self.app.ui.select_page(Page::Power),
            7 => {
                self.app.ui.select_page(Page::Lighting);
                self.app.mode = opencrate_core::EffectMode::Rainbow;
                self.window.focus_window();
            }
            8 => self.info.events.push(egui::ViewportEvent::Close),
            9 => {
                (self.app.diagnostic_tray_handler)(tray_icon::menu::MenuEvent {
                    id: self.app.show_id.clone().into(),
                });
                self.ctx.set_zoom_factor(1.5);
            }
            10 => self.info.events.push(egui::ViewportEvent::Close),
            11 => {
                // Exercise the installed menu callback after an idle hidden
                // phase, after the last diagnostic timer has been consumed.
                // Do not add a repaint that could mask a missed wake.
                let handler = self.app.diagnostic_tray_handler.clone();
                let id = self.app.quit_id.clone().into();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(200));
                    handler(tray_icon::menu::MenuEvent { id });
                });
                return;
            }
            _ => {}
        }
        self.ctx.request_repaint();
    }
    fn drawable(&self) -> bool {
        self.window.is_visible() != Some(false)
            && self.window.is_minimized() != Some(true)
            && !self.occluded
    }

    fn update(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn Error>> {
        let started = Instant::now();
        egui_winit::update_viewport_info(&mut self.info, &self.ctx, &self.window, false);
        self.input
            .egui_input_mut()
            .viewports
            .insert(ViewportId::ROOT, self.info.clone());
        let mut raw = self.input.take_egui_input(&self.window);
        // egui subtracts this estimate from delayed repaints. A software host
        // does not wait for GPU vsync: the default 1/60 would double a 30 Hz timer.
        raw.predicted_dt = self.frame_time.as_secs_f32();
        self.info.events.clear();
        let draw = self.drawable();
        let output = self.ctx.run(raw, |ctx| self.app.update(ctx, draw));
        #[cfg(feature = "diagnostics")]
        crate::diagnostics::updated(&self.ctx);
        self.input
            .handle_platform_output(&self.window, output.platform_output);

        let mut commands = Vec::new();
        if let Some(viewport) = output.viewport_output.get(&ViewportId::ROOT) {
            for command in &viewport.commands {
                match command {
                    ViewportCommand::Close if self.app.quit_requested => event_loop.exit(),
                    ViewportCommand::Close | ViewportCommand::CancelClose => {}
                    ViewportCommand::Visible(visible) => {
                        self.window.set_visible(*visible);
                        if *visible {
                            self.occluded = false;
                            self.ctx.request_repaint();
                        }
                    }
                    _ => commands.push(command.clone()),
                }
            }
        }
        let mut actions = Default::default();
        egui_winit::process_viewport_commands(
            &self.ctx,
            &mut self.info,
            commands,
            &self.window,
            &mut actions,
        );
        // Clipboard shortcuts from native menus are not used by this application.
        self.textures.apply(&output.textures_delta);
        if draw && self.drawable() && !event_loop.exiting() {
            let size = self.window.inner_size();
            if let (Some(width), Some(height)) =
                (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
            {
                if self.painter.is_none() {
                    self.painter = Some(Painter::new(self.window.clone())?);
                }
                let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
                self.painter.as_mut().unwrap().paint(
                    [width, height],
                    &primitives,
                    &output.textures_delta,
                    &self.textures,
                    output.pixels_per_point,
                )?;
            }
        } else {
            self.painter = None;
        }
        self.textures.free(&output.textures_delta);
        self.frame_time = started.elapsed().min(Duration::from_millis(16));
        Ok(())
    }
}

struct Host {
    startup: Option<(preferences::Store, windows_startup::Instance, bool)>,
    running: Option<Running>,
    proxy: EventLoopProxy<Event>,
    schedule: Schedule,
    error: Option<Box<dyn Error>>,
}

impl Host {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: Box<dyn Error>) {
        self.error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler<Event> for Host {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Some((store, instance, hidden)) = self.startup.take() else {
            return;
        };
        let result = (|| -> Result<Running, Box<dyn Error>> {
            let ctx = egui::Context::default();
            let proxy = self.proxy.clone();
            ctx.set_request_repaint_callback(move |request| {
                if let Some(when) = Instant::now().checked_add(request.delay) {
                    let _ = proxy.send_event(Event::Repaint {
                        when,
                        pass: request.current_cumulative_pass_nr,
                    });
                }
            });
            let viewport = egui::ViewportBuilder::default()
                .with_title("OpenCrate")
                .with_icon(icon_from_png(include_bytes!(
                    "../../../assets/branding/opencrate-icon-256.png"
                ))?)
                .with_inner_size([1100.0, 800.0])
                .with_min_inner_size([760.0, 620.0])
                .with_visible(false);
            let window = Arc::new(egui_winit::create_window(&ctx, event_loop, &viewport)?);
            let mut input = egui_winit::State::new(
                ctx.clone(),
                ViewportId::ROOT,
                event_loop,
                Some(window.scale_factor() as f32),
                window.theme(),
                Some(8192),
            );
            input.init_accesskit(event_loop, &window, self.proxy.clone());
            let mut info = ViewportInfo::default();
            egui_winit::update_viewport_info(&mut info, &ctx, &window, true);
            let proxy = self.proxy.clone();
            let app = App::new(&ctx, &window, store, instance, move |event| {
                let _ = proxy.send_event(Event::Tray(event));
            })?;
            window.set_visible(!hidden);
            Ok(Running {
                app,
                painter: None,
                textures: Textures::default(),
                input,
                ctx,
                window,
                info,
                occluded: false,
                frame_time: Duration::ZERO,
            })
        })();
        match result {
            Ok(running) => {
                self.running = Some(running);
                self.schedule.request(Instant::now());
            }
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: Event) {
        match event {
            Event::Tray(event) => {
                if let Some(running) = &mut self.running {
                    running.app.handle_tray_event(event);
                    // Tray actions must run even while hidden or minimized,
                    // independently of egui's coalesced repaint callbacks.
                    self.schedule.request(Instant::now());
                }
            }
            Event::Repaint { when, pass } => {
                let current = self
                    .running
                    .as_ref()
                    .map_or(0, |running| running.ctx.cumulative_pass_nr());
                // A queued callback must not redraw an already superseded pass.
                if current <= pass.saturating_add(1) {
                    self.schedule.request(when);
                }
            }
            Event::AccessKit(event) => {
                let Some(running) = self.running.as_mut() else {
                    return;
                };
                if event.window_id != running.window.id() {
                    return;
                }
                use egui_winit::accesskit_winit::WindowEvent;
                match event.window_event {
                    WindowEvent::InitialTreeRequested => running.ctx.enable_accesskit(),
                    WindowEvent::ActionRequested(request) => {
                        running.input.on_accesskit_action_request(request)
                    }
                    WindowEvent::AccessibilityDeactivated => {}
                }
                self.schedule.request(Instant::now());
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(running) = &mut self.running else {
            return;
        };
        if id != running.window.id() {
            return;
        }
        let response = running.input.on_window_event(&running.window, &event);
        #[cfg(feature = "diagnostics")]
        crate::diagnostics::window_event(&event);
        match event {
            WindowEvent::CloseRequested => {
                running.info.events.push(egui::ViewportEvent::Close);
                self.schedule.request(Instant::now());
            }
            WindowEvent::Occluded(occluded) => {
                running.occluded = occluded;
                if occluded {
                    running.painter = None;
                }
                self.schedule.request(Instant::now());
            }
            WindowEvent::RedrawRequested => {
                // Windows dispatches redraws inside its modal resize loop, when
                // about_to_wait is suspended. Lay out and paint at the live size.
                self.schedule.take_due(Instant::now());
                if let Err(error) = running.update(event_loop) {
                    self.fail(event_loop, error);
                }
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                running.window.request_redraw();
                self.schedule.request(Instant::now());
            }
            _ if response.repaint => self.schedule.request(Instant::now()),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(feature = "diagnostics")]
        if crate::diagnostics::enabled() {
            if let Some(running) = &mut self.running {
                crate::diagnostics::advance(running);
            }
        }
        if self.schedule.take_due(Instant::now()) {
            if let Some(running) = &mut self.running {
                if running.drawable() {
                    running.window.request_redraw();
                } else if let Err(error) = running.update(event_loop) {
                    // Hidden windows still service workers, tray actions and
                    // preferences without relying on native paint events.
                    self.fail(event_loop, error);
                }
            }
        }
        #[cfg(feature = "diagnostics")]
        if crate::diagnostics::enabled() {
            if let Some(due) = crate::diagnostics::deadline() {
                self.schedule.request(due);
            }
        }
        event_loop.set_control_flow(
            self.schedule
                .0
                .map_or(ControlFlow::Wait, ControlFlow::WaitUntil),
        );
    }
}

pub fn run(
    store: preferences::Store,
    instance: windows_startup::Instance,
    hidden: bool,
) -> Result<crate::updates::PendingInstall, Box<dyn Error>> {
    let event_loop = EventLoop::<Event>::with_user_event().build()?;
    let mut host = Host {
        startup: Some((store, instance, hidden)),
        running: None,
        proxy: event_loop.create_proxy(),
        schedule: Schedule::default(),
        error: None,
    };
    event_loop.run_app(&mut host)?;
    if let Some(error) = host.error {
        return Err(error);
    }
    // Transfer only the verified download. Host and App are dropped before the
    // caller can launch Setup, releasing the hardware workers and instance lock.
    Ok(host
        .running
        .as_mut()
        .and_then(|running| running.app.install_after_exit.take()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn repaint_deadlines_coalesce_without_polling_or_delaying_worker_wakes() {
        let now = Instant::now();
        let mut schedule = Schedule::default();
        assert!(!schedule.take_due(now));
        schedule.request(now + Duration::from_secs(3));
        schedule.request(now + Duration::from_secs(5));
        assert!(!schedule.take_due(now));
        schedule.request(now);
        assert!(schedule.take_due(now));
        assert_eq!(schedule.0, None);
        assert!(!schedule.take_due(now + Duration::from_secs(10)));
    }
}
