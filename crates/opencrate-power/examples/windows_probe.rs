//! Read-only by default. --test-roundtrip temporarily changes policies and undoes them.
#[cfg(windows)]
fn main() -> Result<(), String> {
    use opencrate_power::{windows::Windows, *};
    struct Guard(Session<Windows>);
    impl Drop for Guard {
        fn drop(&mut self) {
            if self.0.snapshot().is_ok_and(|s| s.can_undo) {
                if let Err(error) = self.0.undo() {
                    eprintln!("Power test cleanup: {error}");
                }
            }
        }
    }
    let mut session = Guard(Session::new(Windows));
    let original = session.0.snapshot()?;
    println!(
        "Active: {} ({:032x}), source: {:?}",
        original.active_name(),
        original.active,
        original.source
    );
    for plan in &original.plans {
        println!("Plan: {} ({:032x})", plan.name, plan.id);
    }
    for source in [Source::Ac, Source::Dc] {
        println!("{}: {:#?}", source.label(), original.cpu[source.index()]);
    }
    if !std::env::args().any(|a| a == "--test-roundtrip") {
        return Ok(());
    }
    // Capture all exposed fields in every installed plan; compare again at the end.
    let mut native = Windows;
    let mut baseline = Vec::new();
    for plan in &original.plans {
        for source in [Source::Ac, Source::Dc] {
            for key in Setting::ALL {
                if let Ok(value) = native.read(plan.id, source, key) {
                    baseline.push((plan.id, source, key, value));
                }
            }
        }
    }
    for plan in &original.plans {
        if plan.id == original.active {
            continue;
        }
        println!("{}", session.0.activate(original.active, plan.id)?);
        if session.0.snapshot()?.active != plan.id {
            return Err("Plan activation readback failed.".into());
        }
        println!("{}", session.0.undo()?);
        if session.0.snapshot()?.active != original.active {
            return Err("Plan undo readback failed.".into());
        }
    }
    for source in [Source::Ac, Source::Dc] {
        let snapshot = session.0.snapshot()?;
        let controls = &snapshot.cpu[source.index()].controls;
        let proposed: Vec<_> = controls
            .iter()
            .map(|c| {
                (
                    c.key,
                    match c.key {
                        Setting::Minimum => 5,
                        Setting::Maximum => 90,
                        Setting::Boost => 0,
                        Setting::EnergyPreference => 60,
                    },
                )
            })
            .collect();
        let edit = Edit {
            plan: original.active,
            source,
            expected: values(controls),
            values: proposed.clone(),
        };
        println!("{}", session.0.apply(edit)?);
        let applied = session.0.snapshot()?;
        for (key, value) in proposed {
            if !applied.cpu[source.index()]
                .controls
                .iter()
                .any(|c| c.key == key && c.value == value)
            {
                return Err("Processor setting readback failed.".into());
            }
        }
        let other = if source == Source::Ac {
            Source::Dc
        } else {
            Source::Ac
        };
        if values(&applied.cpu[other.index()].controls)
            != values(&original.cpu[other.index()].controls)
        {
            return Err("Unexpected change to other power source.".into());
        }
        println!("{}", session.0.undo()?);
    }
    for (plan, source, key, value) in baseline {
        if native.read(plan, source, key)? != value {
            return Err(format!("Baseline mismatch: {plan:032x} {source:?} {key:?}"));
        }
    }
    if native.active()? != original.active {
        return Err("Original plan was not restored.".into());
    }
    println!(
        "PASS: every installed plan and all AC/DC processor values match the original baseline."
    );
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("This diagnostic requires Windows.");
}
