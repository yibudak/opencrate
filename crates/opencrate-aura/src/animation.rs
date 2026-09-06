//! Speed-controlled software interpretations of the hardware effect names.
//! Patterns deliberately repeat every 12 LEDs: no physical fan/strip count is
//! known, and all headers receive the same frame. These are not firmware replays.

use opencrate_core::{EffectMode, RgbColor};
use std::f64::consts::TAU;

pub const MIN_SPEED: f64 = 0.25;
pub const MAX_SPEED: f64 = 4.0;

pub fn is_animated(mode: EffectMode) -> bool {
    !matches!(mode, EffectMode::Off | EffectMode::Static)
}

pub fn normalize_speed(speed: f64) -> f64 {
    if speed.is_finite() {
        speed.clamp(MIN_SPEED, MAX_SPEED)
    } else {
        1.0
    }
}

/// Scaled animation time. Integrating elapsed time preserves the current phase
/// when speed changes and does not slow effects down when a frame is delayed.
#[derive(Default)]
pub struct Timeline {
    seconds: f64,
}

impl Timeline {
    pub fn advance(&mut self, elapsed: std::time::Duration, speed: f64) {
        // Every effect period divides 120; keep time bounded for long tray runs.
        self.seconds = (self.seconds + elapsed.as_secs_f64() * normalize_speed(speed)) % 120.0;
    }

    pub fn seconds(&self) -> f64 {
        self.seconds
    }
}

fn dim(color: RgbColor, level: f64) -> RgbColor {
    let level = level.clamp(0.0, 1.0);
    RgbColor::new(
        (color.r as f64 * level).round() as u8,
        (color.g as f64 * level).round() as u8,
        (color.b as f64 * level).round() as u8,
    )
}

/// Scale all RGB components equally without modifying the selected base color.
pub fn apply_brightness(color: RgbColor, percent: u8) -> RgbColor {
    let percent = u16::from(percent.min(100));
    let scale = |value| ((u16::from(value) * percent + 50) / 100) as u8;
    RgbColor::new(scale(color.r), scale(color.g), scale(color.b))
}

fn spectrum(hue: f64) -> RgbColor {
    let h = hue.rem_euclid(1.0) * 6.0;
    let rising = ((h.fract()) * 255.0).round() as u8;
    let falling = 255 - rising;
    match h as u8 {
        0 => RgbColor::new(255, rising, 0),
        1 => RgbColor::new(falling, 255, 0),
        2 => RgbColor::new(0, 255, rising),
        3 => RgbColor::new(0, falling, 255),
        4 => RgbColor::new(rising, 0, 255),
        _ => RgbColor::new(255, 0, falling),
    }
}

fn pulse(phase: f64) -> f64 {
    (1.0 - (TAU * phase).cos()) / 2.0
}

/// Render at already speed-scaled time. Static/Off are included for previews;
/// live playback sends those two through the validated native effect path.
pub fn render(
    mode: EffectMode,
    color: RgbColor,
    seconds: f64,
    brightness: u8,
    out: &mut [RgbColor],
) {
    let t = if seconds.is_finite() {
        seconds.rem_euclid(120.0)
    } else {
        0.0
    };
    let breath = pulse(t / 3.0);
    for (index, led) in out.iter_mut().enumerate() {
        let position = (index % 12) as f64 / 12.0;
        let behind = (t / 3.0 - position).rem_euclid(1.0);
        let chase = if behind < 0.25 { 1.0 } else { 0.0 };
        let tail = (1.0 - behind / 0.6).max(0.0).powi(2);
        let rainbow = spectrum(position - t / 6.0);
        *led = match mode {
            EffectMode::Off => RgbColor::BLACK,
            EffectMode::Static => color,
            EffectMode::Breathing => dim(color, breath),
            // At maximum speed this is two flashes per second.
            EffectMode::Flashing => dim(color, if t % 2.0 < 1.0 { 1.0 } else { 0.0 }),
            EffectMode::SpectrumCycle => spectrum(t / 6.0),
            EffectMode::Rainbow => rainbow,
            EffectMode::SpectrumCycleBreathing => dim(spectrum(t / 6.0), breath),
            EffectMode::ChaseFade => dim(color, tail),
            EffectMode::SpectrumCycleChaseFade => dim(spectrum(t / 6.0), tail),
            EffectMode::Chase => dim(color, chase),
            EffectMode::SpectrumCycleChase => dim(spectrum(t / 6.0), chase),
            EffectMode::SpectrumCycleWave => dim(spectrum(t / 6.0), pulse(position - t / 3.0)),
            EffectMode::ChaseRainbowPulse => dim(rainbow, tail * breath),
            // Deterministic, smoothly varying sparkle; no frame-rate-dependent RNG.
            EffectMode::RainbowFlicker => dim(rainbow, pulse(t / 2.0 + position * 7.0).powi(4)),
            EffectMode::GentleTransition => dim(spectrum(t / 12.0), 0.4 + 0.6 * pulse(t / 6.0)),
            EffectMode::WavePropagation | EffectMode::WavePropagationPause => {
                let phase = if mode == EffectMode::WavePropagationPause {
                    (t / 5.0).fract() * 1.5
                } else {
                    (t / 3.0).fract()
                };
                let distance = (position - 0.5).abs() * 2.0;
                let level = (1.0 - (distance - phase * 1.4).abs() / 0.35).max(0.0);
                dim(spectrum(t / 6.0), level)
            }
        };
        *led = apply_brightness(*led, brightness);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn speed_scales_time_and_changes_without_phase_jump() {
        let mut clock = Timeline::default();
        clock.advance(Duration::from_millis(750), 1.0);
        assert_eq!(clock.seconds(), 0.75);
        clock.advance(Duration::ZERO, 4.0);
        assert_eq!(clock.seconds(), 0.75);
        clock.advance(Duration::from_millis(250), 4.0);
        assert_eq!(clock.seconds(), 1.75);
        clock.advance(Duration::from_secs(1), 0.25);
        assert_eq!(clock.seconds(), 2.0);
        assert_eq!(normalize_speed(f64::NAN), 1.0);
    }

    #[test]
    fn breathing_preserves_selected_color_and_reaches_dark_and_full() {
        let color = RgbColor::new(0, 120, 255);
        let mut frame = [RgbColor::BLACK; 24];
        render(EffectMode::Breathing, color, 0.0, 100, &mut frame);
        assert!(frame.iter().all(|c| *c == RgbColor::BLACK));
        render(EffectMode::Breathing, color, 1.5, 100, &mut frame);
        assert!(frame.iter().all(|c| *c == color));
        render(EffectMode::Breathing, color, 0.75, 100, &mut frame);
        assert!(frame
            .iter()
            .all(|c| *c == RgbColor::new(0, 60, 127) || *c == RgbColor::new(0, 60, 128)));
    }

    #[test]
    fn every_animated_mode_moves_and_static_modes_do_not() {
        let color = RgbColor::new(18, 100, 255);
        for &mode in EffectMode::all() {
            let mut first = [RgbColor::BLACK; 24];
            render(mode, color, 0.0, 100, &mut first);
            let changed = [0.37, 1.13, 2.41].iter().any(|&time| {
                let mut later = first;
                render(mode, color, time, 100, &mut later);
                later != first
            });
            assert_eq!(changed, is_animated(mode), "{mode:?}");
            assert_eq!(&first[..12], &first[12..]);
        }
    }

    #[test]
    fn brightness_preserves_full_color_and_scales_each_component() {
        let color = RgbColor::new(40, 120, 240);
        assert_eq!(apply_brightness(color, 0), RgbColor::BLACK);
        assert_eq!(apply_brightness(color, 25), RgbColor::new(10, 30, 60));
        assert_eq!(apply_brightness(color, 100), color);
        assert_eq!(apply_brightness(color, 255), color);
        assert_eq!(
            apply_brightness(RgbColor::new(0, 255, 0), 50),
            RgbColor::new(0, 128, 0)
        );
    }

    #[test]
    fn brightness_dims_every_effect_and_can_return_from_zero() {
        let color = RgbColor::new(40, 120, 240);
        for &mode in EffectMode::all() {
            let mut full = [RgbColor::BLACK; 24];
            render(mode, color, 1.23, 100, &mut full);
            let mut frame = full;
            render(mode, color, 1.23, 0, &mut frame);
            assert!(frame.iter().all(|&c| c == RgbColor::BLACK), "{mode:?}");
            render(mode, color, 1.23, 25, &mut frame);
            assert_eq!(frame, full.map(|c| apply_brightness(c, 25)), "{mode:?}");
            render(mode, color, 1.23, 100, &mut frame);
            assert_eq!(frame, full, "{mode:?}");
        }
    }
}
