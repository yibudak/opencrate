# Windows Power controls

Implemented in `opencrate-power` and `opencrate-ui/src/power.rs`.

## Available controls

- Enumerate installed Windows plans using their stable GUIDs and localized names.
- Immediate Power saver, Balanced and High performance buttons, when those plans exist.
- Plan selector includes custom plans when present.
- Processor minimum/maximum state (0â€“100%), boost mode, and energy preference (0â€“100%).
- Separate AC (plugged in) and DC (battery) policies. Edits are drafts until Apply.
- Undo the last successful action in this application session.

These are Windows policies, not CPU package watt limits or live power measurements.
CPU/firmware support and the Windows power mode influence the resulting behavior.
No PPT/TDC/EDC, voltage, SMU, BIOS or ASUS WMI writes are made. No additional driver,
ASUS service or elevation prompt is needed for power control; Windows can still deny
writes via policy or permissions, which are reported by the UI.

The selected plan and applied values persist in Windows across application exit and
Windows restarts. OpenCrate does not reapply power settings at startup or on refresh.
Undo history is in memory and is lost when OpenCrate quits. Fan restoration behavior
and saved lighting preferences are independent of power policy persistence.

## API and failure handling

Native `PowrProf.dll` calls through `windows-sys`, with no shell output parsing.
Plan enumeration is bounded, UTF-16 names use byte-counted buffers, and the GUID
allocated by PowerGetActiveScheme is released with LocalFree. Processor ranges,
increments, boost choices and policy access are read from Windows. Unavailable
settings are reported individually; installed-plan selection remains usable.

Commands are serialized on a background thread. Every three seconds a read-only
snapshot refreshes the active plan, source and CPU policies. A single-slot mailbox
retains the latest snapshot and unconsumed action acknowledgement while in the tray.

Before Apply, the active GUID and all displayed source-specific values are compared
with a fresh snapshot. Invalid ranges, min > max, unknown choices, duplicate keys,
read-only policies and stale drafts are rejected before mutation. Only changed
values are written. The active scheme is reapplied to make the writes effective;
the active GUID and each changed value are then read back.

Failure attempts to restore every attempted setting, including a setter that failed
after writing. Min/max writes are ordered to keep intermediate ranges valid. Partial
restore failures are reported. An external active-plan selection is left in place.
Undo checks its last applied values before restoring and preserves unrelated settings.
Once an external conflict has been observed, stale Undo ownership is discarded.
Windows does not offer an atomic compare-and-swap across these APIs; another utility
writing concurrently during the short write sequence can still cause a conflict.

## Verification

Unit tests exercise stale drafts, source isolation, invalid settings, activation
failures, partial rollback, readback mismatches and external changes using mocks.
Hardware diagnostics are separate from CI. The following command is read-only;
its output may include custom plan names and identifiers and should be reviewed
before sharing:

```powershell
cargo run --locked -p opencrate-power --example windows_probe
```

The optional `--test-roundtrip` flag deliberately changes and restores policies;
it must only be used when hardware testing is intended.

## Primary references

- [PowerEnumerate](https://learn.microsoft.com/en-us/windows/win32/api/powrprof/nf-powrprof-powerenumerate)
- [PowerSetActiveScheme](https://learn.microsoft.com/en-us/windows/win32/api/powersetting/nf-powersetting-powersetactivescheme)
- [PowerWriteACValueIndex](https://learn.microsoft.com/en-us/windows/win32/api/powersetting/nf-powersetting-powerwriteacvalueindex): active-plan changes need PowerSetActiveScheme to take effect.
- [PowerReadPossibleValue](https://learn.microsoft.com/en-us/windows/win32/api/powrprof/nf-powrprof-powerreadpossiblevalue)
- [Processor boost mode](https://learn.microsoft.com/en-us/windows-hardware/customize/power-settings/options-for-perf-state-engine-perfboostmode)
- [Energy preference](https://learn.microsoft.com/en-us/windows-hardware/customize/power-settings/options-for-perf-state-engine-perfenergypreference)
