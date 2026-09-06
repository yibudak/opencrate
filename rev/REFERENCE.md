# OpenCrate technical reference

For installation and a quick introduction, see the [README](../README.md).
This guide covers detailed controls, hardware support and development.
For packaging, see the [installer guide](../installer/README.md).

## Features

- **Lighting:** synchronized RGB control, 17 effects, a color picker, brightness adjustment and animation speed from 0.25× to 4×.
- **Cooling:** available ASUS cooling profiles, manual fan speed, custom temperature curves, Full Blast and other actions for all fans, with restoration and undo controls.
- **Power:** installed Windows power plans, separate plugged-in and battery processor settings, boost mode, energy preference and undo for the last change.
- **Desktop integration:** notification-area controls, optional launch at Windows sign-in, optional tray startup and restoration of the last applied lighting settings.
- **Languages:** English (default), Simplified Chinese and Turkish. Change the language in **Settings → Application language**; it applies immediately and is remembered for the next launch.

## Hardware support

Lighting currently targets the ASUS USB controller `0B05:19AF` and its supported four-channel firmware layout. Matching the vendor name alone does not establish compatibility; other controllers and firmware layouts are not supported by this backend. This is an evolving alternative with a smaller hardware and feature set than Armoury Crate.

Armoury Crate itself is not required. Lighting uses the controller's HID interface, and all lights are synchronized. **Fan control requires a compatible `AsusFanControlService` installation to be running.** OpenCrate does not install this service; uninstalling ASUS software may remove it. Without it, fan controls are unavailable, while supported RGB and Windows power controls remain independent. Unsupported fans remain read-only. Fan output is shown as duty percentage, with no live RPM reading. Custom curves preserve the controller's minimum duty and critical-temperature requirements.

Power control uses Windows power-policy APIs. Available settings depend on Windows, hardware and permissions. Processor percentages are performance policies, not CPU watt limits; the app does not expose voltage, PPT/TDC/EDC or BIOS tuning.

## Build and run

Use Windows with Rust and the MinGW-w64 GNU linker/resource compiler (`gcc` and `windres`) on PATH. Select the GNU Windows toolchain for this directory with the commands below. `rust-toolchain.toml` uses native stable Rust for compatibility with Linux tooling and Dependabot. GitHub Actions installs and selects the Windows build tools automatically.

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal --component rustfmt --component clippy
rustup override set stable-x86_64-pc-windows-gnu
cargo build --locked --release -p opencrate-ui
.\target\release\opencrate-ui.exe
```

For development:

```powershell
cargo run --locked -p opencrate-ui
cargo test --locked --workspace
cargo clippy --locked -p opencrate-ui --all-targets -- -D warnings
```

Fonts, translations and icons are embedded in the executable. No language pack or network connection is needed to display the interface. The first Cargo build downloads dependencies unless they are already cached.

## Settings and application lifetime

Preferences are stored in `%APPDATA%\opencrate\settings.json`. They include `language` (`en`, `zh-CN` or `tr`), tray startup, lighting restoration and the last successfully applied lighting configuration. Existing files without a language field continue to work in English; an unknown language also falls back to English.

Closing the window keeps OpenCrate running in the tray. Choose **Show OpenCrate** to reopen it or **Quit** to exit.

- Lighting animations continue in the tray. On Quit, lighting falls back to a built-in controller effect; dimmed multicolor animations remain as a static color.
- Fan changes are temporary. Normal Quit restores the original curves that OpenCrate still owns. Fan settings are not reapplied at startup.
- Applied power settings persist in Windows after Quit and restart. Undo history lasts only for the current session.

**Launch with Windows** registers the current executable under the current user's Windows Run key. If you move the executable, disable and re-enable this option from the new location.

## Project layout

| Directory | Purpose |
| --- | --- |
| `crates/opencrate-ui` | Desktop interface, translations, tray and preferences |
| `crates/opencrate-core` | Shared effect identifiers and RGB types |
| `crates/opencrate-aura` | Lighting protocol, animation and playback |
| `crates/opencrate-fan` | ASUS fan service integration and cooling actions |
| `crates/opencrate-power` | Windows power policy integration |
| `crates/opencrate-cli`, `crates/opencrate-probe` | Command-line tools and hardware diagnostics |
| `assets/locales` | Embedded English, Simplified Chinese and Turkish catalogs |
| `assets/fonts` | Bundled CJK fallback font and license |
| `rev` | Protocol and implementation documentation |

## Contributing translations

See [CONTRIBUTING.md](../CONTRIBUTING.md) for development setup, portable tests,
dependency policy and the pull request process. Bug reports and feature proposals
have [issue templates](https://github.com/yibudak/opencrate/issues/new/choose).
Follow the [code of conduct](../CODE_OF_CONDUCT.md) and report vulnerabilities through
the [private security process](../SECURITY.md).

Keep code, comments and documentation in English. User-visible translations belong in `assets/locales/en.json`, `zh-CN.json` and `tr.json`. English source strings are the catalog keys; update all three catalogs together and preserve named placeholders such as `{percent}`. Display labels must never replace serialized effect IDs, fan IDs or Windows power-plan GUIDs.

Catalog tests check key and placeholder parity, effect/menu coverage and font coverage. Preference tests cover migration and saved-language round trips. Windows-owned names, user-defined plan names and low-level diagnostics retain their original text; the surrounding UI is translated. Some built-in color-picker tooltips are supplied in English by egui.

## Automated builds and releases

Pull requests, pushes to `main` and manual workflow runs check Rust formatting,
Clippy, unit tests and Rustdoc; workflow, Python, PowerShell and documentation
quality; dependency advisories, licenses and sources; and CodeQL analysis for
Rust, Python and Actions. Linux jobs run portable tests and publish an LCOV report.
Windows jobs build and test the installer. Fork PRs run without repository secrets.

Passing runs provide a setup executable and checksum as workflow artifacts.
A `v<major>.<minor>.<patch>` tag matching `Cargo.toml` publishes those files to
GitHub Releases only after all quality, security and Windows checks succeed.
Weekly checks and Dependabot updates help catch dependency changes. See
[release instructions](../installer/README.md#github-actions-and-releases).

## Privacy when contributing

Do not commit personal settings, device captures, firmware dumps, log files, credentials or local build outputs. Diagnostic tools can print device paths and custom power-plan names; review their output before sharing it. Public tests use synthetic fixtures. The build remaps source paths and checks release payloads for local user directories before packaging.

## Credits

OpenRGB protocol research informed the lighting implementation; supporting notes and references are in `rev`. OpenCrate's own branding assets are documented in `assets/branding/README.md`. The bundled Noto Sans SC font uses the SIL Open Font License; see [font attribution](../assets/fonts/README.md) and [license](../assets/fonts/OFL-NotoSansSC.txt).
