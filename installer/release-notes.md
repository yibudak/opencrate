# OpenCrate release

## What's new

- Added **Settings → Updates** with automatic and manual GitHub release checks, release notes, download progress and cancellation in English, Simplified Chinese and Turkish.
- Updates download inside OpenCrate without opening a browser. Installer size and SHA-256 are verified before Setup can start; preferences are preserved and the app closes normally first.
- Fixed **Quit** from the tray waiting until the window was shown. Tray commands now reach the native event loop directly, preserving graceful hardware cleanup.

## Installation

Download `OpenCrate-<version>-windows-x64-setup.exe` to install.
The source-code archives are for developers and do not contain the installer.

- Native x64 Windows 10 (1809+) and Windows 11; per-user installation.
- RGB effects, speed and brightness; ASUS service fan controls; Windows power policies.
- English, Simplified Chinese and Turkish; tray controls and optional Windows startup.
- Lighting targets compatible ASUS USB `0B05:19AF` firmware. Fan control requires a compatible, running `AsusFanControlService`, which is not bundled. Armoury Crate itself is not required.
- The build is unsigned; Windows may show an unknown-publisher or SmartScreen prompt. A SHA-256 checksum is included.

For the first upgrade from 0.1.0 or 0.1.1, quit OpenCrate from its tray menu and run the installer manually. After installing 0.1.2, use **Settings → Updates** for future releases. Saved preferences are preserved.

OpenCrate is an independent project and is not affiliated with, supported or endorsed by ASUS.
