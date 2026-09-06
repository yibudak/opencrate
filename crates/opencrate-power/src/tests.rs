use super::*;

struct Fake {
    snapshot: Snapshot,
    writes: Vec<(Source, Setting, u32)>,
    activations: Vec<PlanId>,
    fail_writes: Vec<usize>,
    fail_activation: bool,
    ignore_write: bool,
}
impl Fake {
    fn new() -> Self {
        let controls = Setting::ALL
            .into_iter()
            .zip([5, 100, 2, 25])
            .map(|(key, value)| Control {
                key,
                value,
                write_error: None,
                allowed: if key == Setting::Boost {
                    Allowed::Choices(vec![(0, "Disabled".into()), (2, "Aggressive".into())])
                } else {
                    Allowed::Range {
                        min: 0,
                        max: 100,
                        step: 1,
                    }
                },
            })
            .collect();
        let cpu = CpuSettings {
            controls,
            unavailable: vec![],
        };
        Self {
            snapshot: Snapshot {
                plans: vec![
                    Plan {
                        id: BALANCED,
                        name: "Dengeli".into(),
                    },
                    Plan {
                        id: HIGH_PERFORMANCE,
                        name: "Performance".into(),
                    },
                ],
                active: BALANCED,
                cpu: [cpu.clone(), cpu],
                source: Some(Source::Ac),
                battery_percent: None,
                can_undo: false,
            },
            writes: vec![],
            activations: vec![],
            fail_writes: vec![],
            fail_activation: false,
            ignore_write: false,
        }
    }
}
impl Backend for Fake {
    fn snapshot(&mut self) -> Result<Snapshot, String> {
        Ok(self.snapshot.clone())
    }
    fn active(&mut self) -> Result<PlanId, String> {
        Ok(self.snapshot.active)
    }
    fn activate(&mut self, plan: PlanId) -> Result<(), String> {
        self.activations.push(plan);
        self.snapshot.active = plan;
        if std::mem::take(&mut self.fail_activation) {
            Err("Activation failure".into())
        } else {
            Ok(())
        }
    }
    fn read(&mut self, _: PlanId, source: Source, key: Setting) -> Result<u32, String> {
        self.snapshot.cpu[source.index()]
            .controls
            .iter()
            .find(|c| c.key == key)
            .map(|c| c.value)
            .ok_or("Missing setting".into())
    }
    fn write(&mut self, _: PlanId, source: Source, key: Setting, value: u32) -> Result<(), String> {
        self.writes.push((source, key, value));
        if !std::mem::take(&mut self.ignore_write) {
            self.snapshot.cpu[source.index()]
                .controls
                .iter_mut()
                .find(|c| c.key == key)
                .unwrap()
                .value = value;
        }
        // Model a setter that mutates before returning an error.
        if self.fail_writes.contains(&self.writes.len()) {
            Err("Write failure".into())
        } else {
            Ok(())
        }
    }
}
fn edit(session: &mut Session<Fake>, source: Source, changes: &[(Setting, u32)]) -> Edit {
    let snapshot = session.snapshot().unwrap();
    Edit {
        plan: snapshot.active,
        source,
        expected: values(&snapshot.cpu[source.index()].controls),
        values: changes.to_vec(),
    }
}

#[test]
fn reads_and_noop_actions_never_write_or_replace_undo() {
    let mut session = Session::new(Fake::new());
    let noop = edit(&mut session, Source::Ac, &[(Setting::Maximum, 100)]);
    session.apply(noop).unwrap();
    session.activate(BALANCED, BALANCED).unwrap();
    assert!(session.backend.writes.is_empty());
    assert!(session.backend.activations.is_empty());
    session.activate(BALANCED, HIGH_PERFORMANCE).unwrap();
    session
        .activate(HIGH_PERFORMANCE, HIGH_PERFORMANCE)
        .unwrap();
    session.undo().unwrap();
    assert_eq!(session.backend.snapshot.active, BALANCED);
}

#[test]
fn rejects_invalid_stale_unsupported_and_readonly_edits_before_any_write() {
    let mut session = Session::new(Fake::new());
    for changes in [
        vec![(Setting::Maximum, 101)],
        vec![(Setting::Minimum, 95), (Setting::Maximum, 90)],
        vec![(Setting::Boost, 7)],
        vec![(Setting::Maximum, 90), (Setting::Maximum, 80)],
    ] {
        let edit = edit(&mut session, Source::Ac, &changes);
        assert!(session.apply(edit).is_err());
    }
    let stale = edit(&mut session, Source::Ac, &[(Setting::Maximum, 90)]);
    session.backend.snapshot.cpu[0].controls[0].value = 10;
    assert!(session.apply(stale).is_err());
    session.backend.snapshot.cpu[0].controls[1].write_error = Some("Managed by policy".into());
    let blocked = edit(&mut session, Source::Ac, &[(Setting::Maximum, 90)]);
    assert!(session.apply(blocked).is_err());
    assert!(session.backend.writes.is_empty());
}

#[test]
fn source_isolation_apply_and_undo_restore_exact_values() {
    let mut session = Session::new(Fake::new());
    let before = session.backend.snapshot.cpu.clone();
    let edit = edit(
        &mut session,
        Source::Dc,
        &[
            (Setting::Maximum, 80),
            (Setting::Boost, 0),
            (Setting::EnergyPreference, 70),
        ],
    );
    session.apply(edit).unwrap();
    assert_eq!(
        values(&session.backend.snapshot.cpu[0].controls),
        values(&before[0].controls)
    );
    assert!(session.snapshot().unwrap().can_undo);
    session.undo().unwrap();
    assert_eq!(
        values(&session.backend.snapshot.cpu[1].controls),
        values(&before[1].controls)
    );
    assert!(!session.snapshot().unwrap().can_undo);
    assert_eq!(session.backend.activations, [BALANCED, BALANCED]);
}

#[test]
fn partial_failure_restores_failing_setting_then_prior_settings() {
    let mut session = Session::new(Fake::new());
    session.backend.fail_writes = vec![2];
    let edit = edit(
        &mut session,
        Source::Ac,
        &[
            (Setting::Maximum, 90),
            (Setting::Boost, 0),
            (Setting::EnergyPreference, 50),
        ],
    );
    assert!(session
        .apply(edit)
        .unwrap_err()
        .contains("Previous values restored"));
    assert_eq!(
        session.backend.writes,
        [
            (Source::Ac, Setting::Maximum, 90),
            (Source::Ac, Setting::Boost, 0),
            (Source::Ac, Setting::Boost, 2),
            (Source::Ac, Setting::Maximum, 100)
        ]
    );
    assert_eq!(
        values(&session.backend.snapshot.cpu[0].controls),
        values(&Fake::new().snapshot.cpu[0].controls)
    );
}

#[test]
fn rollback_failure_does_not_skip_remaining_restore_attempts() {
    let mut session = Session::new(Fake::new());
    session.backend.fail_writes = vec![2, 3];
    let edit = edit(
        &mut session,
        Source::Ac,
        &[(Setting::Maximum, 90), (Setting::Boost, 0)],
    );
    assert!(session
        .apply(edit)
        .unwrap_err()
        .contains("could not be restored"));
    assert_eq!(session.backend.writes.len(), 4);
    assert!(!session.snapshot().unwrap().can_undo);
}

#[test]
fn activation_failure_rolls_back_all_values_and_plan_selection() {
    let mut session = Session::new(Fake::new());
    session.backend.fail_activation = true;
    let edit = edit(&mut session, Source::Ac, &[(Setting::Maximum, 90)]);
    assert!(session.apply(edit).is_err());
    assert_eq!(
        session
            .backend
            .read(BALANCED, Source::Ac, Setting::Maximum)
            .unwrap(),
        100
    );
    session.backend.fail_activation = true;
    assert!(session.activate(BALANCED, HIGH_PERFORMANCE).is_err());
    assert_eq!(session.backend.snapshot.active, BALANCED);
}

#[test]
fn mismatched_readback_is_not_reported_as_success() {
    let mut session = Session::new(Fake::new());
    session.backend.ignore_write = true;
    let edit = edit(&mut session, Source::Ac, &[(Setting::Maximum, 90)]);
    assert!(session.apply(edit).unwrap_err().contains("readback"));
    assert!(!session.snapshot().unwrap().can_undo);
}

#[test]
fn external_changes_invalidate_undo_without_overwriting_them() {
    let mut session = Session::new(Fake::new());
    let edit = edit(&mut session, Source::Ac, &[(Setting::Maximum, 90)]);
    session.apply(edit).unwrap();
    session.backend.snapshot.cpu[0].controls[1].value = 80;
    assert!(!session.snapshot().unwrap().can_undo);
    let count = session.backend.writes.len();
    assert!(session.undo().is_err());
    assert_eq!(session.backend.writes.len(), count);
    // Returning to the previous value must not revive stale undo ownership.
    session.backend.snapshot.cpu[0].controls[1].value = 90;
    assert!(!session.snapshot().unwrap().can_undo);
}

#[test]
fn unrelated_external_setting_is_preserved_by_undo() {
    let mut session = Session::new(Fake::new());
    let edit = edit(&mut session, Source::Ac, &[(Setting::Maximum, 90)]);
    session.apply(edit).unwrap();
    session.backend.snapshot.cpu[0].controls[3].value = 70;
    session.undo().unwrap();
    assert_eq!(
        session
            .backend
            .read(BALANCED, Source::Ac, Setting::EnergyPreference)
            .unwrap(),
        70
    );
}

#[test]
fn changing_ranges_uses_an_order_valid_for_intermediate_states() {
    let mut session = Session::new(Fake::new());
    session.backend.snapshot.cpu[0].controls[0].value = 100;
    let edit = edit(
        &mut session,
        Source::Ac,
        &[(Setting::Maximum, 80), (Setting::Minimum, 5)],
    );
    session.apply(edit).unwrap();
    assert_eq!(session.backend.writes[0], (Source::Ac, Setting::Minimum, 5));
    session.undo().unwrap();
    assert_eq!(
        session.backend.writes[2],
        (Source::Ac, Setting::Maximum, 100)
    );
}

#[test]
fn external_plan_changes_reject_old_drafts_and_undo() {
    let mut session = Session::new(Fake::new());
    let edit = edit(&mut session, Source::Ac, &[(Setting::Maximum, 90)]);
    session.backend.snapshot.active = HIGH_PERFORMANCE;
    assert!(session.apply(edit).is_err());
    assert!(session.activate(BALANCED, HIGH_PERFORMANCE).is_err());
    assert!(session.backend.writes.is_empty());
    session.activate(HIGH_PERFORMANCE, BALANCED).unwrap();
    session.backend.snapshot.active = HIGH_PERFORMANCE;
    assert!(session.undo().is_err());
}

#[test]
fn range_increment_and_unknown_settings_are_validated() {
    assert!(!Allowed::Range {
        min: 10,
        max: 90,
        step: 10
    }
    .contains(15));
    assert!(Allowed::Range {
        min: 10,
        max: 90,
        step: 10
    }
    .contains(20));
    let mut session = Session::new(Fake::new());
    session.backend.snapshot.cpu[0]
        .controls
        .retain(|c| c.key != Setting::Boost);
    let edit = edit(&mut session, Source::Ac, &[(Setting::Boost, 0)]);
    assert!(session.apply(edit).is_err());
    assert!(session.backend.writes.is_empty());
}
