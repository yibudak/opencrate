//! Fan control primitives: presets, headers, controller abstraction.
//!
//! `asus` provides the Windows ASUS COM service backend used by the GUI;
//! `service` owns asynchronous control and thermal curve validation.
//! The legacy mock controller below remains available for CLI dry-runs.

use opencrate_core::FanCurve;

#[cfg(windows)]
pub mod asus;
pub mod quick;
pub mod service;

/// Synthetic fan-header descriptor for CLI demonstrations and unit tests.
#[derive(Debug, Clone, PartialEq)]
pub struct FanHeader {
    pub id: u8,
    pub name: String,
    pub max_rpm: Option<u32>,
}

impl FanHeader {
    pub fn new(id: u8, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            max_rpm: None,
        }
    }
}

/// Built-in PWM presets mirroring Polylux/Armoury Crate choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PwmPreset {
    Auto,
    Off,
    Silent,
    Medium,
    Full,
}

impl PwmPreset {
    /// Fixed duty for presets that don't need a curve. `Auto` returns None
    /// (firmware decides).
    pub fn fixed_duty(self) -> Option<f32> {
        match self {
            Self::Auto => None,
            Self::Off => Some(0.0),
            Self::Silent => Some(30.0),
            Self::Medium => Some(60.0),
            Self::Full => Some(100.0),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Off => "off",
            Self::Silent => "silent",
            Self::Medium => "medium",
            Self::Full => "full",
        }
    }
}

/// What to apply to a header: firmware-auto, a fixed preset duty, or a curve.
#[derive(Debug, Clone)]
pub enum FanTarget {
    Auto,
    Fixed(f32),
    Curve(FanCurve),
}

impl FanTarget {
    pub fn from_preset(preset: PwmPreset) -> Self {
        match preset.fixed_duty() {
            None => Self::Auto,
            Some(d) => Self::Fixed(d),
        }
    }

    pub fn duty_for(&self, temp_c: f32) -> Option<f32> {
        match self {
            Self::Auto => None,
            Self::Fixed(d) => Some(*d),
            Self::Curve(c) => Some(c.duty_for(temp_c)),
        }
    }
}

/// Legacy abstraction for CLI dry-runs. The GUI uses the ASUS service backend.
pub trait FanController {
    fn headers(&self) -> &[FanHeader];
    fn set_pwm_pct(&mut self, header_id: u8, pct: f32) -> Result<(), FanError>;
    fn set_auto(&mut self, header_id: u8) -> Result<(), FanError>;
}

/// In-memory fake for tests and dry-runs.
#[derive(Debug, Default)]
pub struct MockFanController {
    headers: Vec<FanHeader>,
    applied: Vec<(u8, FanState)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FanState {
    Auto,
    Pwm(f32),
}

impl MockFanController {
    pub fn new(headers: Vec<FanHeader>) -> Self {
        Self {
            headers,
            applied: Vec::new(),
        }
    }

    pub fn applied(&self) -> &[(u8, FanState)] {
        &self.applied
    }
}

impl FanController for MockFanController {
    fn headers(&self) -> &[FanHeader] {
        &self.headers
    }

    fn set_pwm_pct(&mut self, header_id: u8, pct: f32) -> Result<(), FanError> {
        if !(0.0..=100.0).contains(&pct) {
            return Err(FanError::OutOfRange(pct));
        }
        if !self.headers.iter().any(|h| h.id == header_id) {
            return Err(FanError::UnknownHeader(header_id));
        }
        self.applied.push((header_id, FanState::Pwm(pct)));
        Ok(())
    }

    fn set_auto(&mut self, header_id: u8) -> Result<(), FanError> {
        if !self.headers.iter().any(|h| h.id == header_id) {
            return Err(FanError::UnknownHeader(header_id));
        }
        self.applied.push((header_id, FanState::Auto));
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FanError {
    UnknownHeader(u8),
    OutOfRange(f32),
    Io(String),
}

impl std::fmt::Display for FanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownHeader(id) => write!(f, "unknown fan header {id}"),
            Self::OutOfRange(p) => write!(f, "duty out of range 0..=100: {p}"),
            Self::Io(s) => write!(f, "fan io error: {s}"),
        }
    }
}

impl std::error::Error for FanError {}

#[cfg(test)]
mod tests {
    use super::*;
    use opencrate_core::{FanCurve, FanPoint};

    fn ctrl() -> MockFanController {
        MockFanController::new(vec![
            FanHeader::new(0, "CPU_FAN"),
            FanHeader::new(1, "CHA_FAN1"),
            FanHeader::new(2, "CHA_FAN2"),
        ])
    }

    #[test]
    fn presets_map() {
        assert_eq!(PwmPreset::Silent.fixed_duty(), Some(30.0));
        assert_eq!(PwmPreset::Auto.fixed_duty(), None);
    }

    #[test]
    fn mock_records_writes_and_validates() {
        let mut c = ctrl();
        c.set_pwm_pct(0, 60.0).unwrap();
        c.set_auto(1).unwrap();
        assert_eq!(
            c.applied(),
            &[(0, FanState::Pwm(60.0)), (1, FanState::Auto)]
        );
        assert!(c.set_pwm_pct(9, 50.0).is_err());
        assert!(c.set_pwm_pct(0, 150.0).is_err());
    }

    #[test]
    fn curve_target_interpolates() {
        let curve = FanCurve::new(vec![
            FanPoint::new(40.0, 10.0).unwrap(),
            FanPoint::new(50.0, 25.0).unwrap(),
            FanPoint::new(60.0, 40.0).unwrap(),
            FanPoint::new(70.0, 55.0).unwrap(),
            FanPoint::new(75.0, 65.0).unwrap(),
            FanPoint::new(80.0, 75.0).unwrap(),
            FanPoint::new(85.0, 85.0).unwrap(),
            FanPoint::new(90.0, 100.0).unwrap(),
        ])
        .unwrap();
        let t = FanTarget::Curve(curve);
        assert!((t.duty_for(55.0).unwrap() - 32.5).abs() < 1e-4);
    }
}
