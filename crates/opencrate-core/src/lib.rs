//! Shared types for opencrate: colors, lighting effects, channels, fan curves.
//!
//! These are hardware-agnostic primitives. Protocol-specific packet layout
//! lives in the driver crates (`opencrate-aura`, ...).

use std::fmt;
use std::str::FromStr;

/// An 8-bit per channel RGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RgbColor {
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0 };

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parse `RRGGBB` or `#RRGGBB`.
    pub fn from_hex(s: &str) -> Result<Self, CoreError> {
        let s = s.strip_prefix('#').unwrap_or(s);
        if s.len() != 6 {
            return Err(CoreError::InvalidHex(s.to_string()));
        }
        let v = u32::from_str_radix(s, 16).map_err(|_| CoreError::InvalidHex(s.to_string()))?;
        Ok(Self {
            r: ((v >> 16) & 0xFF) as u8,
            g: ((v >> 8) & 0xFF) as u8,
            b: (v & 0xFF) as u8,
        })
    }

    pub fn to_hex(&self) -> String {
        format!("{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

impl FromStr for RgbColor {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s)
    }
}

impl fmt::Display for RgbColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.to_hex())
    }
}

/// Lighting effect supported by ASUS Aura USB controllers in effect mode.
///
/// Numeric codes match the publicly documented Aura USB effect-mode table
/// (see e.g. OpenRGB wiki "ASUS Aura USB" and the liquidctl `aura_led`
/// driver docs). Codes are protocol facts, reimplemented here from scratch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectMode {
    Off,
    Static,
    Breathing,
    Flashing,
    SpectrumCycle,
    Rainbow,
    SpectrumCycleBreathing,
    ChaseFade,
    SpectrumCycleChaseFade,
    Chase,
    SpectrumCycleChase,
    SpectrumCycleWave,
    ChaseRainbowPulse,
    RainbowFlicker,
    GentleTransition,
    WavePropagation,
    WavePropagationPause,
}

impl EffectMode {
    pub fn code(self) -> u8 {
        match self {
            Self::Off => 0x00,
            Self::Static => 0x01,
            Self::Breathing => 0x02,
            Self::Flashing => 0x03,
            Self::SpectrumCycle => 0x04,
            Self::Rainbow => 0x05,
            Self::SpectrumCycleBreathing => 0x06,
            Self::ChaseFade => 0x07,
            Self::SpectrumCycleChaseFade => 0x08,
            Self::Chase => 0x09,
            Self::SpectrumCycleChase => 0x0A,
            Self::SpectrumCycleWave => 0x0B,
            Self::ChaseRainbowPulse => 0x0C,
            Self::RainbowFlicker => 0x0D,
            Self::GentleTransition => 0x10,
            Self::WavePropagation => 0x11,
            Self::WavePropagationPause => 0x12,
        }
    }

    /// Whether this mode takes a color argument.
    pub fn takes_color(self) -> bool {
        matches!(
            self,
            Self::Static | Self::Breathing | Self::Flashing | Self::ChaseFade | Self::Chase
        )
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Static => "static",
            Self::Breathing => "breathing",
            Self::Flashing => "flashing",
            Self::SpectrumCycle => "spectrum_cycle",
            Self::Rainbow => "rainbow",
            Self::SpectrumCycleBreathing => "spectrum_cycle_breathing",
            Self::ChaseFade => "chase_fade",
            Self::SpectrumCycleChaseFade => "spectrum_cycle_chase_fade",
            Self::Chase => "chase",
            Self::SpectrumCycleChase => "spectrum_cycle_chase",
            Self::SpectrumCycleWave => "spectrum_cycle_wave",
            Self::ChaseRainbowPulse => "chase_rainbow_pulse",
            Self::RainbowFlicker => "rainbow_flicker",
            Self::GentleTransition => "gentle_transition",
            Self::WavePropagation => "wave_propagation",
            Self::WavePropagationPause => "wave_propagation_pause",
        }
    }

    /// Human-readable name for menus and status messages.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Static => "Static",
            Self::Breathing => "Breathing",
            Self::Flashing => "Flashing",
            Self::SpectrumCycle => "Spectrum Cycle",
            Self::Rainbow => "Rainbow",
            Self::SpectrumCycleBreathing => "Spectrum Cycle Breathing",
            Self::ChaseFade => "Chase Fade",
            Self::SpectrumCycleChaseFade => "Spectrum Cycle Chase Fade",
            Self::Chase => "Chase",
            Self::SpectrumCycleChase => "Spectrum Cycle Chase",
            Self::SpectrumCycleWave => "Spectrum Cycle Wave",
            Self::ChaseRainbowPulse => "Chase Rainbow Pulse",
            Self::RainbowFlicker => "Rainbow Flicker",
            Self::GentleTransition => "Gentle Transition",
            Self::WavePropagation => "Wave Propagation",
            Self::WavePropagationPause => "Wave Propagation Pause",
        }
    }

    pub fn all() -> &'static [Self] {
        use EffectMode::*;
        &[
            Off,
            Static,
            Breathing,
            Flashing,
            SpectrumCycle,
            Rainbow,
            SpectrumCycleBreathing,
            ChaseFade,
            SpectrumCycleChaseFade,
            Chase,
            SpectrumCycleChase,
            SpectrumCycleWave,
            ChaseRainbowPulse,
            RainbowFlicker,
            GentleTransition,
            WavePropagation,
            WavePropagationPause,
        ]
    }
}

impl FromStr for EffectMode {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let norm = s.to_ascii_lowercase().replace('-', "_");
        Self::all()
            .iter()
            .find(|m| m.name() == norm)
            .copied()
            .ok_or_else(|| CoreError::UnknownMode(s.to_string()))
    }
}

/// Logical lighting channel on ASUS desktop boards.
///
/// `led1` is the 12V non-addressable RGB header; `led2..=led4` are the
/// 5V addressable Gen2 channels. Physical header availability is board-dependent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    Led1,
    Led2,
    Led3,
    Led4,
    /// Apply to every channel (synchronized effect).
    Sync,
}

impl FromStr for Channel {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "led1" => Ok(Self::Led1),
            "led2" => Ok(Self::Led2),
            "led3" => Ok(Self::Led3),
            "led4" => Ok(Self::Led4),
            "sync" | "all" => Ok(Self::Sync),
            _ => Err(CoreError::UnknownChannel(s.to_string())),
        }
    }
}

/// One anchor of a fan curve: at `temp_c`, run at `duty_pct`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FanPoint {
    pub temp_c: f32,
    pub duty_pct: f32,
}

impl FanPoint {
    pub fn new(temp_c: f32, duty_pct: f32) -> Result<Self, CoreError> {
        if !temp_c.is_finite() || !(0.0..=120.0).contains(&temp_c) {
            return Err(CoreError::InvalidFanPoint(format!(
                "temperature {temp_c} out of range 0..=120C"
            )));
        }
        if !duty_pct.is_finite() || !(0.0..=100.0).contains(&duty_pct) {
            return Err(CoreError::InvalidFanPoint(format!(
                "duty {duty_pct} out of range 0..=100%"
            )));
        }
        Ok(Self { temp_c, duty_pct })
    }
}

/// Piecewise-linear fan curve, mirroring Armoury Crate / Fan Xpert rules:
/// 8 anchors, strictly increasing temperature, non-decreasing duty.
#[derive(Debug, Clone, PartialEq)]
pub struct FanCurve {
    points: Vec<FanPoint>,
}

impl FanCurve {
    pub const EXPECTED_POINTS: usize = 8;

    pub fn new(mut points: Vec<FanPoint>) -> Result<Self, CoreError> {
        if points.len() != Self::EXPECTED_POINTS {
            return Err(CoreError::InvalidFanPoint(format!(
                "expected {} points, got {}",
                Self::EXPECTED_POINTS,
                points.len()
            )));
        }
        points.sort_by(|a, b| a.temp_c.partial_cmp(&b.temp_c).unwrap());
        for w in points.windows(2) {
            if w[1].temp_c <= w[0].temp_c {
                return Err(CoreError::InvalidFanPoint(
                    "temperatures must be strictly increasing".into(),
                ));
            }
            if w[1].duty_pct < w[0].duty_pct {
                return Err(CoreError::InvalidFanPoint(
                    "duty must be non-decreasing (like Armoury Crate requires)".into(),
                ));
            }
        }
        Ok(Self { points })
    }

    pub fn points(&self) -> &[FanPoint] {
        &self.points
    }

    /// Linear interpolation of duty (%) for a temperature.
    /// Clamps to the end anchors outside the range.
    pub fn duty_for(&self, temp_c: f32) -> f32 {
        let pts = &self.points;
        if temp_c <= pts[0].temp_c {
            return pts[0].duty_pct;
        }
        if temp_c >= pts[pts.len() - 1].temp_c {
            return pts[pts.len() - 1].duty_pct;
        }
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if temp_c >= a.temp_c && temp_c <= b.temp_c {
                let t = (temp_c - a.temp_c) / (b.temp_c - a.temp_c);
                return a.duty_pct + t * (b.duty_pct - a.duty_pct);
            }
        }
        unreachable!()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoreError {
    InvalidHex(String),
    UnknownMode(String),
    UnknownChannel(String),
    InvalidFanPoint(String),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHex(s) => write!(f, "invalid hex color (want RRGGBB): {s}"),
            Self::UnknownMode(s) => write!(f, "unknown effect mode: {s}"),
            Self::UnknownChannel(s) => write!(f, "unknown channel (want led1..led4|sync): {s}"),
            Self::InvalidFanPoint(s) => write!(f, "invalid fan curve: {s}"),
        }
    }
}

impl std::error::Error for CoreError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let c = RgbColor::from_hex("#ff0080").unwrap();
        assert_eq!((c.r, c.g, c.b), (0xFF, 0x00, 0x80));
        assert_eq!(c.to_hex(), "FF0080");
        assert!(RgbColor::from_hex("ZZZZZZ").is_err());
        assert!(RgbColor::from_hex("12345").is_err());
    }

    #[test]
    fn mode_parse_and_color_flag() {
        assert_eq!("static".parse::<EffectMode>().unwrap(), EffectMode::Static);
        assert!("nope".parse::<EffectMode>().is_err());
        assert!(EffectMode::Static.takes_color());
        assert!(!EffectMode::Rainbow.takes_color());
        assert_eq!(EffectMode::Static.code(), 0x01);
    }

    fn sample_curve() -> FanCurve {
        FanCurve::new(vec![
            FanPoint::new(40.0, 10.0).unwrap(),
            FanPoint::new(50.0, 25.0).unwrap(),
            FanPoint::new(60.0, 40.0).unwrap(),
            FanPoint::new(70.0, 55.0).unwrap(),
            FanPoint::new(75.0, 65.0).unwrap(),
            FanPoint::new(80.0, 75.0).unwrap(),
            FanPoint::new(85.0, 85.0).unwrap(),
            FanPoint::new(90.0, 100.0).unwrap(),
        ])
        .unwrap()
    }

    #[test]
    fn curve_interp_and_clamp() {
        let c = sample_curve();
        assert_eq!(c.duty_for(30.0), 10.0);
        assert_eq!(c.duty_for(95.0), 100.0);
        assert!((c.duty_for(55.0) - 32.5).abs() < 1e-4);
        assert!((c.duty_for(70.0) - 55.0).abs() < 1e-4);
    }

    #[test]
    fn curve_rejects_bad_shapes() {
        // wrong count
        let pts: Vec<FanPoint> = (0..7)
            .map(|i| FanPoint::new(40.0 + i as f32 * 5.0, 20.0).unwrap())
            .collect();
        assert!(FanCurve::new(pts).is_err());
        // decreasing duty
        let mut pts: Vec<FanPoint> = (0..8)
            .map(|i| FanPoint::new(40.0 + i as f32 * 5.0, 50.0).unwrap())
            .collect();
        pts[7].duty_pct = 10.0;
        assert!(FanCurve::new(pts).is_err());
    }
}
