# Windows installer

OpenCrate ships as a single Inno Setup executable for native x64 Windows 10 (1809 or later) and Windows 11. The installation is per user, under `%LOCALAPPDATA%\Programs\OpenCrate`, with no elevation request. The app currently accesses supported HID devices, the existing `AsusFanControlService` and Windows power APIs without installing its own service or driver.

## Build

Run from PowerShell with Rust, Python 3 and MinGW-w64 (`gcc` and `windres`) available:

```powershell
.\scripts\build-installer.ps1 -BootstrapInnoSetup
```

The bootstrap option selects Inno Setup 6.7.3 even if another version is installed. Its first run downloads the compiler from its upstream release, verifies the pinned SHA-256 and Authenticode publisher, and extracts it in portable mode under `target/installer-tools`. It does not register the compiler, associate files or change PATH. Alternatively, supply `-IsccPath` with a compatible Inno Setup compiler. The bootstrap option is only for developers; end users do not need build tools.

The build compiles `opencrate-ui` for `x86_64-pc-windows-gnu`, verifies its x64 PE architecture and version resource, and packages its executable with the README and license notices. Fonts, translations and icons are already embedded. Output files:

```text
dist/OpenCrate-<version>-windows-x64-setup.exe
dist/OpenCrate-<version>-windows-x64-setup.exe.sha256
```

Use `-SkipBuild` to repackage the existing target-specific release binary after documentation or installer-only changes. Increment `workspace.package.version` in `Cargo.toml` for new releases; the executable, installer and Windows installed-app entry use this value. Keep the installer `AppId` unchanged so subsequent releases update the existing installation. Generated files and compiler downloads are ignored by Git.

## Installation behavior

- English, Simplified Chinese and Turkish setup and uninstall messages. Installer language follows Windows initially and can be changed; application language retains its own saved preference and defaults to English on first run.
- A Start menu shortcut and an optional desktop shortcut; a Windows Installed apps entry and uninstaller.
- The running-app mutex blocks installation and removal while OpenCrate is active. The message explains how to Quit from the tray. Setup never force-kills the app or asks Windows to restart it, preserving the normal fan-restoration path.
- Upgrades retain settings. An already enabled, recognized OpenCrate startup command is updated to the installed executable. Fresh installations do not opt users into Windows startup.
- Uninstall removes the installed files and shortcuts, and removes the startup entry only if it still points at this installation. `%APPDATA%\opencrate\settings.json` is preserved. Applied Windows power policies are not reset by the uninstaller.
- No ASUS drivers or services are bundled. Fan support still requires a compatible, running `AsusFanControlService`. An installation information page explains current hardware support and project independence.

## GitHub Actions and releases

The Windows workflow checks source privacy, formatting, unit tests and Clippy,
then builds and tests the installer on a fresh Windows runner. Installer checks
do not launch the app or touch physical hardware. Successful runs upload only
the setup executable and its SHA-256 checksum, with a 14-day artifact lifetime.
Source paths are remapped during compilation, release symbols are stripped,
and the unpacked payload is checked before Inno Setup compresses it. A reused
binary supplied with `-SkipBuild` must pass the same payload privacy check.

Publication also requires repository lint, dependency policy, portable tests and
CodeQL checks to pass. Repeated publication runs verify an existing public release
and leave its assets untouched. Interrupted draft uploads can resume; the installer
is downloaded and checked against the tested build before the draft becomes public.
New releases record their source tree in the release notes to reject conflicting
drafts or retagged source. Legacy releases without that record still have their
asset names and checksums verified. Main and tag runs finish normally; only an
older PR run is cancelled when a newer commit supersedes it.

To publish a release:

1. Update `workspace.package.version` in `Cargo.toml`, keep internal workspace
   dependency requirements compatible, and refresh `Cargo.lock`.
2. Commit and push the reviewed changes to `main` and check the workflow result.
3. Create a lightweight `v<major>.<minor>.<patch>` tag on that commit and push it.
4. The tag workflow requires an exact package-version match. Once all checks
   pass, it publishes the tested setup and checksum as a GitHub Release.

For example, use `git tag v0.1.0` followed by `git push origin v0.1.0` for the
initial version. Release files remain available independently of workflow
artifact expiration. Published tags and assets should not be overwritten;
release a new version for changes. No signing credentials are required for
the current unsigned build pipeline.

## Verification

Close OpenCrate using its tray menu. On an account with no installed OpenCrate copy or existing OpenCrate shortcuts, run:

```powershell
.\scripts\test-installer.ps1 -InstallerPath .\dist\OpenCrate-0.1.0-windows-x64-setup.exe
```

This test installs into a path containing spaces under `target`, reinstalls over the same AppId, checks shortcuts and registration, exercises all three setup languages, and uninstalls twice. It verifies startup migration, no opt-in on a fresh install, cleanup of an owned startup entry, preservation of an entry belonging to another copy, and byte-for-byte preservation of the settings file. The test restores the user's original startup entry unless it changed externally during the test. It does not launch the app or modify hardware settings. Logs remain under `target/installer smoke test`.

## Signing and distribution

This build is unsigned. Windows may show an unknown-publisher or SmartScreen reputation prompt when users download it. The checksum detects changes to a downloaded file; it does not replace publisher signing. For a public signed release, sign the payload executable and configure Inno Setup's SignTool/SignedUninstaller with the project's own trusted code-signing certificate, then sign the setup executable and regenerate its checksum. No certificate or signing credentials are included in this repository.

Automated installation checks run on a fresh GitHub-hosted Windows runner. Hardware behavior requires separate testing on compatible devices; the installer does not expand hardware compatibility.

## Build inputs and third-party notices

Chinese setup messages come from the official Inno Setup translation at commit `6ef32198ef1f7b7b375cd4b6b90896c2a58eb4c2`; SHA-256: `e0b0b350e2245f3c5e65586dfe43d574f6e7f06f2261149aba284954b3fc9a8d`. The translation's upstream comments and attribution are retained. English and Turkish base messages come with the pinned compiler; OpenCrate-specific translations are under `locales`.

`generate-notices.ps1` walks enabled normal and build dependencies for the UI target, collecting their packaged license texts and embedded font licenses. A few Cargo packages omit the repository-level license files; `licenses/supplements.json` maps those exact package versions to upstream texts, with source revisions inside each supplement. The build stops on missing notices. Review this map when changing dependencies. Inno Setup's license is included with the notices.

References: [Inno Setup downloads](https://jrsoftware.org/isdl.php), [AppMutex](https://jrsoftware.org/is6help/topic_setup_appmutex.htm), [portable compiler mode](https://github.com/jrsoftware/issrc/blob/is-6_7_3/isportable.iss), [official translations](https://jrsoftware.org/files/istrans/).
