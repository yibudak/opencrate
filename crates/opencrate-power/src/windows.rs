//! Native PowrProf API. GUID identity is independent of localized plan names.
use crate::*;
use std::ptr::{null, null_mut, NonNull};
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::{LocalFree, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS},
        System::{Power::*, Registry::REG_DWORD, SystemServices::*},
    },
};

#[derive(Default)]
pub struct Windows;

fn check(code: u32, operation: &str) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!(
            "{operation}: {}",
            std::io::Error::from_raw_os_error(code as i32)
        ))
    }
}
fn id(guid: GUID) -> PlanId {
    ((guid.data1 as u128) << 96)
        | ((guid.data2 as u128) << 80)
        | ((guid.data3 as u128) << 64)
        | u64::from_be_bytes(guid.data4) as u128
}

/// Copy and release a successful PowerGetActiveScheme response.
///
/// # Safety
/// On success, a non-null pointer must own an initialized GUID allocated by
/// LocalAlloc. On failure, the output is unspecified and must not be accessed.
unsafe fn consume_active_scheme(code: u32, pointer: *mut GUID) -> Result<PlanId, String> {
    check(code, "Read active Windows plan")?;
    let pointer = NonNull::new(pointer).ok_or("Windows did not return an active power plan.")?;
    // The successful API response owns this allocation until LocalFree below.
    let value = unsafe { id(pointer.read()) };
    unsafe { LocalFree(pointer.as_ptr().cast()) };
    Ok(value)
}

fn setting_guid(key: Setting) -> GUID {
    match key {
        Setting::Minimum => GUID_PROCESSOR_THROTTLE_MINIMUM,
        Setting::Maximum => GUID_PROCESSOR_THROTTLE_MAXIMUM,
        Setting::Boost => GUID_PROCESSOR_PERF_BOOST_MODE,
        Setting::EnergyPreference => GUID_PROCESSOR_PERF_ENERGY_PERFORMANCE_PREFERENCE,
    }
}

fn name(plan: PlanId) -> Result<String, String> {
    let guid = GUID::from_u128(plan);
    let mut bytes = 0;
    let code =
        unsafe { PowerReadFriendlyName(null_mut(), &guid, null(), null(), null_mut(), &mut bytes) };
    if code != ERROR_MORE_DATA {
        check(code, "Read power plan name")?;
    }
    if bytes == 0 || bytes > 65_536 || bytes % 2 != 0 {
        return Err("Invalid power plan name length.".into());
    }
    let mut buffer = vec![0u16; bytes as usize / 2];
    let capacity = bytes;
    check(
        unsafe {
            PowerReadFriendlyName(
                null_mut(),
                &guid,
                null(),
                null(),
                buffer.as_mut_ptr().cast(),
                &mut bytes,
            )
        },
        "Read power plan name",
    )?;
    if bytes > capacity || bytes % 2 != 0 {
        return Err("Invalid power plan name response.".into());
    }
    buffer.truncate(bytes as usize / 2);
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    Ok(String::from_utf16_lossy(&buffer[..end]))
}

fn plans() -> Result<Vec<Plan>, String> {
    let mut result = Vec::new();
    for index in 0..256 {
        let mut guid = GUID::from_u128(0);
        let mut bytes = std::mem::size_of::<GUID>() as u32;
        let code = unsafe {
            PowerEnumerate(
                null_mut(),
                null(),
                null(),
                ACCESS_SCHEME,
                index,
                (&mut guid as *mut GUID).cast(),
                &mut bytes,
            )
        };
        if code == ERROR_NO_MORE_ITEMS {
            return Ok(result);
        }
        check(code, "List Windows power plans")?;
        if bytes != std::mem::size_of::<GUID>() as u32 {
            return Err("Invalid power plan identifier.".into());
        }
        let id = id(guid);
        result.push(Plan {
            id,
            name: name(id).unwrap_or_else(|_| format!("Power plan {id:032x}")),
        });
    }
    Err("Windows returned too many power plans.".into())
}

fn allowed(key: Setting) -> Result<Allowed, String> {
    let guid = setting_guid(key);
    if key == Setting::Boost {
        let names = [
            "Disabled",
            "Enabled",
            "Aggressive",
            "Efficient enabled",
            "Efficient aggressive",
            "Aggressive at guaranteed",
            "Efficient aggressive at guaranteed",
        ];
        let mut choices = Vec::new();
        // Only expose documented modes that this Windows installation defines.
        for index in 0..=6 {
            let mut value = 0u32;
            let mut kind = 0;
            let mut bytes = 4;
            let code = unsafe {
                PowerReadPossibleValue(
                    null_mut(),
                    &GUID_PROCESSOR_SETTINGS_SUBGROUP,
                    &guid,
                    &mut kind,
                    index,
                    (&mut value as *mut u32).cast(),
                    &mut bytes,
                )
            };
            if code == 0 && kind == REG_DWORD && bytes == 4 && value <= 6 {
                choices.push((value, names[value as usize].into()));
            }
        }
        if choices.is_empty() {
            return Err("Windows does not expose supported boost modes.".into());
        }
        return Ok(Allowed::Choices(choices));
    }
    let (mut min, mut max, mut step) = (0, 0, 0);
    unsafe {
        check(
            PowerReadValueMin(
                null_mut(),
                &GUID_PROCESSOR_SETTINGS_SUBGROUP,
                &guid,
                &mut min,
            ),
            "Read processor setting minimum",
        )?;
        check(
            PowerReadValueMax(
                null_mut(),
                &GUID_PROCESSOR_SETTINGS_SUBGROUP,
                &guid,
                &mut max,
            ),
            "Read processor setting maximum",
        )?;
        check(
            PowerReadValueIncrement(
                null_mut(),
                &GUID_PROCESSOR_SETTINGS_SUBGROUP,
                &guid,
                &mut step,
            ),
            "Read processor setting increment",
        )?;
    }
    if min > max || max > 100 || step == 0 {
        return Err("Windows returned an unsupported processor setting range.".into());
    }
    Ok(Allowed::Range { min, max, step })
}

impl Backend for Windows {
    fn install_ultimate(&mut self) -> Result<PlanId, String> {
        let current = plans()?;
        if let Some(plan) = ultimate_plan(&current) {
            return Ok(plan);
        }
        let mut destination = GUID::from_u128(OPENCRATE_ULTIMATE);
        let mut pointer = &mut destination as *mut GUID;
        check(
            unsafe {
                PowerDuplicateScheme(
                    null_mut(),
                    &GUID::from_u128(ULTIMATE_PERFORMANCE),
                    &mut pointer,
                )
            },
            "Install Ultimate Performance",
        )?;
        if !plans()?.iter().any(|p| p.id == OPENCRATE_ULTIMATE) {
            return Err("Windows did not install the Ultimate Performance plan.".into());
        }
        Ok(OPENCRATE_ULTIMATE)
    }
    fn snapshot(&mut self) -> Result<Snapshot, String> {
        let active = self.active()?;
        let mut plans = plans()?;
        if !plans.iter().any(|p| p.id == active) {
            plans.push(Plan {
                id: active,
                name: name(active)?,
            });
        }
        let cpu = [Source::Ac, Source::Dc].map(|source| {
            let mut cpu = CpuSettings::default();
            for key in Setting::ALL {
                let control = (|| {
                    let value = self.read(active, source, key)?;
                    let allowed = allowed(key)?;
                    let access = if source == Source::Ac {
                        ACCESS_AC_POWER_SETTING_INDEX
                    } else {
                        ACCESS_DC_POWER_SETTING_INDEX
                    };
                    let write_error = check(
                        unsafe { PowerSettingAccessCheck(access, &setting_guid(key)) },
                        key.label(),
                    )
                    .err();
                    Ok::<_, String>(Control {
                        key,
                        value,
                        allowed,
                        write_error,
                    })
                })();
                match control {
                    Ok(control) => cpu.controls.push(control),
                    Err(error) => cpu.unavailable.push(format!("{}: {error}", key.label())),
                }
            }
            cpu
        });
        if self.active()? != active {
            return Err(
                "Windows changed the power plan while reading. Refresh to try again.".into(),
            );
        }
        let mut status = SYSTEM_POWER_STATUS::default();
        let ok = unsafe { GetSystemPowerStatus(&mut status) } != 0;
        let source = if ok {
            match status.ACLineStatus {
                0 => Some(Source::Dc),
                1 => Some(Source::Ac),
                _ => None,
            }
        } else {
            None
        };
        let mut capabilities = SYSTEM_POWER_CAPABILITIES::default();
        let capabilities_ok = unsafe { GetPwrCapabilities(&mut capabilities) };
        let role = unsafe { PowerDeterminePlatformRoleEx(2) };
        let has_battery = battery_available(
            role == PlatformRoleDesktop
                || role == PlatformRoleWorkstation
                || role == PlatformRoleEnterpriseServer,
            capabilities_ok.then_some((
                capabilities.SystemBatteriesPresent,
                capabilities.BatteriesAreShortTerm,
            )),
            ok.then_some(status.BatteryFlag),
        );
        let battery_percent = (has_battery
            && ok
            && status.BatteryFlag != 255
            && status.BatteryFlag & 128 == 0
            && status.BatteryLifePercent <= 100)
            .then_some(status.BatteryLifePercent);
        Ok(Snapshot {
            ultimate_plan: ultimate_plan(&plans),
            plans,
            active,
            cpu,
            source,
            battery_percent,
            has_battery,
            can_undo: false,
        })
    }
    fn active(&mut self) -> Result<PlanId, String> {
        let mut pointer = null_mut();
        let code = unsafe { PowerGetActiveScheme(null_mut(), &mut pointer) };
        // PowerGetActiveScheme supplies a LocalAlloc GUID only on success.
        unsafe { consume_active_scheme(code, pointer) }
    }
    fn activate(&mut self, plan: PlanId) -> Result<(), String> {
        check(
            unsafe { PowerSetActiveScheme(null_mut(), &GUID::from_u128(plan)) },
            "Activate Windows power plan",
        )
    }
    fn read(&mut self, plan: PlanId, source: Source, key: Setting) -> Result<u32, String> {
        let mut value = 0;
        let read = if source == Source::Ac {
            PowerReadACValueIndex
        } else {
            PowerReadDCValueIndex
        };
        check(
            unsafe {
                read(
                    null_mut(),
                    &GUID::from_u128(plan),
                    &GUID_PROCESSOR_SETTINGS_SUBGROUP,
                    &setting_guid(key),
                    &mut value,
                )
            },
            key.label(),
        )?;
        Ok(value)
    }
    fn write(
        &mut self,
        plan: PlanId,
        source: Source,
        key: Setting,
        value: u32,
    ) -> Result<(), String> {
        let write = if source == Source::Ac {
            PowerWriteACValueIndex
        } else {
            PowerWriteDCValueIndex
        };
        check(
            unsafe {
                write(
                    null_mut(),
                    &GUID::from_u128(plan),
                    &GUID_PROCESSOR_SETTINGS_SUBGROUP,
                    &setting_guid(key),
                    value,
                )
            },
            key.label(),
        )
    }
}

fn ultimate_plan(plans: &[Plan]) -> Option<PlanId> {
    // A copied Windows template has a new GUID. Its localized friendly name is
    // only a discovery hint; activation always uses the enumerated GUID.
    let template_name = name(ULTIMATE_PERFORMANCE).ok();
    find_ultimate_plan(plans, template_name.as_deref())
}

fn find_ultimate_plan(plans: &[Plan], template_name: Option<&str>) -> Option<PlanId> {
    plans
        .iter()
        .find(|p| p.id == ULTIMATE_PERFORMANCE || p.id == OPENCRATE_ULTIMATE)
        .or_else(|| {
            plans.iter().find(|p| {
                !matches!(p.id, BALANCED | POWER_SAVER | HIGH_PERFORMANCE)
                    && template_name == Some(p.name.as_str())
            })
        })
        .map(|p| p.id)
}

fn battery_available(desktop: bool, capabilities: Option<(bool, bool)>, flag: Option<u8>) -> bool {
    if desktop {
        return false;
    }
    if let Some((present, short_term)) = capabilities {
        return present && !short_term;
    }
    flag.is_some_and(|f| f != 255 && f & 128 == 0)
}

#[cfg(test)]
mod pointer_tests {
    use super::*;
    use windows_sys::Win32::{
        Foundation::ERROR_ACCESS_DENIED,
        System::Memory::{LocalAlloc, LMEM_FIXED},
    };

    #[test]
    fn ultimate_discovery_accepts_localized_copies_but_not_renamed_standard_plans() {
        let mut plans = vec![Plan {
            id: BALANCED,
            name: "Nihai Performans".into(),
        }];
        assert_eq!(find_ultimate_plan(&plans, Some("Nihai Performans")), None);
        plans.push(Plan {
            id: 123,
            name: "Nihai Performans".into(),
        });
        assert_eq!(
            find_ultimate_plan(&plans, Some("Nihai Performans")),
            Some(123)
        );
        plans.push(Plan {
            id: OPENCRATE_ULTIMATE,
            name: "Custom name".into(),
        });
        assert_eq!(find_ultimate_plan(&plans, None), Some(OPENCRATE_ULTIMATE));
    }

    #[test]
    fn battery_detection_distinguishes_desktops_ups_laptops_and_unknown_status() {
        assert!(!battery_available(true, Some((true, false)), Some(1)));
        assert!(!battery_available(false, Some((true, true)), Some(1)));
        assert!(!battery_available(false, Some((false, false)), Some(128)));
        assert!(battery_available(false, Some((true, false)), Some(255)));
        assert!(battery_available(false, None, Some(8)));
        assert!(!battery_available(false, None, Some(128)));
        assert!(!battery_available(false, None, Some(255)));
        assert!(!battery_available(false, None, None));
    }

    #[test]
    fn failed_response_never_reads_or_frees_unspecified_output() {
        // This address cannot be read or freed. A failed API call does not own it.
        let unspecified = NonNull::<GUID>::dangling().as_ptr();
        let result = unsafe { consume_active_scheme(ERROR_ACCESS_DENIED, unspecified) };
        assert!(result.unwrap_err().starts_with("Read active Windows plan:"));
    }

    #[test]
    fn successful_response_still_requires_a_non_null_guid() {
        let result = unsafe { consume_active_scheme(0, null_mut()) };
        assert_eq!(
            result.unwrap_err(),
            "Windows did not return an active power plan."
        );
    }

    #[test]
    fn successful_response_copies_a_local_allocation() {
        let expected = 0x11223344_5566_7788_99aa_bbccddeeff00;
        let pointer = unsafe { LocalAlloc(LMEM_FIXED, std::mem::size_of::<GUID>()).cast::<GUID>() };
        assert!(!pointer.is_null());
        unsafe { pointer.write(GUID::from_u128(expected)) };
        // Ownership is transferred to the consumer; this test never reads it again.
        assert_eq!(
            unsafe { consume_active_scheme(0, pointer) }.unwrap(),
            expected
        );
    }
}
