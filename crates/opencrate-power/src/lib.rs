//! Windows power plans and processor policies. No driver or firmware writes.
//! Changes persist in Windows; opening or closing a session never writes settings.

pub mod service;
#[cfg(windows)]
pub mod windows;

pub type PlanId = u128;
pub const BALANCED: PlanId = 0x381b4222_f694_41f0_9685_ff5bb260df2e;
pub const POWER_SAVER: PlanId = 0xa1841308_3541_4fab_bc81_f71556f20b4a;
pub const HIGH_PERFORMANCE: PlanId = 0x8c5e7fda_e8bf_4a96_9a85_a6e23a8c635c;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Ac,
    Dc,
}
impl Source {
    pub fn index(self) -> usize {
        if self == Self::Ac {
            0
        } else {
            1
        }
    }
    pub fn label(self) -> &'static str {
        if self == Self::Ac {
            "Plugged in"
        } else {
            "On battery"
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Minimum,
    Maximum,
    Boost,
    EnergyPreference,
}
impl Setting {
    pub const ALL: [Self; 4] = [
        Self::Minimum,
        Self::Maximum,
        Self::Boost,
        Self::EnergyPreference,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Minimum => "Minimum processor state",
            Self::Maximum => "Maximum processor state",
            Self::Boost => "CPU boost mode",
            Self::EnergyPreference => "Energy preference",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Allowed {
    Range { min: u32, max: u32, step: u32 },
    Choices(Vec<(u32, String)>),
}
impl Allowed {
    pub fn contains(&self, value: u32) -> bool {
        match self {
            Self::Range { min, max, step } => {
                value >= *min && value <= *max && *step > 0 && (value - min).is_multiple_of(*step)
            }
            Self::Choices(values) => values.iter().any(|(v, _)| *v == value),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Control {
    pub key: Setting,
    pub value: u32,
    pub allowed: Allowed,
    pub write_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub id: PlanId,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct CpuSettings {
    pub controls: Vec<Control>,
    pub unavailable: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub plans: Vec<Plan>,
    pub active: PlanId,
    pub cpu: [CpuSettings; 2],
    pub source: Option<Source>,
    pub battery_percent: Option<u8>,
    pub can_undo: bool,
}
impl Snapshot {
    pub fn active_name(&self) -> &str {
        self.plans
            .iter()
            .find(|p| p.id == self.active)
            .map_or("Unknown plan", |p| p.name.as_str())
    }
}

pub trait Backend {
    fn snapshot(&mut self) -> Result<Snapshot, String>;
    fn active(&mut self) -> Result<PlanId, String>;
    fn activate(&mut self, plan: PlanId) -> Result<(), String>;
    fn read(&mut self, plan: PlanId, source: Source, key: Setting) -> Result<u32, String>;
    fn write(
        &mut self,
        plan: PlanId,
        source: Source,
        key: Setting,
        value: u32,
    ) -> Result<(), String>;
}

#[derive(Clone, Debug)]
pub struct Edit {
    pub plan: PlanId,
    pub source: Source,
    /// Values displayed when editing began, used to reject stale drafts.
    pub expected: Vec<(Setting, u32)>,
    pub values: Vec<(Setting, u32)>,
}

pub fn values(controls: &[Control]) -> Vec<(Setting, u32)> {
    controls.iter().map(|c| (c.key, c.value)).collect()
}

fn check_values(controls: &[Control], proposed: &[(Setting, u32)]) -> Result<(), String> {
    for (i, (key, value)) in proposed.iter().enumerate() {
        if proposed[..i].iter().any(|(k, _)| k == key) {
            return Err("Duplicate processor setting.".into());
        }
        let control = controls
            .iter()
            .find(|c| c.key == *key)
            .ok_or_else(|| format!("{} is unavailable.", key.label()))?;
        if *value != control.value {
            if let Some(error) = &control.write_error {
                return Err(error.clone());
            }
            if !control.allowed.contains(*value) {
                return Err(format!("{} is outside the supported range.", key.label()));
            }
        }
    }
    let value = |key| {
        proposed
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
            .or_else(|| controls.iter().find(|c| c.key == key).map(|c| c.value))
    };
    if let (Some(min), Some(max)) = (value(Setting::Minimum), value(Setting::Maximum)) {
        if min > max {
            return Err("Minimum processor state must not exceed maximum.".into());
        }
    }
    Ok(())
}

pub fn validate(controls: &[Control], edit: &Edit) -> Result<(), String> {
    if values(controls) != edit.expected {
        return Err(
            "Power settings changed outside this editor. Reload current values before applying."
                .into(),
        );
    }
    check_values(controls, &edit.values)
}

#[derive(Clone, Debug)]
struct Change {
    key: Setting,
    before: u32,
    after: u32,
}
#[derive(Clone, Debug)]
enum Undo {
    Plan {
        before: PlanId,
        after: PlanId,
    },
    Settings {
        plan: PlanId,
        source: Source,
        changes: Vec<Change>,
    },
}

pub struct Session<B: Backend> {
    backend: B,
    undo: Option<Undo>,
}
impl<B: Backend> Session<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            undo: None,
        }
    }
    pub fn snapshot(&mut self) -> Result<Snapshot, String> {
        let mut snapshot = self.backend.snapshot()?;
        snapshot.can_undo = self.undo_matches(&snapshot);
        // Once an external change has been observed, never revive a stale undo.
        if !snapshot.can_undo {
            self.undo = None;
        }
        Ok(snapshot)
    }
    fn undo_matches(&self, snapshot: &Snapshot) -> bool {
        match &self.undo {
            None => false,
            Some(Undo::Plan { before, after }) => {
                snapshot.active == *after && snapshot.plans.iter().any(|p| p.id == *before)
            }
            Some(Undo::Settings {
                plan,
                source,
                changes,
            }) => {
                let controls = &snapshot.cpu[source.index()].controls;
                snapshot.active == *plan
                    && changes.iter().all(|change| {
                        controls
                            .iter()
                            .any(|c| c.key == change.key && c.value == change.after)
                    })
                    && check_values(
                        controls,
                        &changes
                            .iter()
                            .map(|c| (c.key, c.before))
                            .collect::<Vec<_>>(),
                    )
                    .is_ok()
            }
        }
    }
    pub fn activate(&mut self, expected: PlanId, plan: PlanId) -> Result<String, String> {
        let snapshot = self.snapshot()?;
        if snapshot.active != expected {
            return Err("The active Windows plan changed. Refresh and try again.".into());
        }
        let name = snapshot
            .plans
            .iter()
            .find(|p| p.id == plan)
            .ok_or("This power plan is no longer available.")?
            .name
            .clone();
        if plan == expected {
            return Ok(format!("{name} is already active."));
        }
        self.switch(expected, plan)?;
        self.undo = Some(Undo::Plan {
            before: expected,
            after: plan,
        });
        Ok(format!("{name} is now active."))
    }
    fn switch(&mut self, before: PlanId, after: PlanId) -> Result<(), String> {
        let result = self.backend.activate(after).and_then(|_| {
            if self.backend.active()? == after {
                Ok(())
            } else {
                Err("Windows did not activate the requested plan.".into())
            }
        });
        if let Err(error) = result {
            // A failed call may have switched the plan before returning an error.
            let recovery = match self.backend.active() {
                Ok(id) if id == before => Ok(()),
                Ok(id) if id == after => self.backend.activate(before).and_then(|_| {
                    if self.backend.active()? == before {
                        Ok(())
                    } else {
                        Err("Previous plan readback failed.".into())
                    }
                }),
                _ => Err("The active plan changed externally; it was left in place.".into()),
            };
            return Err(match recovery {
                Ok(()) => format!("{error} Previous plan preserved."),
                Err(e) => {
                    self.undo = None;
                    format!("{error} Recovery: {e}")
                }
            });
        }
        Ok(())
    }
    pub fn apply(&mut self, edit: Edit) -> Result<String, String> {
        let snapshot = self.snapshot()?;
        if snapshot.active != edit.plan {
            return Err("The active plan changed. Reload current values before applying.".into());
        }
        let controls = &snapshot.cpu[edit.source.index()].controls;
        validate(controls, &edit)?;
        let changes: Vec<_> = edit
            .values
            .iter()
            .filter_map(|(key, after)| {
                let before = controls.iter().find(|c| c.key == *key)?.value;
                (before != *after).then_some(Change {
                    key: *key,
                    before,
                    after: *after,
                })
            })
            .collect();
        if changes.is_empty() {
            return Ok("Processor settings are already applied.".into());
        }
        self.apply_changes(edit.plan, edit.source, &changes)?;
        self.undo = Some(Undo::Settings {
            plan: edit.plan,
            source: edit.source,
            changes,
        });
        Ok(format!(
            "Processor settings applied to {} · {}.",
            snapshot.active_name(),
            edit.source.label()
        ))
    }
    fn apply_changes(
        &mut self,
        plan: PlanId,
        source: Source,
        changes: &[Change],
    ) -> Result<(), String> {
        let mut ordered = changes.to_vec();
        // Keep min <= max during both forward writes and reverse rollback.
        ordered.sort_by_key(|c| match c.key {
            Setting::Minimum if c.after < c.before => 0,
            Setting::Maximum if c.after > c.before => 0,
            _ => 1,
        });
        let mut attempted = Vec::new();
        let result = (|| {
            for change in &ordered {
                if self.backend.active()? != plan {
                    return Err("The active plan changed during Apply.".into());
                }
                attempted.push(change.clone());
                self.backend.write(plan, source, change.key, change.after)?;
                if self.backend.read(plan, source, change.key)? != change.after {
                    return Err(format!("{} readback did not match.", change.key.label()));
                }
            }
            if self.backend.active()? != plan {
                return Err("The active plan changed before activation.".into());
            }
            self.backend.activate(plan)?;
            if self.backend.active()? != plan {
                return Err("Windows changed the active plan during activation.".into());
            }
            for change in &ordered {
                if self.backend.read(plan, source, change.key)? != change.after {
                    return Err(format!("{} changed during activation.", change.key.label()));
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            let mut errors = Vec::new();
            for change in attempted.iter().rev() {
                let restored: Result<(), String> = (|| {
                    let current = self.backend.read(plan, source, change.key)?;
                    if current == change.before {
                        return Ok(());
                    }
                    if current != change.after {
                        return Err("Changed externally; left in place.".into());
                    }
                    self.backend
                        .write(plan, source, change.key, change.before)?;
                    if self.backend.read(plan, source, change.key)? != change.before {
                        return Err("Restore readback failed.".into());
                    }
                    Ok(())
                })();
                if let Err(e) = restored {
                    errors.push(format!("{}: {e}", change.key.label()));
                }
            }
            // Never switch back over a plan selected by another application.
            match self.backend.active() {
                Ok(active) if active == plan => {
                    if let Err(e) = self.backend.activate(plan) {
                        errors.push(e);
                    }
                }
                Err(e) => errors.push(e),
                _ => {}
            }
            if errors.is_empty() {
                return Err(format!("{error} Previous values restored."));
            }
            self.undo = None;
            return Err(format!(
                "{error} Some values could not be restored: {}",
                errors.join("; ")
            ));
        }
        Ok(())
    }
    pub fn undo(&mut self) -> Result<String, String> {
        let snapshot = self.snapshot()?;
        if !snapshot.can_undo {
            return Err(
                "Nothing to undo, or Windows settings changed since the last action.".into(),
            );
        }
        match self.undo.clone().unwrap() {
            Undo::Plan { before, after } => self.switch(after, before)?,
            Undo::Settings {
                plan,
                source,
                changes,
            } => {
                let reverse = changes
                    .iter()
                    .map(|c| Change {
                        key: c.key,
                        before: c.after,
                        after: c.before,
                    })
                    .collect::<Vec<_>>();
                self.apply_changes(plan, source, &reverse)?;
            }
        }
        self.undo = None;
        Ok("Previous power settings restored.".into())
    }
}

#[cfg(test)]
mod tests;
