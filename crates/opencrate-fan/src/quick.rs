//! All-fan actions, preflight validation, and rollback of partial applies.

use crate::service::{manual_curve, Fan, Point};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickMode {
    FullBlast,
    Silent,
    Standard,
    Turbo,
}
impl QuickMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::FullBlast => "Full Blast",
            Self::Silent => "Silent",
            Self::Standard => "Standard",
            Self::Turbo => "Turbo",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Change {
    pub id: u32,
    pub name: String,
    pub before: Vec<Point>,
    pub after: Vec<Point>,
}

pub fn plan(fans: &[Fan], mode: QuickMode) -> Result<Vec<Change>, String> {
    if fans.is_empty() {
        return Err("No fans are available.".into());
    }
    let mut changes = Vec::new();
    for fan in fans {
        if !fan.writable {
            return Err(format!(
                "{} does not support this action. No fans were changed.",
                fan.name
            ));
        }
        if mode == QuickMode::FullBlast && fan.curve.iter().all(|p| p.duty == 255) {
            continue;
        }
        let after = if mode == QuickMode::FullBlast {
            manual_curve(fan, 100)?
        } else {
            fan.profiles
                .iter()
                .find(|p| p.name == mode.label())
                .ok_or_else(|| {
                    format!(
                        "{} has no {} profile. No fans were changed.",
                        fan.name,
                        mode.label()
                    )
                })?
                .curve
                .clone()
        };
        if fan.curve != after {
            changes.push(Change {
                id: fan.id,
                name: fan.name.clone(),
                before: fan.curve.clone(),
                after,
            });
        }
    }
    Ok(changes)
}

pub fn can_undo(changes: &[Change], fans: &[Fan]) -> bool {
    !changes.is_empty()
        && changes.iter().all(|c| {
            fans.iter()
                .any(|f| f.id == c.id && f.writable && f.curve == c.after)
        })
}

#[derive(Debug)]
pub struct ApplyError {
    pub message: String,
    pub rollback_failures: Vec<(u32, String)>,
}

pub fn execute(
    changes: &[Change],
    mut write: impl FnMut(u32, &[Point]) -> Result<(), String>,
) -> Result<(), ApplyError> {
    for (index, change) in changes.iter().enumerate() {
        if let Err(error) = write(change.id, &change.after) {
            let mut rollback_failures = Vec::new();
            // The failing call may have changed hardware before returning an
            // error. Restore it too, followed by previously written fans.
            for previous in changes[..=index].iter().rev() {
                if let Err(e) = write(previous.id, &previous.before) {
                    rollback_failures.push((previous.id, format!("{}: {e}", previous.name)));
                }
            }
            let recovery = if rollback_failures.is_empty() {
                "Previous settings restored.".into()
            } else {
                format!(
                    "Restore failed for {}",
                    rollback_failures
                        .iter()
                        .map(|(_, e)| e.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            };
            return Err(ApplyError {
                message: format!("{}: {error} {recovery}", change.name),
                rollback_failures,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::Profile;
    fn fans() -> Vec<Fan> {
        (1..=3)
            .map(|id| {
                let curve = [50, 70, 80, 85]
                    .into_iter()
                    .zip([51, 153, 204, 255])
                    .map(|(temperature, duty)| Point { temperature, duty })
                    .collect::<Vec<_>>();
                Fan {
                    id,
                    name: format!("Fan {id}"),
                    duty: 51,
                    minimum: 102,
                    writable: true,
                    can_restore: false,
                    profiles: vec![Profile {
                        index: id as i32 + 3,
                        name: "Silent".into(),
                        curve: curve.clone(),
                    }],
                    curve,
                }
            })
            .collect()
    }
    #[test]
    fn full_blast_sets_every_point_of_every_fan_to_255() {
        let fans = fans();
        let plan = plan(&fans, QuickMode::FullBlast).unwrap();
        assert_eq!(plan.len(), 3);
        for change in plan {
            assert!(change.after.iter().all(|p| p.duty == 255));
            assert_eq!(change.before[0].duty, 51);
        }
    }
    #[test]
    fn preflight_rejects_unsupported_fans_and_missing_profiles() {
        let mut fans = fans();
        fans[2].writable = false;
        assert!(plan(&fans, QuickMode::FullBlast).is_err());
        fans[2].writable = true;
        fans[1].profiles.clear();
        assert!(plan(&fans, QuickMode::Silent).is_err());
        assert!(plan(&[], QuickMode::FullBlast).is_err());
    }
    #[test]
    fn repeating_full_blast_is_a_no_op_and_preserves_the_previous_undo() {
        let mut fans = fans();
        for fan in &mut fans {
            for point in &mut fan.curve {
                point.duty = 255;
            }
        }
        assert!(plan(&fans, QuickMode::FullBlast).unwrap().is_empty());
    }
    #[test]
    fn profiles_are_resolved_per_fan_by_name() {
        let mut fans = fans();
        for f in &mut fans {
            f.profiles[0].curve[0].duty = 153;
        }
        let plan = plan(&fans, QuickMode::Silent).unwrap();
        assert_eq!(plan.len(), 3);
        assert!(plan.iter().all(|c| c.after[0].duty == 153));
    }
    #[test]
    fn partial_failure_restores_even_the_failing_fan_in_reverse_order() {
        let changes = plan(&fans(), QuickMode::FullBlast).unwrap();
        let mut writes = Vec::new();
        let mut actual = fans();
        let result = execute(&changes, |id, curve| {
            writes.push(id);
            actual[(id - 1) as usize].curve = curve.to_vec();
            if writes.len() == 2 {
                Err("Failed after write".into())
            } else {
                Ok(())
            }
        });
        assert!(result.unwrap_err().rollback_failures.is_empty());
        assert_eq!(writes, [1, 2, 2, 1]);
        assert!(actual.iter().zip(fans()).all(|(a, b)| a.curve == b.curve));
    }
    #[test]
    fn rollback_failures_are_reported_without_stopping_other_restores() {
        let changes = plan(&fans(), QuickMode::FullBlast).unwrap();
        let mut writes = Vec::new();
        let error = execute(&changes, |id, _| {
            writes.push(id);
            if id == 2 {
                Err("Offline".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(error.rollback_failures[0].0, 2);
        assert_eq!(writes, [1, 2, 2, 1]);
    }
    #[test]
    fn undo_does_not_overwrite_later_external_changes() {
        let mut fans = fans();
        let changes = plan(&fans, QuickMode::FullBlast).unwrap();
        for (fan, change) in fans.iter_mut().zip(&changes) {
            fan.curve = change.after.clone();
        }
        assert!(can_undo(&changes, &fans));
        fans[0].curve[0].duty = 200;
        assert!(!can_undo(&changes, &fans));
        assert!(!can_undo(&changes, &fans[1..]));
    }
}
