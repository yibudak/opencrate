//! ASUS service data and fan policy. Duty values on the wire are 0..=255.

#[cfg(windows)]
use std::time::Duration;
use std::{
    sync::{mpsc, Arc, Mutex},
    thread,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub temperature: u8,
    pub duty: u8,
}

pub fn percent(raw: u8) -> f32 {
    f32::from(raw) * 100.0 / 255.0
}

pub fn raw_duty(percent: u8) -> Result<u8, String> {
    if percent > 100 {
        return Err("Fan duty must be between 0 and 100%.".into());
    }
    Ok(((u16::from(percent) * 255 + 50) / 100) as u8)
}

#[derive(Clone, Debug)]
pub struct Profile {
    pub index: i32,
    pub name: String,
    pub curve: Vec<Point>,
}

#[derive(Clone, Debug)]
pub struct Fan {
    pub id: u32,
    pub name: String,
    pub duty: u8,
    pub minimum: u8,
    pub curve: Vec<Point>,
    pub profiles: Vec<Profile>,
    pub can_restore: bool,
    pub writable: bool,
}

#[derive(Clone, Debug)]
pub enum Target {
    Profile(i32),
    Custom(Vec<Point>),
    Restore,
}

// Explicit custom controls never go below the service's reported minimum.
// Preserve the controller's final full-speed thermal point, or use 85 C if
// its current curve is a constant full-speed profile.
pub fn critical_temperature(curve: &[Point]) -> u8 {
    curve
        .last()
        .filter(|p| p.temperature >= 20 && p.duty == 255)
        .map_or(85, |p| p.temperature.min(85))
}

pub fn validate_custom(points: &[Point], fan: &Fan) -> Result<(), String> {
    if points.len() != fan.curve.len() || !(4..=10).contains(&points.len()) {
        return Err("The curve must use the fan controller's point count.".into());
    }
    let critical = critical_temperature(&fan.curve);
    if points
        .iter()
        .any(|p| !(20..=critical).contains(&p.temperature) || p.duty < fan.minimum)
    {
        return Err(format!(
            "Use 20–{critical} °C and at least {:.0}% duty.",
            percent(fan.minimum).ceil()
        ));
    }
    if points
        .windows(2)
        .any(|p| p[0].temperature > p[1].temperature || p[0].duty > p[1].duty)
    {
        return Err("Temperature and speed must increase from left to right.".into());
    }
    if points.last().is_none_or(|p| p.duty != 255) {
        return Err(format!("The final point must reach 100% by {critical} °C."));
    }
    Ok(())
}

pub fn manual_curve(fan: &Fan, duty: u8) -> Result<Vec<Point>, String> {
    let duty = raw_duty(duty)?;
    let critical = critical_temperature(&fan.curve);
    let n = fan.curve.len();
    if !(4..=10).contains(&n) {
        return Err("Unsupported fan curve point count.".into());
    }
    let points = (0..n)
        .map(|i| Point {
            temperature: 20 + ((usize::from(critical - 20) * i) / (n - 1)) as u8,
            duty: if i == n - 1 { 255 } else { duty },
        })
        .collect::<Vec<_>>();
    validate_custom(&points, fan)?;
    Ok(points)
}

pub enum Command {
    Refresh,
    Apply { id: u32, target: Target },
    Quick(crate::quick::QuickMode),
    UndoQuick,
    Stop,
}

#[derive(Debug, Default)]
pub struct Snapshot {
    pub fans: Vec<Fan>,
    pub can_undo: bool,
}

pub struct Event {
    pub result: Result<Snapshot, String>,
    /// Present for an explicit operation; periodic reads do not clear feedback.
    pub action: Option<Result<String, String>>,
}

/// Keep only the latest telemetry while retaining any unconsumed command
/// acknowledgement. Hidden windows may not drain events for many hours.
#[derive(Default)]
struct Mailbox(Mutex<Option<Event>>);
impl Mailbox {
    fn publish(&self, mut event: Event) {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if event.action.is_none() {
            event.action = slot.take().and_then(|previous| previous.action);
        }
        *slot = Some(event);
    }
    fn take(&self) -> Option<Event> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

pub struct Controller {
    commands: mpsc::Sender<Command>,
    events: Arc<Mailbox>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Controller {
    pub fn start(wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let (commands, incoming) = mpsc::channel();
        let events = Arc::new(Mailbox::default());
        let outgoing = events.clone();
        let worker = thread::Builder::new()
            .name("opencrate-fans".into())
            .spawn(move || {
                #[cfg(windows)]
                run(incoming, outgoing, wake);
                #[cfg(not(windows))]
                {
                    let _ = incoming;
                    outgoing.publish(Event {
                        result: Err("ASUS fan control requires Windows.".into()),
                        action: None,
                    });
                    wake();
                }
            })?;
        Ok(Self {
            commands,
            events,
            worker: Some(worker),
        })
    }
    pub fn send(&self, command: Command) -> Result<(), String> {
        self.commands
            .send(command)
            .map_err(|_| "Fan worker is unavailable.".into())
    }
    pub fn try_event(&self) -> Option<Event> {
        self.events.take()
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(windows)]
fn run(incoming: mpsc::Receiver<Command>, outgoing: Arc<Mailbox>, wake: impl Fn()) {
    let mut session: Result<crate::asus::Session, String> = Err("Not connected".into());
    let mut command = Command::Refresh;
    loop {
        if matches!(command, Command::Stop) {
            break;
        }
        let action = match command {
            Command::Apply { id, target } => Some(
                session
                    .as_mut()
                    .map_err(|e| e.clone())
                    .and_then(|s| s.apply(id, target)),
            ),
            Command::Quick(mode) => Some(
                session
                    .as_mut()
                    .map_err(|e| e.clone())
                    .and_then(|s| s.apply_quick(mode)),
            ),
            Command::UndoQuick => Some(
                session
                    .as_mut()
                    .map_err(|e| e.clone())
                    .and_then(|s| s.undo_quick()),
            ),
            Command::Refresh => {
                let result = if let Ok(connected) = &mut session {
                    connected.reconnect()
                } else {
                    session = crate::asus::Session::connect();
                    session.as_ref().map(|_| ()).map_err(Clone::clone)
                };
                Some(result.map(|_| "Fan readings refreshed.".into()))
            }
            Command::Stop => unreachable!(),
        };
        let result = session
            .as_mut()
            .map_err(|e| e.clone())
            .and_then(|s| s.reading());
        outgoing.publish(Event { result, action });
        wake();
        loop {
            match incoming.recv_timeout(Duration::from_secs(2)) {
                Ok(next) => {
                    command = next;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let result = session
                        .as_mut()
                        .map_err(|e| e.clone())
                        .and_then(|s| s.reading());
                    outgoing.publish(Event {
                        result,
                        action: None,
                    });
                    wake();
                }
            }
        }
    }
    // Session::drop restores unchanged OpenCrate curves on every exit path.
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hidden_window_retains_acknowledgement_and_only_latest_reading() {
        let mailbox = Mailbox::default();
        mailbox.publish(Event {
            result: Ok(Snapshot::default()),
            action: Some(Ok("Applied".into())),
        });
        for duty in 0..=255 {
            let mut f = fan();
            f.duty = duty;
            mailbox.publish(Event {
                result: Ok(Snapshot {
                    fans: vec![f],
                    can_undo: true,
                }),
                action: None,
            });
        }
        let event = mailbox.take().unwrap();
        assert_eq!(event.action, Some(Ok("Applied".into())));
        let reading = event.result.unwrap();
        assert_eq!(reading.fans[0].duty, 255);
        assert!(reading.can_undo);
        assert!(mailbox.take().is_none());
    }
    #[test]
    fn telemetry_failure_does_not_discard_apply_failure() {
        let mailbox = Mailbox::default();
        mailbox.publish(Event {
            result: Ok(Snapshot::default()),
            action: Some(Err("Rejected curve".into())),
        });
        mailbox.publish(Event {
            result: Err("Service disconnected".into()),
            action: None,
        });
        let event = mailbox.take().unwrap();
        assert_eq!(event.action, Some(Err("Rejected curve".into())));
        assert_eq!(event.result.unwrap_err(), "Service disconnected");
    }
    fn fan() -> Fan {
        Fan {
            id: 1,
            name: "CPU".into(),
            duty: 102,
            minimum: 102,
            curve: vec![
                Point {
                    temperature: 50,
                    duty: 102,
                },
                Point {
                    temperature: 70,
                    duty: 153,
                },
                Point {
                    temperature: 80,
                    duty: 204,
                },
                Point {
                    temperature: 85,
                    duty: 255,
                },
            ],
            profiles: vec![],
            can_restore: false,
            writable: true,
        }
    }
    #[test]
    fn percentage_conversion_is_not_raw_bytes() {
        assert_eq!(raw_duty(100), Ok(255));
        assert_eq!(raw_duty(40), Ok(102));
        assert_eq!(percent(153), 60.0);
        assert!(raw_duty(101).is_err());
    }
    #[test]
    fn manual_has_a_service_executed_thermal_ramp() {
        let f = fan();
        let c = manual_curve(&f, 60).unwrap();
        assert_eq!(c[0].duty, 153);
        assert_eq!(
            c.last(),
            Some(&Point {
                temperature: 85,
                duty: 255
            })
        );
        assert!(manual_curve(&f, 39).is_err());
    }
    #[test]
    fn rejects_unsafe_or_malformed_custom_curves() {
        let f = fan();
        let mut c = f.curve.clone();
        c[3].duty = 254;
        assert!(validate_custom(&c, &f).is_err());
        c = f.curve.clone();
        c[2].temperature = 90;
        assert!(validate_custom(&c, &f).is_err());
        c = f.curve.clone();
        c[1].duty = 100;
        assert!(validate_custom(&c, &f).is_err());
        c.pop();
        assert!(validate_custom(&c, &f).is_err());
    }
    #[test]
    fn respects_an_earlier_critical_temperature() {
        let mut f = fan();
        f.curve[3].temperature = 80;
        assert_eq!(manual_curve(&f, 60).unwrap()[3].temperature, 80);
    }
}
