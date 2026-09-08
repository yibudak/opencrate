# ASUS fan service integration

Fan control requires a compatible, installed and running `AsusFanControlService`.
OpenCrate does not bundle or install this proprietary service. Armoury Crate's
interface need not be running, but removing ASUS software may remove the service.
The RGB and Windows power backends do not depend on it.

## COM interface contract

The Windows backend activates the service as a local COM server:

| Contract | Identifier |
| --- | --- |
| FanControlManager class | `{14083c53-b8e7-48e4-9320-811f3478c4a4}` |
| IFanControlManager | `{bb327645-e3ec-4b7d-90ee-67fcb949e584}` |
| IFanControl | `{d20e7b8f-c878-4538-b624-9bc08fe7d54f}` |
| IFanCurve | `{e6bb07b4-f360-45af-814e-eb0d12d55bec}` |

These are interface identifiers, not identifiers assigned to an individual PC.
Controls are enumerated by their 32-bit `Id`. The service supplies names,
profiles, current curves, curve point counts and minimum duty. `DutyCycle` and
`MinimalDuty` are raw byte values; the GUI converts them to percentages.

RPM comes from the optional installed ASUS `aaHM.acpiHmData2` COM provider
(`{2627f8be-4482-4081-bc62-8a12ca24bdf8}`). Each snapshot calls `Refresh`, enumerates
`Sensors`, and matches a unique sensor `name` to the control's canonical `Name`,
not its editable `DisplayName` or collection position. The sensor's `current`
value is the measured RPM; it is never estimated from duty. Zero RPM is retained.
Missing providers, read failures, ambiguous names and invalid readings display
as unavailable without disabling duty readings or fan control. Retry connection
also reconnects this provider. No sensor settings are written.

All COM objects stay on a dedicated MTA worker. Requests use a command channel;
periodic readback uses a single-slot mailbox holding the newest snapshot and
unconsumed acknowledgement. Opening the app only reads fans.

## Applying curves

The backend edits the `CurrentFanCurve` buffer, calls `ApplyFanCurveButNotSave`,
checks both the method result and HRESULT, and reads the complete curve back.
Editing the buffer alone is not treated as a successful hardware apply. Failure
attempts to restore the pre-operation curve. Named profiles use the same path
when copying the profile's curve.

No direct `DutyCycle` setter, `ApplyIndex`, `EnableManualMode`, WMI setter,
FanStore.xml write or BIOS write is used. Manual speed is a thermal curve that
holds the requested duty at low temperatures and reaches full duty at the final
thermal point, no later than 85 degrees Celsius. Curve validation preserves the
controller's supported minimum duty and critical temperature, requires
nondecreasing points and ends at full duty. Some services report a conservative
`MinimalDuty` even while an accepted BIOS curve runs lower. The effective minimum
includes the lowest nonzero duty in valid current, saved/profile and original
session curves. A valid 20% BIOS curve therefore permits 20% custom control even
when `MinimalDuty` reports 40%. Invalid curves, instantaneous duty readings and
fan-stop points cannot lower the limit. The original curve preserves this
capability during a temporary full-speed apply. Hardware write/readback checks
still determine whether a requested curve is accepted. Unsupported modes or
point counts remain read-only.

Quick actions explicitly apply once to all discovered controls; they are buttons,
not persistent toggles or repeated writes. The service continues executing the
resulting thermal curve. Named groups save only their name and control IDs in
user preferences. Selecting a group or mode never writes hardware or schedules
an apply. Group presets resolve by name on each fan; manual mode uses each fan's
thermal limit and point count. A shared custom curve must meet every member's
limits and point count; incompatibility rejects the entire action before writes.
Missing or duplicate group members are also rejected. Partial write failures
restore affected fans in reverse order, including the failing fan. Undo restores
the last quick/group action only while its exact curves still match readback.

The editor supports dragging numbered chart points, numeric temperature/duty
editing, starting from current or ASUS profile curves, and a linear ramp. It
keeps the controller's actual point count and final full-speed endpoint. Drafts
are applied only on explicit click; telemetry does not overwrite in-progress
edits. Readings refresh every two seconds, including RPM when available.

## Lifetime and ownership

The first explicit apply captures the original raw curve. Restore original,
normal Quit, worker disconnection and Rust unwinding restore curves that still
match OpenCrate's last write. External controller changes take precedence during
cleanup. A subsequent explicit apply captures that external curve as the new
baseline. Retry reconnects without automatically reapplying settings.

Closing the window keeps the app active in the tray. Fan settings are not saved
for startup. A forcibly killed process cannot clean up, so the last thermal
curve continues in the ASUS service. A hung service call can delay shutdown;
COM calls never run on the GUI thread.

Tests use synthetic controls to cover validation, rollback, readback failures,
external ownership changes, quick actions and shutdown restoration. CI does not
write to physical fans or establish compatibility with untested service versions.
