//! Read-only diagnostic: observe CPU samples for ten seconds without changing settings.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let monitor = opencrate_power::telemetry::Monitor::start(|| {})?;
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_secs(1));
        if let Some(reading) = monitor.take() {
            println!("{reading:#?}");
        }
    }
    Ok(())
}
