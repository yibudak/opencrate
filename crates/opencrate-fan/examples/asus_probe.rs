//! Read-only by default. --test-full briefly applies 100% to the first chassis
//! fan, checks readback, and restores its original curve before exiting.
#[cfg(windows)]
fn main() -> Result<(), String> {
    use opencrate_fan::{
        asus::Session,
        service::{manual_curve, Target},
    };
    let mut session = Session::connect()?;
    let fans = session.snapshot()?;
    for fan in &fans {
        println!(
            "{} [{}]: raw duty {}, minimum {}, curve {:?}",
            fan.name, fan.id, fan.duty, fan.minimum, fan.curve
        );
    }
    if std::env::args().any(|s| s == "--test-full") {
        let fan = fans
            .iter()
            .find(|f| f.name == "Chassis Fan 1")
            .ok_or("Test fan not found")?;
        let expected = manual_curve(fan, 100)?;
        let test = session.apply(fan.id, Target::Custom(expected.clone()));
        std::thread::sleep(std::time::Duration::from_secs(3));
        let during = session.snapshot();
        let restored = session.apply(fan.id, Target::Restore);
        println!("Apply: {test:?}\nRestore: {restored:?}");
        test?;
        restored?;
        let during = during?;
        let active = during
            .iter()
            .find(|f| f.id == fan.id)
            .ok_or("Fan disappeared")?;
        if active.duty != 255 || active.curve != expected {
            return Err("Full-speed readback did not match; original was restored".into());
        }
        let after = session.snapshot()?;
        for original in &fans {
            if after
                .iter()
                .find(|f| f.id == original.id)
                .ok_or("Fan disappeared")?
                .curve
                != original.curve
            {
                return Err(format!("{} curve changed after restore", original.name));
            }
        }
        println!("Full-speed duty/curve and all original curves verified.");
        // Exercise the same RAII restoration used by the worker on Quit.
        session.apply(fan.id, Target::Custom(expected))?;
        drop(session);
        let session = Session::connect()?;
        let after = session.snapshot()?;
        if after
            .iter()
            .find(|f| f.id == fan.id)
            .ok_or("Fan disappeared")?
            .curve
            != fan.curve
        {
            return Err("Session shutdown did not restore the original curve".into());
        }
        println!("Session shutdown restore verified.");
    }
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    eprintln!("This probe requires Windows.");
}
