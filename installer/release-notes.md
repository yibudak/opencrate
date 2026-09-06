# OpenCrate release

## What's new

- Reduced idle memory and CPU use with cached software rendering. Closing the window to the tray releases rendering buffers while hardware controls remain active.
- Added System, Light and Dark appearance settings, with saved preferences and labels in English, Simplified Chinese and Turkish.
- Simplified the download and installation guide, with an application screenshot and a separate technical reference.
- Fixed Windows power API error handling and expanded automated quality, security and installer checks.

## Installation

Download `OpenCrate-<version>-windows-x64-setup.exe` to install.
The source-code archives are for developers and do not contain the installer.

- Native x64 Windows 10 (1809+) and Windows 11; per-user installation.
- RGB effects, speed and brightness; ASUS service fan controls; Windows power policies.
- English, Simplified Chinese and Turkish; tray controls and optional Windows startup.
- Lighting targets compatible ASUS USB `0B05:19AF` firmware. Fan control requires a compatible, running `AsusFanControlService`, which is not bundled. Armoury Crate itself is not required.
- The build is unsigned; Windows may show an unknown-publisher or SmartScreen prompt. A SHA-256 checksum is included.

Quit OpenCrate from its tray menu before upgrading. Saved preferences are preserved.

OpenCrate is an independent project and is not affiliated with, supported or endorsed by ASUS.
