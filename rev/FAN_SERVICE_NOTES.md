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
`MinimalDuty` are raw byte values; the GUI converts them to percentages. No RPM
reading is exposed by this interface.

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
service's minimum duty and critical temperature, requires nondecreasing points
and ends at full duty. Unsupported modes or point counts remain read-only.

Full Blast and other bulk actions operate on writable, discovered controls.
They preserve per-control restoration information and report individual errors.

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
