# Adjustable RGB animation speed

The lighting backend implements adjustable speed and brightness for its
supported USB controller layout.

## Protocol evidence

OpenRGB reference checkout: `rev/upstream/openrgb`, commit
`fd1bc449ae50eb549a0472efaf72cb33ccef28ab`.

- `Controllers/AsusAuraUSBController/AsusAuraUSBController/RGBController_AsusAuraUSB.cpp`
  exposes no speed parameter for native motherboard effects.
- `AsusAuraMainboardController.cpp` distinguishes effect indices from direct IDs.
- `AsusAuraUSBController.cpp`, `SendDirect`, documents the wire format through
  its implementation: `EC 40 channel|apply offset count RGB...`, 20 LEDs per
  65-byte report, final-chunk apply bit `80`.

Upstream source:
[OpenRGB Aura USB controller](https://github.com/CalcProgrammer1/OpenRGB/tree/fd1bc449ae50eb549a0472efaf72cb33ccef28ab/Controllers/AsusAuraUSBController/AsusAuraUSBController).

The native effect command used here has no verified speed field. Speed is
implemented by independently generated direct-mode frames, not an invented
firmware parameter. Software patterns are interpretations of the effect names,
not reproductions of the firmware's exact animations. Native CLI effect
commands remain available.

## Behavior

- GUI speed: 0.25xâ€“4x; 1x resets the multiplier. Static and Off disable speed.
- Apply starts an effect. Speed changes then update the running effect without
  restarting its phase. Pending color edits still require Apply.
- One worker owns the HID handle and sends frames at approximately 30 FPS,
  subject to USB transfer time. Timing uses elapsed time, not frame count.
- All logical headers receive the same colors. The fixed RGB header uses the
  first LED's color; three ARGB channels receive 120 colors each. This configured
  frame size does not establish a physical LED count. Spatial effects repeat
  every 12 LEDs so no physical fan/strip topology is assumed in the UI.
- Sliding updates are coalesced; the UI does not wait for HID writes.
- Closing the window hides it to the tray and playback continues. Tray events
  explicitly wake the GUI. Quit restores the selected native effect at the
  firmware's own speed. Abrupt process termination cannot perform that restore.
- Transport errors stop streaming and attempt a native fallback once; Apply
  retries the connection. Successful writes validate the full report length.
- Startup only restores lighting when enabled and a valid saved configuration exists.

## Automated coverage

Packet tests cover chunk boundaries, RGB reconstruction, offsets, final apply
bits, padding and invalid frame sizes. Animation tests cover elapsed-time scaling,
phase continuity, color preservation and temporal movement. Worker tests cover
speed updates without reinitialization, Static transitions and Quit fallback.
These tests do not establish physical device compatibility or LED topology.

## Brightness

The GUI now has a global 0â€“100% Brightness slider and a 100% reset button.
After Apply, brightness changes update the running setting immediately, just
like speed. Off disables the slider. The chosen RGB color remains unmodified;
each output component is scaled with integer rounding. Static uses the native
color command with scaled RGB, while animated modes scale every rendered LED.
Moving through 0% does not replace the selected effect or reset animation time.

Quit/failure fallback preserves 0% as Off. Native color-taking effects can use
scaled RGB. Dimmed multicolor effects have no verified native brightness field,
so those fall back to the current first LED's dimmed static color, avoiding an
unexpected jump to full brightness. At 100%, native fallback is unchanged.

Tests cover component scaling, zero brightness, recovery from zero, Static
output and native fallback without double dimming.
