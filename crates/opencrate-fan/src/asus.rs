//! Native, late-bound COM client for AsusFanControlService. All objects stay on
//! the thread that initialized COM. No direct SuperIO or kernel driver writes.

use crate::quick::{self, Change, QuickMode, Setting};
use crate::service::{supported_minimum, validate_custom, Fan, Point, Profile, Target};
use std::{collections::HashMap, marker::PhantomData, rc::Rc};
use windows::{
    core::{IUnknown, Interface, BSTR, GUID, PCWSTR},
    Win32::System::{
        Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, IDispatch, CLSCTX_ALL,
            CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, DISPATCH_FLAGS, DISPATCH_METHOD,
            DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT, DISPPARAMS,
        },
        Ole::DISPID_PROPERTYPUT,
        Variant::VARIANT,
    },
};

type Result<T> = std::result::Result<T, String>;

struct Apartment(PhantomData<Rc<()>>);
impl Apartment {
    fn new() -> Result<Self> {
        // This guard is deliberately !Send/!Sync. COM interfaces are released
        // before CoUninitialize, including error paths during connection.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }
            .map_err(|e| format!("Initialize fan COM: {e}"))?;
        Ok(Self(PhantomData))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

#[derive(Clone)]
struct Dispatch(IDispatch);
impl Dispatch {
    fn invoke(&self, name: &str, flags: DISPATCH_FLAGS, mut args: Vec<VARIANT>) -> Result<VARIANT> {
        let wide = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let mut id = 0;
        let nil = GUID::zeroed();
        unsafe {
            self.0
                .GetIDsOfNames(&nil, &PCWSTR(wide.as_ptr()), 1, 0, &mut id)
        }
        .map_err(|e| format!("ASUS {name}: {e}"))?;
        // Automation arguments are in reverse order. Property puts require
        // the named DISPID_PROPERTYPUT argument as well as the value.
        args.reverse();
        let mut put = DISPID_PROPERTYPUT;
        let params = DISPPARAMS {
            rgvarg: args.as_mut_ptr(),
            cArgs: args.len() as u32,
            rgdispidNamedArgs: if flags == DISPATCH_PROPERTYPUT {
                &mut put
            } else {
                std::ptr::null_mut()
            },
            cNamedArgs: u32::from(flags == DISPATCH_PROPERTYPUT),
        };
        let mut result = VARIANT::default();
        unsafe {
            self.0
                .Invoke(id, &nil, 0, flags, &params, Some(&mut result), None, None)
        }
        .map_err(|e| format!("ASUS {name}: {e}"))?;
        Ok(result)
    }
    fn get(&self, name: &str) -> Result<VARIANT> {
        self.invoke(name, DISPATCH_PROPERTYGET, vec![])
    }
    fn number(&self, name: &str) -> Result<u32> {
        u32::try_from(&self.get(name)?).map_err(|e| format!("Invalid ASUS {name}: {e}"))
    }
    fn boolean(&self, name: &str) -> Result<bool> {
        bool::try_from(&self.get(name)?).map_err(|e| format!("Invalid ASUS {name}: {e}"))
    }
    fn string(&self, name: &str) -> Result<String> {
        BSTR::try_from(&self.get(name)?)
            .map(|s| s.to_string())
            .map_err(|e| format!("Invalid ASUS {name}: {e}"))
    }
    fn object(value: VARIANT) -> Result<Self> {
        IDispatch::try_from(&value)
            .or_else(|_| IUnknown::try_from(&value)?.cast())
            .map(Self)
            .map_err(|e| format!("Invalid ASUS object: {e}"))
    }
    fn child(&self, name: &str) -> Result<Self> {
        Self::object(self.get(name)?)
    }
    fn item(&self, index: i32) -> Result<Self> {
        Self::object(self.invoke("Item", DISPATCH_PROPERTYGET, vec![index.into()])?)
    }
    fn put(&self, name: &str, value: i32) -> Result<()> {
        self.invoke(name, DISPATCH_PROPERTYPUT, vec![value.into()])
            .map(|_| ())
    }
    fn count(&self, max: u32) -> Result<u32> {
        let count = self.number("Count")?;
        if count > max {
            return Err(format!("Unsupported ASUS collection size: {count}"));
        }
        Ok(count)
    }
}

fn read_curve(curve: &Dispatch) -> Result<Vec<Point>> {
    let count = curve.count(10)?;
    if count == 0 {
        return Err("ASUS returned an empty fan curve.".into());
    }
    (0..count)
        .map(|i| {
            let point = curve.item(i as i32)?;
            Ok(Point {
                temperature: u8::try_from(point.number("Temperature")?)
                    .map_err(|_| "Invalid fan temperature")?,
                duty: u8::try_from(point.number("Speed")?).map_err(|_| "Invalid fan duty")?,
            })
        })
        .collect()
}

#[derive(Clone)]
struct OwnedCurve {
    original: Vec<Point>,
    applied: Vec<Point>,
}

pub struct Session {
    controls: Dispatch,
    sensors: Option<Dispatch>,
    owned: HashMap<u32, OwnedCurve>,
    quick_undo: Option<Vec<Change>>,
    // Rust drops fields in declaration order: release all interfaces first.
    _apartment: Apartment,
}

impl Session {
    fn connect_sensors() -> Option<Dispatch> {
        // aaHM.acpiHmData2 is the installed ASUS hardware-monitor provider.
        // Its absence must not disable fan control or duty telemetry.
        unsafe {
            CoCreateInstance(
                &GUID::from_u128(0x2627f8be_4482_4081_bc62_8a12ca24bdf8),
                None,
                CLSCTX_ALL,
            )
        }
        .ok()
        .map(Dispatch)
    }

    fn rpm_readings(&self) -> Result<Vec<(String, u32)>> {
        let monitor = self
            .sensors
            .as_ref()
            .ok_or("ASUS RPM provider unavailable")?;
        monitor.invoke("Refresh", DISPATCH_METHOD, vec![])?;
        let sensors = monitor.child("Sensors")?;
        (0..sensors.count(256)?)
            .map(|i| {
                let sensor = sensors.item(i as i32)?;
                Ok((sensor.string("name")?, sensor.number("current")?))
            })
            .collect()
    }

    fn connect_controls() -> Result<Dispatch> {
        let manager = Dispatch(
            unsafe {
                CoCreateInstance(
                    &GUID::from_u128(0x14083c53_b8e7_48e4_9320_811f3478c4a4),
                    None,
                    CLSCTX_LOCAL_SERVER,
                )
            }
            .map_err(|e| {
                format!("ASUS fan service unavailable. Install or start AsusFanControlService. {e}")
            })?,
        );
        let controls = manager.child("Controls")?;
        if controls.count(32)? == 0 {
            return Err("ASUS did not report any controllable fans.".into());
        }
        Ok(controls)
    }

    pub fn connect() -> Result<Self> {
        let apartment = Apartment::new()?;
        let controls = Self::connect_controls()?;
        Ok(Self {
            controls,
            sensors: Self::connect_sensors(),
            owned: HashMap::new(),
            quick_undo: None,
            _apartment: apartment,
        })
    }

    pub fn reconnect(&mut self) -> Result<()> {
        self.controls = Self::connect_controls()?;
        self.sensors = Self::connect_sensors();
        Ok(())
    }

    fn control(&self, id: u32) -> Result<Dispatch> {
        for i in 0..self.controls.count(32)? {
            let fan = self.controls.item(i as i32)?;
            if fan.number("Id")? == id {
                return Ok(fan);
            }
        }
        Err(format!("ASUS fan {id} is no longer available."))
    }

    fn read_fan(&self, control: &Dispatch) -> Result<Fan> {
        let id = control.number("Id")?;
        let curve = read_curve(&control.child("CurrentFanCurve")?)?;
        let collection = control.child("Profiles")?;
        let mut profiles = Vec::new();
        let mut controller_curves = Vec::new();
        for i in 0..collection.count(16)? {
            let profile = collection.item(i as i32)?;
            let name = profile.string("Name")?;
            let Ok(points) = profile
                .child("FanCurve")
                .and_then(|curve| read_curve(&curve))
            else {
                continue;
            };
            controller_curves.push(points.clone());
            // Do not expose unknown/disabled/user profiles as named presets.
            // ASUS "Disable" means full speed, not stopping the fan.
            if ["Standard", "Silent", "Turbo"].contains(&name.as_str())
                && points.len() == curve.len()
                && points.last().is_some_and(|p| p.duty == 255)
            {
                profiles.push(Profile {
                    index: i as i32,
                    name,
                    curve: points,
                });
            }
        }
        let reported_minimum =
            u8::try_from(control.number("MinimalDuty")?).map_err(|_| "Invalid fan minimum")?;
        let minimum = supported_minimum(
            reported_minimum,
            std::iter::once(curve.as_slice())
                .chain(controller_curves.iter().map(Vec::as_slice))
                // Keep low-speed capability after temporarily applying a
                // faster curve, including Full Blast.
                .chain(self.owned.get(&id).map(|o| o.original.as_slice())),
        );
        Ok(Fan {
            id,
            name: control
                .string("DisplayName")
                .or_else(|_| control.string("Name"))?,
            duty: u8::try_from(control.number("DutyCycle")?)
                .map_err(|_| "Invalid duty readback")?,
            rpm: None,
            minimum,
            writable: !control.boolean("IsRpmMode")? && (4..=10).contains(&curve.len()),
            can_restore: self.owned.contains_key(&id),
            curve,
            profiles,
        })
    }

    pub fn snapshot(&self) -> Result<Vec<Fan>> {
        let readings = self.rpm_readings().unwrap_or_default();
        (0..self.controls.count(32)?)
            .map(|i| {
                let control = self.controls.item(i as i32)?;
                let mut fan = self.read_fan(&control)?;
                // DisplayName can be user-edited; match the stable ASUS Name.
                fan.rpm = control
                    .string("Name")
                    .ok()
                    .and_then(|name| match_rpm(&readings, &name));
                Ok(fan)
            })
            .collect()
    }

    fn write_curve(control: &Dispatch, points: &[Point]) -> Result<()> {
        let buffer = control.child("CurrentFanCurve")?;
        if buffer.count(10)? as usize != points.len() {
            return Err("Fan point count changed; refresh before applying.".into());
        }
        // CurrentFanCurve is a detached edit buffer inside the ASUS service.
        // Point setters alone do not change hardware. The method copies that
        // buffer into the controller and leaves FanStore.xml untouched.
        for (i, point) in points.iter().enumerate() {
            let edit = buffer.item(i as i32)?;
            edit.put("Temperature", i32::from(point.temperature))?;
            edit.put("Speed", i32::from(point.duty))?;
        }
        let result = control.invoke(
            "ApplyFanCurveButNotSave",
            DISPATCH_METHOD,
            vec![buffer.0.into()],
        )?;
        if u32::try_from(&result).map_err(|e| e.to_string())? != 1 {
            return Err("ASUS rejected the fan curve.".into());
        }
        let actual = read_curve(&control.child("CurrentFanCurve")?)?;
        if actual != points {
            return Err("ASUS fan curve readback did not match the request.".into());
        }
        Ok(())
    }

    pub fn apply(&mut self, id: u32, target: Target) -> Result<String> {
        let control = self.control(id)?;
        let fan = self.read_fan(&control)?;
        if !fan.writable {
            return Err("This fan's control mode is not supported.".into());
        }
        let restoring = matches!(target, Target::Restore);
        let (curve, label) = match target {
            Target::Profile(index) => {
                let profile = fan
                    .profiles
                    .iter()
                    .find(|p| p.index == index)
                    .ok_or("Fan profile is unavailable.")?;
                (profile.curve.clone(), profile.name.clone())
            }
            Target::Custom(curve) => {
                validate_custom(&curve, &fan)?;
                (curve, "Custom curve".into())
            }
            Target::Restore => (
                self.owned
                    .get(&id)
                    .ok_or("This fan has no OpenCrate changes to restore.")?
                    .original
                    .clone(),
                "Original curve".into(),
            ),
        };
        // Record before any hardware mutation so uncertain COM results still
        // retain the original curve for explicit recovery or shutdown.
        let owned = self.owned.entry(id).or_insert_with(|| OwnedCurve {
            original: fan.curve.clone(),
            applied: fan.curve.clone(),
        });
        // If another application changed the curve, that becomes the new
        // baseline for a subsequent explicit OpenCrate apply.
        if !restoring && owned.applied != fan.curve {
            owned.original = fan.curve.clone();
        }
        owned.applied = curve.clone();
        if let Err(error) = Self::write_curve(&control, &curve) {
            let recovery = Self::write_curve(&control, &fan.curve);
            if recovery.is_ok() {
                self.owned.get_mut(&id).unwrap().applied = fan.curve;
            }
            return Err(match recovery {
                Ok(()) => format!("{error} Previous curve restored."),
                Err(e) => format!("{error} Restore also failed: {e}"),
            });
        }
        if restoring {
            self.owned.remove(&id);
        }
        self.quick_undo = None;
        Ok(format!("{} · {label} applied", fan.name))
    }

    pub fn can_undo_quick(&self, fans: &[Fan]) -> bool {
        self.quick_undo
            .as_ref()
            .is_some_and(|changes| quick::can_undo(changes, fans))
    }

    pub fn reading(&self) -> Result<crate::service::Snapshot> {
        let fans = self.snapshot()?;
        Ok(crate::service::Snapshot {
            can_undo: self.can_undo_quick(&fans),
            fans,
        })
    }

    fn apply_changes(&mut self, changes: &[Change]) -> Result<()> {
        let previous_owned = self.owned.clone();
        for change in changes {
            let owned = self.owned.entry(change.id).or_insert_with(|| OwnedCurve {
                original: change.before.clone(),
                applied: change.before.clone(),
            });
            if owned.applied != change.before {
                owned.original = change.before.clone();
            }
            owned.applied = change.after.clone();
        }
        if let Err(error) = quick::execute(changes, |id, points| {
            Self::write_curve(&self.control(id)?, points)
        }) {
            // Keep recovery information only for fans whose rollback failed.
            // Unchanged/restored fans retain their pre-action ownership.
            for change in changes {
                if error
                    .rollback_failures
                    .iter()
                    .any(|(id, _)| *id == change.id)
                {
                    continue;
                }
                match previous_owned.get(&change.id) {
                    Some(owned) => {
                        self.owned.insert(change.id, owned.clone());
                    }
                    None => {
                        self.owned.remove(&change.id);
                    }
                }
            }
            if !error.rollback_failures.is_empty() {
                self.quick_undo = None;
            }
            return Err(error.message);
        }
        Ok(())
    }

    pub fn apply_quick(&mut self, mode: QuickMode) -> Result<String> {
        let fans = self.snapshot()?;
        let changes = quick::plan(&fans, mode)?;
        if changes.is_empty() {
            return Ok(format!("All fans already use {}.", mode.label()));
        }
        self.apply_changes(&changes)?;
        self.quick_undo = Some(changes);
        Ok(format!(
            "{} · All {} fans applied",
            mode.label(),
            fans.len()
        ))
    }

    pub fn apply_group(&mut self, ids: &[u32], setting: &Setting) -> Result<String> {
        let fans = self.snapshot()?;
        let changes = quick::plan_group(&fans, ids, setting)?;
        if changes.is_empty() {
            return Ok("Selected fans already use these settings.".into());
        }
        self.apply_changes(&changes)?;
        self.quick_undo = Some(changes);
        Ok(format!("Settings applied once to {} fans.", ids.len()))
    }

    pub fn undo_quick(&mut self) -> Result<String> {
        let fans = self.snapshot()?;
        if !self.can_undo_quick(&fans) {
            return Err(
                "Fan settings have changed since the last quick action. Undo is unavailable."
                    .into(),
            );
        }
        let changes = self
            .quick_undo
            .as_ref()
            .unwrap()
            .iter()
            .map(|c| Change {
                id: c.id,
                name: c.name.clone(),
                before: c.after.clone(),
                after: c.before.clone(),
            })
            .collect::<Vec<_>>();
        self.apply_changes(&changes)?;
        self.quick_undo = None;
        // Returning to the session baseline no longer needs Quit cleanup.
        self.owned
            .retain(|_, owned| owned.original != owned.applied);
        Ok("Previous fan settings restored.".into())
    }

    pub fn restore_owned(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        let ids = self.owned.keys().copied().collect::<Vec<_>>();
        for id in ids {
            let result = (|| {
                let control = self.control(id)?;
                let current = read_curve(&control.child("CurrentFanCurve")?)?;
                let owned = &self.owned[&id];
                if current == owned.applied {
                    Self::write_curve(&control, &owned.original)?;
                }
                Ok::<_, String>(())
            })();
            match result {
                Ok(()) => {
                    self.owned.remove(&id);
                }
                Err(e) => errors.push(e),
            }
        }
        errors
    }
}

fn match_rpm(readings: &[(String, u32)], name: &str) -> Option<u32> {
    let mut matches = readings
        .iter()
        .filter(|(n, _)| n.trim().eq_ignore_ascii_case(name.trim()));
    let (_, rpm) = matches.next()?;
    // Ambiguous names and sentinel values must never look like live RPM.
    (matches.next().is_none() && *rpm < u16::MAX.into()).then_some(*rpm)
}

impl Drop for Session {
    fn drop(&mut self) {
        // Covers command-channel disconnects, early returns, and unwinding as
        // well as a normal GUI Quit. The COM apartment is still alive here.
        for error in self.restore_owned() {
            eprintln!("Fan restore failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpm_matching_preserves_zero_and_rejects_missing_ambiguous_or_invalid_readings() {
        let mut readings = vec![("CPU Fan".into(), 760), ("Chassis Fan 1".into(), 0)];
        assert_eq!(match_rpm(&readings, "cpu fan"), Some(760));
        assert_eq!(match_rpm(&readings, "Chassis Fan 1"), Some(0));
        assert_eq!(match_rpm(&readings, "Chassis Fan 2"), None);
        readings.push(("CPU Fan".into(), 900));
        assert_eq!(match_rpm(&readings, "CPU Fan"), None);
        assert_eq!(match_rpm(&[("CPU Fan".into(), u32::MAX)], "CPU Fan"), None);
    }
}
