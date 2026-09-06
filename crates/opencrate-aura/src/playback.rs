//! One background owner for HID writes, independent of window redraws.

use crate::{
    animation::{self, Timeline},
    AuraDevice, AuraError, DIRECT_LED_COUNT,
};
use opencrate_core::{Channel, EffectMode, RgbColor};
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const FRAME_INTERVAL: Duration = Duration::from_millis(33);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub mode: EffectMode,
    pub color: RgbColor,
    pub speed: f64,
    /// Global brightness percentage, independent of the selected base color.
    pub brightness: u8,
}

impl Settings {
    fn native_color(self) -> RgbColor {
        animation::apply_brightness(self.color, self.brightness)
    }

    /// Native multicolor modes have no verified brightness control. Keep the
    /// current dimmed color on exit/failure instead of jumping to full brightness.
    fn hardware_fallback(mut self, frame: &[RgbColor]) -> Self {
        if self.brightness == 0 {
            self.mode = EffectMode::Off;
            self.color = RgbColor::BLACK;
        } else if self.brightness < 100
            && animation::is_animated(self.mode)
            && !self.mode.takes_color()
        {
            self.mode = EffectMode::Static;
            self.color = frame.first().copied().unwrap_or(RgbColor::BLACK);
            self.brightness = 100; // Frame colors have already been scaled.
        }
        self
    }
}

#[derive(Debug)]
pub struct Event {
    pub revision: u64,
    pub result: Result<Settings, String>,
}

enum Command {
    Apply(u64, Settings),
    Shutdown,
}

pub struct LightingController {
    commands: mpsc::Sender<Command>,
    events: mpsc::Receiver<Event>,
    thread: Option<thread::JoinHandle<()>>,
    revision: u64,
}

impl LightingController {
    /// Does not touch hardware until the first apply request.
    pub fn start(wake: impl Fn() + Send + 'static) -> Result<Self, AuraError> {
        let (commands, receiver) = mpsc::channel();
        let (sender, events) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("aura-playback".into())
            .spawn(move || {
                run(
                    receiver,
                    |event| {
                        let _ = sender.send(event);
                        wake();
                    },
                    AuraDevice::open,
                );
            })
            .map_err(|e| AuraError::Transport(e.to_string()))?;
        Ok(Self {
            commands,
            events,
            thread: Some(thread),
            revision: 0,
        })
    }

    pub fn apply(&mut self, mut settings: Settings) -> Result<u64, AuraError> {
        settings.speed = animation::normalize_speed(settings.speed);
        settings.brightness = settings.brightness.min(100);
        self.revision += 1;
        self.commands
            .send(Command::Apply(self.revision, settings))
            .map_err(|_| AuraError::Transport("lighting worker stopped".into()))?;
        Ok(self.revision)
    }

    pub fn try_event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}

impl Drop for LightingController {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

trait Output {
    fn native(&self, settings: Settings) -> Result<(), AuraError>;
    fn direct(&self) -> Result<(), AuraError>;
    fn frame(&self, colors: &[RgbColor]) -> Result<(), AuraError>;
}

impl Output for AuraDevice {
    fn native(&self, settings: Settings) -> Result<(), AuraError> {
        self.apply_effect(Channel::Sync, settings.mode, settings.native_color())
            .map(|_| ())
    }
    fn direct(&self) -> Result<(), AuraError> {
        self.start_direct()
    }
    fn frame(&self, colors: &[RgbColor]) -> Result<(), AuraError> {
        self.write_frame(colors)
    }
}

fn run<D: Output>(
    commands: mpsc::Receiver<Command>,
    emit: impl Fn(Event),
    open: impl Fn() -> Result<D, AuraError>,
) {
    let mut device: Option<D> = None;
    let mut active: Option<(u64, Settings)> = None;
    let mut direct = false;
    let mut timeline = Timeline::default();
    let mut last_tick = Instant::now();
    let mut next_frame = last_tick;
    let mut colors = [RgbColor::BLACK; DIRECT_LED_COUNT];
    loop {
        let command = if direct {
            match commands.recv_timeout(next_frame.saturating_duration_since(Instant::now())) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => Some(Command::Shutdown),
            }
        } else {
            Some(commands.recv().unwrap_or(Command::Shutdown))
        };
        // Coalesce slider updates so slow USB writes cannot build a stale queue.
        let mut latest = command;
        while !matches!(latest, Some(Command::Shutdown)) {
            match commands.try_recv() {
                Ok(command) => latest = Some(command),
                Err(_) => break,
            }
        }
        if let Some(Command::Shutdown) = latest {
            // Leave a self-running hardware effect when the user really quits.
            if direct {
                if let (Some(device), Some((_, settings))) = (&device, active) {
                    if let Err(error) = device.native(settings.hardware_fallback(&colors)) {
                        eprintln!("Could not restore hardware effect on quit: {error}");
                    }
                }
            }
            return;
        }
        let now = Instant::now();
        if let Some((_, settings)) = active {
            timeline.advance(now.duration_since(last_tick), settings.speed);
        }
        last_tick = now;
        let requested = match latest {
            Some(Command::Apply(revision, settings)) => {
                if active.is_none_or(|(_, previous)| previous.mode != settings.mode) {
                    timeline = Timeline::default();
                }
                Some((revision, settings))
            }
            _ => None,
        };
        let Some((revision, settings)) = requested.or(active) else {
            continue;
        };
        let result = (|| {
            if device.is_none() {
                device = Some(open()?);
            }
            let device = device.as_ref().expect("opened above");
            if animation::is_animated(settings.mode) {
                if !direct {
                    device.direct()?;
                }
                animation::render(
                    settings.mode,
                    settings.color,
                    timeline.seconds(),
                    settings.brightness,
                    &mut colors,
                );
                device.frame(&colors)?;
            } else if requested.is_some() {
                device.native(settings)?;
            }
            Ok::<(), AuraError>(())
        })();
        match result {
            Ok(()) => {
                active = Some((revision, settings));
                direct = animation::is_animated(settings.mode);
                if requested.is_some() {
                    emit(Event {
                        revision,
                        result: Ok(settings),
                    });
                }
            }
            Err(error) => {
                // A single best-effort native fallback; no retry loop hammering a
                // disconnected device. A new Apply request opens it again.
                let restored = device
                    .as_ref()
                    .is_some_and(|d| d.native(settings.hardware_fallback(&colors)).is_ok());
                emit(Event {
                    revision,
                    result: Err(format!(
                        "{error}. {}",
                        if restored {
                            "Using hardware fallback; click Apply to retry animation control."
                        } else {
                            "Click Apply to reconnect."
                        }
                    )),
                });
                direct = false;
                active = None;
                device = None;
            }
        }
        // Include transfer time in the cadence; never catch up with a burst.
        next_frame = (now + FRAME_INTERVAL).max(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct Fake(Arc<Mutex<Vec<&'static str>>>);
    impl Output for Fake {
        fn native(&self, _: Settings) -> Result<(), AuraError> {
            self.0.lock().unwrap().push("native");
            Ok(())
        }
        fn direct(&self) -> Result<(), AuraError> {
            self.0.lock().unwrap().push("direct");
            Ok(())
        }
        fn frame(&self, _: &[RgbColor]) -> Result<(), AuraError> {
            self.0.lock().unwrap().push("frame");
            Ok(())
        }
    }

    #[test]
    fn speed_update_keeps_direct_session_and_static_stops_frames() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let fake = Fake(log.clone());
        let (tx, rx) = mpsc::channel();
        let (events, results) = mpsc::channel();
        let worker = thread::spawn(move || {
            run(
                rx,
                |event| {
                    events.send(event).unwrap();
                },
                || Ok(fake.clone()),
            )
        });
        let mut settings = Settings {
            mode: EffectMode::Rainbow,
            color: RgbColor::BLACK,
            speed: 1.0,
            brightness: 100,
        };
        for revision in 1..=3 {
            if revision == 2 {
                settings.speed = 4.0;
            }
            if revision == 3 {
                settings.mode = EffectMode::Static;
            }
            tx.send(Command::Apply(revision, settings)).unwrap();
            let event = results.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(event.revision, revision);
            assert!(event.result.is_ok());
        }
        tx.send(Command::Shutdown).unwrap();
        worker.join().unwrap();
        let log = log.lock().unwrap();
        assert_eq!(log.iter().filter(|&&entry| entry == "direct").count(), 1);
        assert_eq!(log.last(), Some(&"native"));
        assert_eq!(log.iter().filter(|&&entry| entry == "native").count(), 1);
    }

    #[test]
    fn quit_restores_native_animation() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let fake = Fake(log.clone());
        let (tx, rx) = mpsc::channel();
        let stop = tx.clone();
        tx.send(Command::Apply(
            1,
            Settings {
                mode: EffectMode::Breathing,
                color: RgbColor::new(0, 0, 255),
                speed: 0.25,
                brightness: 100,
            },
        ))
        .unwrap();
        run(
            rx,
            |_| {
                stop.send(Command::Shutdown).unwrap();
            },
            || Ok(fake.clone()),
        );
        assert_eq!(*log.lock().unwrap(), vec!["direct", "frame", "native"]);
    }

    #[test]
    fn static_and_quit_fallback_respect_brightness() {
        let mut settings = Settings {
            mode: EffectMode::Static,
            color: RgbColor::new(40, 120, 240),
            speed: 1.0,
            brightness: 25,
        };
        assert_eq!(settings.native_color(), RgbColor::new(10, 30, 60));
        assert_eq!(settings.color, RgbColor::new(40, 120, 240));
        settings.mode = EffectMode::Breathing;
        assert_eq!(
            settings.hardware_fallback(&[]).native_color(),
            RgbColor::new(10, 30, 60)
        );
        settings.mode = EffectMode::Rainbow;
        let fallback = settings.hardware_fallback(&[RgbColor::new(0, 64, 32)]);
        assert_eq!(fallback.mode, EffectMode::Static);
        assert_eq!(fallback.native_color(), RgbColor::new(0, 64, 32));
        settings.brightness = 0;
        assert_eq!(settings.hardware_fallback(&[]).mode, EffectMode::Off);
        settings.brightness = 100;
        assert_eq!(settings.hardware_fallback(&[]).mode, EffectMode::Rainbow);
    }
}
