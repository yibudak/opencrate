//! Serialized background access, with a bounded latest-reading mailbox for tray use.
use crate::{Edit, PlanId, Snapshot};
use std::{
    sync::{mpsc, Arc, Mutex},
    thread,
};

pub enum Command {
    Refresh,
    Activate { expected: PlanId, plan: PlanId },
    ActivateUltimate { expected: PlanId },
    Apply(Edit),
    Undo,
    Stop,
}
pub struct Event {
    pub result: Result<Snapshot, String>,
    pub action: Option<Result<String, String>>,
}
#[derive(Default)]
struct Mailbox(Mutex<Option<Event>>);
impl Mailbox {
    fn publish(&self, mut event: Event) {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if event.action.is_none() {
            event.action = slot.take().and_then(|e| e.action);
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
            .name("opencrate-power".into())
            .spawn(move || {
                #[cfg(windows)]
                {
                    let mut session = crate::Session::new(crate::windows::Windows);
                    let mut command = Some(Command::Refresh);
                    loop {
                        let action = match command {
                            Some(Command::Stop) => break,
                            Some(Command::Refresh) => {
                                Some(Ok("Windows power settings refreshed.".into()))
                            }
                            Some(Command::Activate { expected, plan }) => {
                                Some(session.activate(expected, plan))
                            }
                            Some(Command::ActivateUltimate { expected }) => {
                                Some(session.activate_ultimate(expected))
                            }
                            Some(Command::Apply(edit)) => Some(session.apply(edit)),
                            Some(Command::Undo) => Some(session.undo()),
                            None => None,
                        };
                        outgoing.publish(Event {
                            result: session.snapshot(),
                            action,
                        });
                        wake();
                        command = match incoming.recv_timeout(std::time::Duration::from_secs(3)) {
                            Ok(c) => Some(c),
                            Err(mpsc::RecvTimeoutError::Timeout) => None,
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        };
                    }
                }
                #[cfg(not(windows))]
                {
                    let _ = incoming;
                    outgoing.publish(Event {
                        result: Err("Power control requires Windows.".into()),
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
            .map_err(|_| "Power worker is unavailable.".into())
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
