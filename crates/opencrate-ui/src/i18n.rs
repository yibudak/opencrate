//! Embedded UI translations. Hardware identifiers and backend diagnostics stay in English.
//! Only the UI thread selects the language; workers never localize hardware messages.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU8, Ordering},
        OnceLock,
    },
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[serde(rename = "zh-CN")]
    Chinese = 1,
    #[serde(rename = "tr")]
    Turkish = 2,
    #[default]
    #[serde(rename = "en", other)]
    English = 0,
}
impl Language {
    pub const ALL: [Self; 3] = [Self::English, Self::Chinese, Self::Turkish];
    /// Native names remain recognizable after changing languages.
    pub fn name(self) -> &'static str {
        self.translate("English")
    }
    pub fn translate(self, key: &str) -> &str {
        let index = self as usize;
        catalog()[index].get(key).map_or(key, String::as_str)
    }
}
static LANGUAGE: AtomicU8 = AtomicU8::new(0);
type Catalog = [BTreeMap<String, String>; 3];
const RESOURCES: [&str; 3] = [
    include_str!("../../../assets/locales/en.json"),
    include_str!("../../../assets/locales/zh-CN.json"),
    include_str!("../../../assets/locales/tr.json"),
];
fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        RESOURCES
            .map(|json| serde_json::from_str(json).expect("valid embedded translation catalog"))
    })
}
pub fn set_language(language: Language) {
    LANGUAGE.store(language as u8, Ordering::Relaxed);
}
pub fn current() -> Language {
    match LANGUAGE.load(Ordering::Relaxed) {
        1 => Language::Chinese,
        2 => Language::Turkish,
        _ => Language::English,
    }
}
pub fn t(key: &str) -> &str {
    current().translate(key)
}

/// Replace named placeholders in one pass, so values containing braces are literal.
pub fn format_for(language: Language, key: &str, args: &[(&str, String)]) -> String {
    let template = language.translate(key);
    let mut rest = template;
    let mut result = String::new();
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}').map(|end| start + end) else {
            result.push_str(&rest[start..]);
            return result;
        };
        let name = &rest[start + 1..end];
        if let Some((_, value)) = args.iter().find(|(key, _)| *key == name) {
            result.push_str(value);
        } else {
            result.push_str(&rest[start..=end]);
        }
        rest = &rest[end + 1..];
    }
    result.push_str(rest);
    result
}
pub fn f(key: &str, args: &[(&str, String)]) -> String {
    format_for(current(), key, args)
}

/// Store the source key and values so existing status messages switch language too.
#[derive(Clone)]
pub struct Message {
    key: &'static str,
    args: Vec<(&'static str, String)>,
}
impl Message {
    pub fn text(key: &'static str) -> Self {
        Self { key, args: vec![] }
    }
    pub fn with(key: &'static str, args: Vec<(&'static str, String)>) -> Self {
        Self { key, args }
    }
    pub fn render(&self) -> String {
        // Known diagnostic sentences can be translated at display time too.
        // Native error codes and unknown technical details remain verbatim.
        let args = self
            .args
            .iter()
            .map(|(name, value)| {
                (
                    *name,
                    if *name == "details" {
                        t(value).to_string()
                    } else {
                        value.clone()
                    },
                )
            })
            .collect::<Vec<_>>();
        f(self.key, &args)
    }
}

/// Windows returns localized plan names. Standard GUIDs have our own UI names;
/// custom names are user data and must not be translated or used as identifiers.
pub fn plan_name(plan: &opencrate_power::Plan) -> &str {
    use opencrate_power::{BALANCED, HIGH_PERFORMANCE, POWER_SAVER};
    match plan.id {
        BALANCED => t("Balanced"),
        HIGH_PERFORMANCE => t("High performance"),
        POWER_SAVER => t("Power saver"),
        _ => &plan.name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn placeholders(text: &str) -> Vec<&str> {
        let mut found = text
            .split('{')
            .skip(1)
            .filter_map(|s| s.split_once('}').map(|(name, _)| name))
            .collect::<Vec<_>>();
        found.sort();
        found
    }
    #[test]
    fn every_language_has_all_keys_and_matching_placeholders() {
        let catalogs = catalog();
        for translated in &catalogs[1..] {
            assert_eq!(
                catalogs[0].keys().collect::<Vec<_>>(),
                translated.keys().collect::<Vec<_>>()
            );
            for (key, value) in translated {
                assert!(!value.trim().is_empty(), "Empty translation: {key}");
                assert_eq!(
                    placeholders(key),
                    placeholders(value),
                    "Placeholder mismatch: {key}"
                );
            }
        }
        for (key, value) in &catalogs[0] {
            assert_eq!(key, value);
        }
    }
    #[test]
    fn unknown_keys_and_languages_fall_back_to_english() {
        assert_eq!(Language::Chinese.translate("Future label"), "Future label");
        assert_eq!(
            serde_json::from_str::<Language>("\"future\"").unwrap(),
            Language::English
        );
    }
    #[test]
    fn placeholders_allow_reordering_and_never_reinterpret_values() {
        let output = format_for(
            Language::Turkish,
            "Point {number} temperature",
            &[("number", "{literal}".into())],
        );
        assert!(output.contains("{literal}"));
        assert!(!output.contains("{number}"));
    }

    #[test]
    fn dynamic_labels_have_translations_and_fonts_cover_every_catalog() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            ctx.fonts_mut(|fonts| {
                for language in Language::ALL {
                    let labels = &catalog()[language as usize];
                    for effect in opencrate_core::EffectMode::all() {
                        assert!(labels.contains_key(effect.display_name()));
                        assert_eq!(
                            effect.name().parse::<opencrate_core::EffectMode>().unwrap(),
                            *effect
                        );
                    }
                    for preset in crate::PRESETS {
                        assert!(labels.contains_key(preset.name));
                    }
                    for value in labels.values() {
                        assert!(
                            fonts.has_glyphs(&egui::FontId::proportional(14.0), value),
                            "Missing glyphs: {value}"
                        );
                    }
                }
            });
        });
    }
}
