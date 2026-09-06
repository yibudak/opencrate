# Interface localization

- English is the default. Simplified Chinese (`zh-CN`) and Turkish (`tr`) can be selected under Settings â†’ Application language.
- Selection updates the UI and existing tray menu items immediately. Menu IDs and event handlers remain stable. The choice is saved alongside lighting preferences and loaded before constructing the UI and tray.
- English source strings are keys in three embedded JSON catalogs. Named placeholders support translation-specific ordering without interpreting braces inside inserted values. Missing keys and unknown saved languages fall back to English.
- Status messages retain their source keys so switching languages updates previously displayed feedback. Standard Windows power plans use their GUIDs for translated labels; custom plan names remain untouched. Hardware IDs, command-line output, protocol data and backend diagnostic text stay in English or their original source language.
- Noto Sans SC is embedded as a fallback font, with its SIL Open Font License. Chinese does not require a Windows language pack or a system font installation.
- Existing preference files retain their lighting, restore and tray settings. Selecting a language does not enqueue any lighting, fan or power command and does not rebuild their editing state.

## Verification

Unit tests check catalog and placeholder parity, effect and tray labels,
glyph coverage, legacy preference migration, language fallback and saved-language
round trips. Test preferences are synthetic and do not contain runtime settings.

Windows window decorations and custom Windows plan names follow their source language. A few built-in egui color-picker tooltips are English. Low-level diagnostic details remain verbatim, with translated context around app error messages.
