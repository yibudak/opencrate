use super::{cpu_reading, Reading, Sensor};
use std::{
    marker::PhantomData,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};
use windows::{
    core::{BSTR, PCWSTR},
    Win32::System::{
        Com::*,
        Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE},
        Variant::{VARIANT, VT_EMPTY, VT_NULL},
        Wmi::*,
    },
};

struct Apartment(PhantomData<Rc<()>>);
impl Apartment {
    fn new() -> Result<Self, String> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }.map_err(|e| e.to_string())?;
        Ok(Self(PhantomData))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn connect(namespace: &str) -> Result<IWbemServices, String> {
    unsafe {
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| e.to_string())?;
        let empty = BSTR::new();
        let services = locator
            .ConnectServer(
                &BSTR::from(namespace),
                &empty,
                &empty,
                &empty,
                WBEM_FLAG_CONNECT_USE_MAX_WAIT.0,
                &empty,
                None,
            )
            .map_err(|e| e.to_string())?;
        CoSetProxyBlanket(
            &services,
            RPC_C_AUTHN_WINNT,
            RPC_C_AUTHZ_NONE,
            PCWSTR::null(),
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )
        .map_err(|e| e.to_string())?;
        Ok(services)
    }
}

fn query(services: &IWbemServices, sql: &str) -> Result<Vec<IWbemClassObject>, String> {
    let enumeration = unsafe {
        services.ExecQuery(
            &BSTR::from("WQL"),
            &BSTR::from(sql),
            WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
            None,
        )
    }
    .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_millis(800);
    let mut rows = Vec::new();
    loop {
        if Instant::now() >= deadline || rows.len() >= 4096 {
            return Err("CPU sensor query exceeded its time or size limit.".into());
        }
        let mut batch = [const { None }; 32];
        let mut returned = 0;
        let result = unsafe { enumeration.Next(100, &mut batch, &mut returned) };
        result.ok().map_err(|e| e.to_string())?;
        rows.extend(batch.into_iter().take(returned as usize).flatten());
        if result.0 == WBEM_S_FALSE.0 {
            return Ok(rows);
        }
        if result.0 != WBEM_S_TIMEDOUT.0 && returned == 0 {
            return Ok(rows);
        }
    }
}
fn property(row: &IWbemClassObject, key: &str) -> Option<VARIANT> {
    let wide: Vec<_> = key.encode_utf16().chain(Some(0)).collect();
    let mut value = VARIANT::default();
    unsafe { row.Get(PCWSTR(wide.as_ptr()), 0, &mut value, None, None) }.ok()?;
    let kind = unsafe { value.Anonymous.Anonymous.vt };
    (kind != VT_EMPTY && kind != VT_NULL).then_some(value)
}
fn text(row: &IWbemClassObject, key: &str) -> Option<String> {
    BSTR::try_from(&property(row, key)?)
        .ok()
        .map(|s| s.to_string())
}
fn number(row: &IWbemClassObject, key: &str) -> Option<f64> {
    f64::try_from(&property(row, key)?)
        .ok()
        .filter(|v| v.is_finite())
}

pub struct Reader {
    system: Option<IWbemServices>,
    providers: Vec<(&'static str, IWbemServices)>,
    reconnect_at: Instant,
    cpu_name: String,
    ryzen_master: Option<PathBuf>,
    _apartment: Apartment,
}
impl Reader {
    pub fn new() -> Result<Self, String> {
        let apartment = Apartment::new()?;
        let system = connect("ROOT\\CIMV2").ok();
        let cpu_name = system
            .as_ref()
            .and_then(|s| query(s, "SELECT Name FROM Win32_Processor").ok())
            .and_then(|rows| rows.first().and_then(|row| text(row, "Name")))
            .unwrap_or_default();
        let ryzen_master = ["ProgramW6432", "ProgramFiles"]
            .iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from)
            .map(|base| base.join("AMD/RyzenMaster/bin/AMDRyzenMaster.exe"))
            .find(|p| p.is_file());
        Ok(Self {
            system,
            providers: Vec::new(),
            reconnect_at: Instant::now(),
            cpu_name,
            ryzen_master,
            _apartment: apartment,
        })
    }
    pub fn sample(&mut self) -> Reading {
        if Instant::now() >= self.reconnect_at {
            self.providers = ["LibreHardwareMonitor", "OpenHardwareMonitor"]
                .into_iter()
                .filter_map(|name| connect(&format!("ROOT\\{name}")).ok().map(|s| (name, s)))
                .collect();
            if self.system.is_none() {
                self.system = connect("ROOT\\CIMV2").ok();
            }
            self.reconnect_at = Instant::now() + Duration::from_secs(15);
        }
        let mut reading = Reading::default();
        for (provider, services) in &self.providers {
            match query(
                services,
                "SELECT Parent, Name, SensorType, Value FROM Sensor",
            ) {
                Ok(rows) => {
                    let sensors: Vec<_> = rows
                        .iter()
                        .filter_map(|r| {
                            Some(Sensor {
                                parent: text(r, "Parent")?,
                                name: text(r, "Name")?,
                                kind: text(r, "SensorType")?,
                                value: number(r, "Value"),
                            })
                        })
                        .collect();
                    let candidate = cpu_reading(&sensors);
                    if candidate.power_w.is_some()
                        || candidate.temperature_c.is_some()
                        || candidate.frequency_mhz.is_some()
                    {
                        reading = candidate;
                        reading.provider = Some(provider);
                        break;
                    }
                }
                Err(error) => reading.error = Some(format!("{provider}: {error}")),
            }
        }
        if let Some(system) = &self.system {
            match query(system, "SELECT PercentProcessorTime, ProcessorFrequency, PercentProcessorPerformance FROM Win32_PerfFormattedData_Counters_ProcessorInformation WHERE Name = '_Total'") {
                Ok(rows) => if let Some(row) = rows.first() {
                    if reading.load_percent.is_none() {
                        reading.load_percent = number(row, "PercentProcessorTime").filter(|v| (0.0..=100.0).contains(v));
                    }
                    if reading.frequency_mhz.is_none() {
                        // Windows performance counters are an estimate, explicitly labeled in UI.
                        reading.frequency_mhz = number(row, "ProcessorFrequency").zip(number(row, "PercentProcessorPerformance"))
                            .map(|(base, percent)| base * percent / 100.0).filter(|v| *v > 0.0 && *v <= 15000.0);
                        reading.windows_frequency = reading.frequency_mhz.is_some();
                    }
                },
                Err(error) => { reading.error.get_or_insert(error); },
            }
        }
        reading.cpu_name.clone_from(&self.cpu_name);
        reading.ryzen_master.clone_from(&self.ryzen_master);
        if reading.power_w.is_some()
            || reading.temperature_c.is_some()
            || reading.frequency_mhz.is_some()
            || reading.load_percent.is_some()
        {
            reading.sampled_at = Some(Instant::now());
        }
        reading
    }
}
