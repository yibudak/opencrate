//! OpenCrate hardware control window and tray app.
//!
//! Close button hides to tray. Tray menu: Show, presets, Quit.
//! Lighting uses `opencrate-aura` HID; fans use the ASUS COM service worker.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod dashboard;
#[cfg(feature = "diagnostics")]
mod diagnostics;
mod fans;
mod i18n;
mod power;
mod preferences;
mod runtime;
mod software;
mod theme;
mod updates;
mod window_activation;
mod windows_startup;

use i18n::{t, Message};
use opencrate_aura::{
    animation::is_animated,
    playback::{LightingController, Settings},
};
use opencrate_core::{EffectMode, RgbColor};
use std::time::{Duration, Instant};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
};

/// A one-click lighting preset.
#[derive(Clone, Copy)]
struct Preset {
    name: &'static str,
    mode: EffectMode,
    color: RgbColor,
}

const PRESETS: &[Preset] = &[
    Preset {
        name: "Static Red",
        mode: EffectMode::Static,
        color: RgbColor::new(255, 0, 0),
    },
    Preset {
        name: "Rainbow",
        mode: EffectMode::Rainbow,
        color: RgbColor::BLACK,
    },
    Preset {
        name: "Breathing Red",
        mode: EffectMode::Breathing,
        color: RgbColor::new(255, 0, 0),
    },
    Preset {
        name: "Lights Off",
        mode: EffectMode::Off,
        color: RgbColor::BLACK,
    },
];

/// Small version of the OpenCrate brand mark for the notification area.
fn tray_icon_rgba() -> (Vec<u8>, u32, u32) {
    let icon = crate::runtime::icon_from_png(include_bytes!(
        "../../../assets/branding/opencrate-icon-48.png"
    ))
    .expect("embedded OpenCrate tray icon");
    (icon.rgba, icon.width, icon.height)
}

struct StartupRestore {
    settings: Settings,
    retries_left: u8,
    retry_at: Option<Instant>,
}

struct App {
    ui: dashboard::State,
    activity: dashboard::Activity,
    fans: fans::State,
    power: power::State,
    mode: EffectMode,
    color: RgbColor,
    status: Message,
    speed: f64,
    brightness: u8,
    lighting: Option<LightingController>,
    requested: Option<Settings>,
    revision: u64,
    show_id: String,
    preset_ids: Vec<String>,
    quit_id: String,
    tray_items: Vec<(MenuItem, &'static str)>,
    quit_requested: bool,
    store: preferences::Store,
    updates: updates::State,
    install_after_exit: updates::PendingInstall,
    autostart: bool,
    startup_error: Option<String>,
    startup_restore: Option<StartupRestore>,
    instance: windows_startup::Instance,
    window: window_activation::WindowActivation,
    _tray: tray_icon::TrayIcon,
    #[cfg(feature = "diagnostics")]
    diagnostic_tray_handler: std::sync::Arc<dyn Fn(MenuEvent) + Send + Sync>,
    #[cfg(feature = "diagnostics")]
    diagnostic_tray_icon_handler: std::sync::Arc<dyn Fn(TrayIconEvent) + Send + Sync>,
}

impl App {
    fn change_language(&mut self, language: i18n::Language, ctx: &egui::Context) {
        self.store.preferences.language = language;
        i18n::set_language(language);
        for (item, key) in &self.tray_items {
            item.set_text(t(key));
        }
        let _ = self
            ._tray
            .set_tooltip(Some(t("OpenCrate — Hardware control")));
        self.store.changed();
        ctx.request_repaint();
    }

    fn lighting_status(&self) -> String {
        if self.activity == dashboard::Activity::Applied {
            if let Some(settings) = self.requested {
                let mut parts = vec![t(settings.mode.display_name()).to_string()];
                if settings.mode.takes_color() {
                    parts.push(settings.color.to_string());
                }
                if settings.mode != EffectMode::Off {
                    parts.push(format!("{}%", settings.brightness));
                }
                if is_animated(settings.mode) {
                    parts.push(format!("{:.2}×", settings.speed));
                }
                parts.push(t("Synced").into());
                return parts.join(" · ");
            }
        }
        self.status.render()
    }

    fn new(
        ctx: &egui::Context,
        native_window: &winit::window::Window,
        store: preferences::Store,
        mut instance: windows_startup::Instance,
        tray_handler: impl Fn(MenuEvent) + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        i18n::set_language(store.preferences.language);
        store.preferences.theme.apply(ctx);
        let window = window_activation::WindowActivation::new(ctx, native_window)?;
        let activate = window.clone();
        instance.listen(move || activate.show())?;
        let saved = store.preferences.lighting().unwrap_or(Settings {
            mode: EffectMode::Static,
            color: RgbColor::new(255, 0, 0),
            speed: 1.0,
            brightness: 100,
        });
        let restore = store.preferences.restore_on_launch();
        let (autostart, startup_error) = match windows_startup::autostart_enabled() {
            Ok(enabled) => (enabled, None),
            Err(error) => (
                false,
                Some(format!("Could not read Windows startup setting: {error}")),
            ),
        };
        let wake = ctx.clone();
        let lighting = if runtime::hardware_enabled() {
            LightingController::start(move || wake.request_repaint())
        } else {
            Err(opencrate_aura::AuraError::Transport(
                "Diagnostic mode".into(),
            ))
        };
        let status = match &lighting {
            Ok(_) => Message::text("Choose an effect to get started."),
            Err(error) => Message::with(
                "Lighting control failed: {details}",
                vec![("details", error.to_string())],
            ),
        };
        let menu = Menu::new();
        let show = MenuItem::new(t("Show OpenCrate"), true, None);
        let mut tray_items = vec![(show.clone(), "Show OpenCrate")];
        let show_id = show.id().0.clone();
        menu.append(&show).expect("tray menu");
        menu.append(&tray_icon::menu::PredefinedMenuItem::separator())
            .expect("tray menu");
        let mut preset_ids = Vec::new();
        for p in PRESETS {
            let item = MenuItem::new(t(p.name), true, None);
            tray_items.push((item.clone(), p.name));
            preset_ids.push(item.id().0.clone());
            menu.append(&item).expect("tray menu");
        }
        menu.append(&tray_icon::menu::PredefinedMenuItem::separator())
            .expect("tray menu");
        let quit = MenuItem::new(t("Quit"), true, None);
        tray_items.push((quit.clone(), "Quit"));
        let quit_id = quit.id().0.clone();
        menu.append(&quit).expect("tray menu");

        let tray_handler = std::sync::Arc::new(tray_handler);
        let show_handler = tray_handler.clone();
        let show_menu_id = show.id().clone();
        let tray_icon_handler = move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_handler(MenuEvent {
                    id: show_menu_id.clone(),
                });
            }
        };
        #[cfg(feature = "diagnostics")]
        let tray_icon_handler = std::sync::Arc::new(tray_icon_handler);
        #[cfg(feature = "diagnostics")]
        let diagnostic_tray_icon_handler = tray_icon_handler.clone();
        #[cfg(feature = "diagnostics")]
        TrayIconEvent::set_event_handler(Some(move |event| tray_icon_handler(event)));
        #[cfg(not(feature = "diagnostics"))]
        TrayIconEvent::set_event_handler(Some(tray_icon_handler));
        #[cfg(feature = "diagnostics")]
        let diagnostic_tray_handler = tray_handler.clone();
        MenuEvent::set_event_handler(Some(move |event| tray_handler(event)));

        let (rgba, w, h) = tray_icon_rgba();
        let icon = Icon::from_rgba(rgba, w, h).expect("tray icon");
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip(t("OpenCrate — Hardware control"))
            .with_icon(icon)
            .build()
            .expect("tray icon build");

        let mut app = Self {
            ui: dashboard::State::new(ctx, saved.color),
            fans: fans::State::new(ctx),
            power: power::State::new(ctx),
            activity: if lighting.is_ok() {
                dashboard::Activity::Ready
            } else {
                dashboard::Activity::Error
            },
            mode: saved.mode,
            color: saved.color,
            status,
            speed: saved.speed,
            brightness: saved.brightness,
            lighting: lighting.ok(),
            requested: None,
            revision: 0,
            show_id,
            preset_ids,
            quit_id,
            tray_items,
            quit_requested: false,
            store,
            updates: updates::State::default(),
            install_after_exit: None,
            autostart,
            startup_error,
            startup_restore: None,
            instance,
            window,
            _tray: tray,
            #[cfg(feature = "diagnostics")]
            diagnostic_tray_handler,
            #[cfg(feature = "diagnostics")]
            diagnostic_tray_icon_handler,
        };
        if let Some(settings) = restore {
            app.startup_restore = Some(StartupRestore {
                settings,
                retries_left: 5,
                retry_at: None,
            });
            app.enqueue(settings);
        }
        if app.store.preferences.check_updates {
            app.updates.check(ctx);
        }
        Ok(app)
    }

    fn apply(&mut self, mode: EffectMode, color: RgbColor) {
        self.submit(Settings {
            mode,
            color,
            speed: self.speed,
            brightness: self.brightness,
        });
    }

    fn submit(&mut self, settings: Settings) {
        // Any explicit lighting action takes precedence over a pending restore.
        self.startup_restore = None;
        self.enqueue(settings);
    }

    fn enqueue(&mut self, settings: Settings) {
        let Some(lighting) = &mut self.lighting else {
            return;
        };
        match lighting.apply(settings) {
            Ok(revision) => {
                self.revision = revision;
                self.requested = Some(settings);
                self.activity = dashboard::Activity::Applying;
                self.status = Message::text("Applying your changes…");
            }
            Err(error) => {
                self.requested = None;
                self.activity = dashboard::Activity::Error;
                self.status = Message::with(
                    "Lighting control failed: {details}",
                    vec![("details", error.to_string())],
                );
            }
        }
    }

    fn poll_lighting(&mut self, ctx: &egui::Context) {
        let Some(lighting) = &self.lighting else {
            return;
        };
        while let Some(event) = lighting.try_event() {
            if event.revision != self.revision {
                continue;
            }
            self.status = match event.result {
                Ok(settings) => {
                    self.activity = dashboard::Activity::Applied;
                    self.startup_restore = None;
                    let saved = preferences::SavedLighting::from(settings);
                    if self.store.preferences.last_lighting.as_ref() != Some(&saved) {
                        self.store.preferences.last_lighting = Some(saved);
                        self.store.changed();
                    }
                    Message::text("Your lighting is up to date")
                }
                Err(error) => {
                    self.activity = dashboard::Activity::Error;
                    self.requested = None;
                    if let Some(restore) = &mut self.startup_restore {
                        if restore.retries_left > 0 {
                            restore.retries_left -= 1;
                            restore.retry_at = Some(Instant::now() + Duration::from_secs(2));
                        } else {
                            self.startup_restore = None;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        }
                    }
                    Message::with(
                        "Lighting control failed: {details}",
                        vec![("details", error.to_string())],
                    )
                }
            };
        }
    }

    fn apply_preset(&mut self, idx: usize) {
        if let Some(p) = PRESETS.get(idx) {
            self.mode = p.mode;
            self.color = p.color;
            self.ui.hex = p.color.to_hex();
            self.apply(p.mode, p.color);
        }
    }

    fn handle_tray_event(&mut self, event: MenuEvent) {
        let id = &event.id().0;
        if id == &self.quit_id {
            self.quit_requested = true;
        } else if id == &self.show_id {
            self.window.show();
        } else if let Some(idx) = self.preset_ids.iter().position(|p| p == id) {
            self.apply_preset(idx);
        }
    }

    fn poll_preferences(&mut self, ctx: &egui::Context) {
        if self.instance.take_show_request() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if let Some(restore) = &mut self.startup_restore {
            if let Some(due) = restore.retry_at {
                if Instant::now() >= due {
                    restore.retry_at = None;
                    let settings = restore.settings;
                    self.enqueue(settings);
                } else {
                    ctx.request_repaint_after(due.saturating_duration_since(Instant::now()));
                }
            }
        }
        self.store.flush_if_due();
        if let Some(delay) = self.store.pending_delay() {
            ctx.request_repaint_after(delay);
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.window.deactivate();
    }
}

impl App {
    fn update(&mut self, ctx: &egui::Context, draw: bool) {
        // Close button -> hide to tray instead of quitting.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        if self.quit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.poll_lighting(ctx);
        self.poll_preferences(ctx);
        self.fans.poll();
        self.power.poll();
        self.updates.poll(ctx, self.store.preferences.check_updates);

        if draw {
            self.render_dashboard(ctx);
        }
    }
}

fn main() {
    #[cfg(feature = "diagnostics")]
    if diagnostics::enabled() {
        diagnostics::run();
        return;
    }
    let from_startup = std::env::args_os().skip(1).any(|arg| arg == "--startup");
    let instance = match windows_startup::Instance::claim(from_startup) {
        Ok(Some(instance)) => instance,
        Ok(None) => return,
        Err(error) => {
            eprintln!("Could not initialize opencrate: {error}");
            return;
        }
    };
    let store = preferences::Store::load();
    let start_hidden = from_startup && store.preferences.start_in_tray && store.error.is_none();
    match runtime::run(store, instance, start_hidden) {
        Ok(Some((installer, language))) => {
            // The runtime has dropped App, restored fans and released AppMutex.
            if let Err(error) = installer.launch(language) {
                updates::show_launch_error(&error, language);
                if let Ok(executable) = std::env::current_exe() {
                    let _ = std::process::Command::new(executable).spawn();
                }
            }
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("opencrate-ui failed: {error}");
            std::process::exit(1);
        }
    }
}
