# OpenCrate release

## What's new

- Improved desktop behavior: the window redraws while resizing, a single left click on the tray icon reopens it, and display text no longer becomes selected. **Settings → About** now includes developer and repository links.
- Expanded fan controls with measured RPM, draggable curve points, numeric curve editing, saved fan groups and explicit one-time Apply actions with Undo. Minimum duty now respects valid controller curves while preserving thermal protection.
- Updated the Power page for desktops and UPS systems, added Ultimate Performance support, and introduced Quiet, Everyday and Responsive processor-policy presets with review, Apply and Undo.
- Added live CPU usage, frequency, package power and temperature with 60-second charts. Windows supplies usage and a labeled frequency estimate; package power and temperature require a separately running LibreHardwareMonitor or OpenHardwareMonitor provider with administrator access and WMI enabled.
- Updated English, Simplified Chinese and Turkish translations and the fan and power documentation.

Included changes: [#10](https://github.com/yibudak/opencrate/pull/10), [#11](https://github.com/yibudak/opencrate/pull/11), [#12](https://github.com/yibudak/opencrate/pull/12).

## Installation

Download `OpenCrate-<version>-windows-x64-setup.exe` to install.
The source-code archives are for developers and do not contain the installer.

- Native x64 Windows 10 (1809+) and Windows 11; per-user installation.
- RGB effects, speed and brightness; ASUS service fan controls; Windows power policies.
- English, Simplified Chinese and Turkish; tray controls and optional Windows startup.
- Lighting targets compatible ASUS USB `0B05:19AF` firmware. Fan control requires a compatible, running `AsusFanControlService`, which is not bundled. Armoury Crate itself is not required.
- The build is unsigned; Windows may show an unknown-publisher or SmartScreen prompt. A SHA-256 checksum is included.

From 0.1.2 or later, use **Settings → Updates** to download and install this release. From 0.1.0 or 0.1.1, quit OpenCrate from its tray menu and run the installer manually. Saved preferences are preserved.

OpenCrate is an independent project and is not affiliated with, supported or endorsed by ASUS.
