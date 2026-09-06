use opencrate_aura::{AuraDevice, DIRECT_LED_COUNT};
use opencrate_core::RgbColor;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let hex = std::env::args().nth(1).unwrap_or_else(|| "0000FF".into());
    let color: RgbColor = hex.parse()?;
    let device = AuraDevice::open()?;
    device.start_direct()?;
    device.write_frame(&[color; DIRECT_LED_COUNT])?;
    println!("Sent direct color {} to all lighting channels", color);
    Ok(())
}
