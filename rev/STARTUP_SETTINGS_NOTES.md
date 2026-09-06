# Startup and saved lighting settings

- The Settings section exposes Launch with Windows, Restore last lighting on
  launch, and Start in tray with Windows. Launch with Windows is opt-in;
  restoration and tray startup default to enabled.
- Settings are stored in `%APPDATA%\opencrate\settings.json`. Only successfully
  applied lighting configurations are remembered, including effect, base RGB,
  speed and brightness. Merely editing the palette or selecting a mode does not
  replace the last applied configuration.
- Saves are debounced by 400 ms and use a temporary file plus rename. Pending
  saves are flushed on normal shutdown. Invalid, out-of-range or unsupported
  saved settings do not reach hardware and are not overwritten just by loading.
- Disabling restoration still loads the last values into the controls but does
  not apply them to hardware. An empty first-run configuration does not write to
  the device either.
- Startup registration uses only the current user's `opencrate` value under
  `Software\Microsoft\Windows\CurrentVersion\Run`, with a quoted executable
  path and `--startup`. Updates and removal are read back. The current build's
  executable must remain at that path; toggle the setting off/on after moving it.
- The startup launch can hide in the tray. Manual launches show the window.
  A session-local mutex prevents competing instances; an event lets a second
  manual launch reveal the existing window. The GUI has no console window.
- Initial lighting restore can retry five times, two seconds apart, if the
  device is not ready. User lighting actions cancel pending restoration.

Primary Windows references:
[Run keys](https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys),
[CreateMutexW](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createmutexw).
Run starts at user sign-in, and Windows may delay its execution.

## Verification

Unit tests cover preference round trips, rejected invalid values, disabled
restoration, atomic file replacement, preservation of invalid files on load,
quoted Unicode paths and registry operations in an isolated test key outside
Windows Run. Installer tests cover migration and ownership of startup entries.
Windows sign-in behavior requires a separate interactive check.

## Tray window activation

Hidden windows cannot be relied on to produce an egui redraw when requested.
Window activation now uses the root HWND supplied by eframe and calls
[ShowWindowAsync](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-showwindowasync)
directly from the Show menu callback. Minimized windows are restored; egui also
receives visibility, restore and focus commands to synchronize its state. The
second-instance activation listener uses the same path. The handle is
invalidated on App drop so static menu callbacks cannot retain an active handle.
