//! Explicit manual diagnostic: temporarily writes all fan headers and restores
//! the exact pre-test curves. It is never run by cargo test.
#[cfg(windows)]
fn main() -> Result<(), String> {
    use opencrate_fan::{asus::Session, quick::QuickMode, service::Fan};
    fn same_curves(expected: &[Fan], actual: &[Fan]) -> Result<(), String> {
        for fan in expected {
            if actual
                .iter()
                .find(|f| f.id == fan.id)
                .is_none_or(|f| f.curve != fan.curve)
            {
                return Err(format!("{} curve differs from its baseline", fan.name));
            }
        }
        Ok(())
    }
    let mut session = Session::connect()?;
    let baseline = session.snapshot()?;
    if !std::env::args().any(|s| s == "--test-all") {
        println!("{} fans detected. Pass --test-all to test Full Blast, profiles, and Undo with real hardware writes.", baseline.len());
        return Ok(());
    }
    for mode in [
        QuickMode::FullBlast,
        QuickMode::Standard,
        QuickMode::Silent,
        QuickMode::Turbo,
    ] {
        println!("{}", session.apply_quick(mode)?);
        std::thread::sleep(std::time::Duration::from_secs(2));
        let during = session.snapshot()?;
        let check = if mode == QuickMode::FullBlast {
            if during
                .iter()
                .all(|f| f.duty == 255 && f.curve.iter().all(|p| p.duty == 255))
            {
                Ok(())
            } else {
                Err("Full Blast readback was not 100% on every fan".into())
            }
        } else {
            if during.iter().all(|f| {
                f.profiles
                    .iter()
                    .any(|p| p.name == mode.label() && p.curve == f.curve)
            }) {
                Ok(())
            } else {
                Err(format!("{} profile readback mismatch", mode.label()))
            }
        };
        if session.can_undo_quick(&during) {
            println!("{}", session.undo_quick()?);
        }
        same_curves(&baseline, &session.snapshot()?)?;
        check?;
        println!(
            "{} and exact Undo verified for all {} fans.",
            mode.label(),
            during.len()
        );
    }
    // Undo Full Blast must return to a preceding quick profile, not startup.
    session.apply_quick(QuickMode::Turbo)?;
    let turbo = session.snapshot()?;
    session.apply_quick(QuickMode::FullBlast)?;
    session.apply_quick(QuickMode::FullBlast)?; // no-op must preserve undo
    session.undo_quick()?;
    same_curves(&turbo, &session.snapshot()?)?;
    drop(session);
    let session = Session::connect()?;
    same_curves(&baseline, &session.snapshot()?)?;
    println!(
        "Repeated Full Blast, return to prior profile, and Quit baseline restoration verified."
    );
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    eprintln!("This diagnostic requires Windows.");
}
