//! Read-only CPU telemetry. Sensor failures never block power-plan changes.
use std::{
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

#[cfg(windows)]
#[path = "telemetry_windows.rs"]
mod native;

#[derive(Clone, Debug, Default)]
pub struct Reading {
    pub cpu_name: String,
    pub power_w: Option<f64>,
    pub temperature_c: Option<f64>,
    pub frequency_mhz: Option<f64>,
    pub load_percent: Option<f64>,
    pub voltage_v: Option<f64>,
    pub tdc_a: Option<f64>,
    pub edc_a: Option<f64>,
    pub provider: Option<&'static str>,
    pub windows_frequency: bool,
    pub sampled_at: Option<Instant>,
    pub error: Option<String>,
    pub ryzen_master: Option<PathBuf>,
}
impl Reading {
    pub fn is_ryzen(&self) -> bool {
        self.cpu_name.to_ascii_lowercase().contains("ryzen")
    }
    pub fn fresh(&self) -> bool {
        self.sampled_at
            .is_some_and(|time| time.elapsed() < Duration::from_secs(5))
    }
}

#[derive(Clone, Debug)]
pub struct Sensor {
    pub parent: String,
    pub name: String,
    pub kind: String,
    pub value: Option<f64>,
}

/// Choose package sensors from one CPU, never GPU/board/SoC power or a bus clock.
/// Missing and sentinel values stay absent; clocks are the mean of core clocks.
pub fn cpu_reading(sensors: &[Sensor]) -> Reading {
    let cpu = sensors
        .iter()
        .find(|s| s.parent.starts_with("/amdcpu/") || s.parent.starts_with("/intelcpu/"))
        .map(|s| s.parent.as_str());
    let sensors: Vec<_> = sensors
        .iter()
        .filter(|s| Some(s.parent.as_str()) == cpu)
        .collect();
    let find = |kind: &str, names: &[&str], max: f64| {
        names.iter().find_map(|name| {
            sensors.iter().find_map(|s| {
                (s.kind == kind && s.name.eq_ignore_ascii_case(name))
                    .then_some(s.value)
                    .flatten()
                    .filter(|v| v.is_finite() && *v > 0.0 && *v <= max)
            })
        })
    };
    let clocks: Vec<_> = sensors
        .iter()
        .filter(|s| s.kind == "Clock" && s.name.starts_with("CPU Core"))
        .filter_map(|s| s.value)
        .filter(|v| v.is_finite() && *v > 0.0 && *v <= 15000.0)
        .collect();
    Reading {
        power_w: find("Power", &["CPU Package", "Package", "CPU PPT"], 2000.0),
        temperature_c: find(
            "Temperature",
            &[
                "Core (Tctl/Tdie)",
                "CPU Package",
                "Core (Tdie)",
                "CPU (Tctl/Tdie)",
                "CPU Cores",
                "CPU Core",
            ],
            150.0,
        ),
        frequency_mhz: (!clocks.is_empty())
            .then(|| clocks.iter().sum::<f64>() / clocks.len() as f64),
        load_percent: sensors
            .iter()
            .find(|s| s.kind == "Load" && s.name == "CPU Total")
            .and_then(|s| s.value)
            .filter(|v| v.is_finite() && (0.0..=100.0).contains(v)),
        voltage_v: find(
            "Voltage",
            &["CPU Core (SVI3 TFN)", "CPU Core (SVI2 TFN)", "CPU Core"],
            3.0,
        ),
        tdc_a: find("Current", &["CPU TDC", "TDC"], 2000.0),
        edc_a: find("Current", &["CPU EDC", "EDC"], 2000.0),
        ..Reading::default()
    }
}

pub struct Monitor {
    stop: mpsc::Sender<bool>,
    active: bool,
    latest: Arc<Mutex<Option<Reading>>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Monitor {
    pub fn start(wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let (stop, incoming) = mpsc::channel();
        let latest = Arc::new(Mutex::new(None));
        let outgoing = latest.clone();
        let worker = thread::Builder::new()
            .name("opencrate-cpu".into())
            .spawn(move || {
                #[cfg(windows)]
                {
                    let mut reader = native::Reader::new();
                    loop {
                        let started = Instant::now();
                        let reading = match &mut reader {
                            Ok(reader) => reader.sample(),
                            Err(error) => Reading {
                                error: Some(error.clone()),
                                ..Reading::default()
                            },
                        };
                        *outgoing.lock().unwrap_or_else(|e| e.into_inner()) = Some(reading);
                        wake();
                        match incoming
                            .recv_timeout(Duration::from_secs(1).saturating_sub(started.elapsed()))
                        {
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                if reader.is_err() {
                                    reader = native::Reader::new();
                                }
                            }
                            Ok(true) => {}
                            Ok(false) => loop {
                                match incoming.recv() {
                                    Ok(true) => break,
                                    Ok(false) => {}
                                    Err(_) => return,
                                }
                            },
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    }
                }
                #[cfg(not(windows))]
                {
                    let _ = incoming;
                    *outgoing.lock().unwrap_or_else(|e| e.into_inner()) = Some(Reading {
                        error: Some("CPU telemetry requires Windows.".into()),
                        ..Reading::default()
                    });
                    wake();
                }
            })?;
        Ok(Self {
            stop,
            active: true,
            latest,
            worker: Some(worker),
        })
    }
    pub fn set_active(&mut self, active: bool) {
        if self.active != active {
            self.active = active;
            let _ = self.stop.send(active);
        }
    }
    pub fn take(&self) -> Option<Reading> {
        self.latest.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        // Dropping the sender below stops both active and paused workers.
        // A third-party WMI provider may hang in COM. Never block GUI shutdown.
        if let Some(worker) = self.worker.take().filter(|w| w.is_finished()) {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sensor(parent: &str, kind: &str, name: &str, value: Option<f64>) -> Sensor {
        Sensor {
            parent: parent.into(),
            kind: kind.into(),
            name: name.into(),
            value,
        }
    }
    #[test]
    fn ryzen_package_metrics_exclude_gpu_soc_bus_and_other_sockets() {
        let readings = cpu_reading(&[
            sensor("/gpu-amd/0", "Power", "CPU Package", Some(300.0)),
            sensor("/amdcpu/0", "Power", "CPU Cores", Some(25.0)),
            sensor("/amdcpu/0", "Power", "CPU Package", Some(45.0)),
            sensor("/amdcpu/0", "Temperature", "Core (Tctl/Tdie)", Some(65.0)),
            sensor("/amdcpu/0", "Clock", "Bus Speed", Some(100.0)),
            sensor("/amdcpu/0", "Clock", "CPU Core #1", Some(4200.0)),
            sensor("/amdcpu/0", "Clock", "CPU Core #2", Some(4400.0)),
            sensor("/amdcpu/1", "Clock", "CPU Core #1", Some(5000.0)),
            sensor("/amdcpu/0", "Load", "CPU Total", Some(0.0)),
        ]);
        assert_eq!(readings.power_w, Some(45.0));
        assert_eq!(readings.temperature_c, Some(65.0));
        assert_eq!(readings.frequency_mhz, Some(4300.0));
        assert_eq!(readings.load_percent, Some(0.0));
    }
    #[test]
    fn missing_nonfinite_and_provider_zero_sentinels_are_not_measurements() {
        let reading = cpu_reading(&[
            sensor("/intelcpu/0", "Power", "CPU Package", None),
            sensor("/intelcpu/0", "Temperature", "CPU Package", Some(0.0)),
            sensor("/intelcpu/0", "Clock", "CPU Core #1", Some(f64::NAN)),
            sensor("/intelcpu/0", "Load", "CPU Total", Some(101.0)),
            sensor("/intelcpu/0", "Voltage", "CPU Core", Some(f64::INFINITY)),
        ]);
        assert!(reading.power_w.is_none());
        assert!(reading.temperature_c.is_none());
        assert!(reading.frequency_mhz.is_none());
        assert!(reading.load_percent.is_none());
        assert!(reading.voltage_v.is_none());
        assert!(!reading.fresh());
    }
    #[test]
    fn intel_package_and_expired_readings() {
        let mut reading = cpu_reading(&[sensor(
            "/intelcpu/0",
            "Temperature",
            "CPU Package",
            Some(51.0),
        )]);
        assert_eq!(reading.temperature_c, Some(51.0));
        reading.sampled_at = Some(Instant::now() - Duration::from_secs(6));
        assert!(!reading.fresh());
        assert!(
            cpu_reading(&[sensor("/gpu-nvidia/0", "Power", "CPU Package", Some(200.0))])
                .power_w
                .is_none()
        );
    }
}
