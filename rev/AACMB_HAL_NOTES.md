# ASUS USB lighting protocol

The live backend targets USB `0B05:19AF` with one fixed RGB channel and three
logical addressable channels. Logical channels do not establish physical header
availability. The implementation uses 65-byte, zero-padded HID interrupt reports.

## Runtime effects and color

- `EC 52 53 00 01` initializes Gen1-compatible ARGB operation.
- `EC 35 <channel> 00 00 <mode>` selects an effect on a zero-based channel.
- `EC 36 <mask_hi> <mask_lo> 00 ...RGB` sets color. The LED mask is big-endian;
  RGB starts at byte `5 + 3 * start_led`.
- Byte 4 selects runtime/shutdown settings. Runtime writes keep it zero and do
  not issue NVRAM commits.

| Logical channel | Effect index | Color mask | RGB byte indices |
| --- | --- | --- | --- |
| led1 | 0 | 1 | 5, 6, 7 |
| led2 | 1 | 2 | 8, 9, 10 |
| led3 | 2 | 4 | 11, 12, 13 |
| led4 | 3 | 8 | 14, 15, 16 |

Effect indices, direct channel IDs and color masks are distinct fields.
Color data must not overlap the command header or selection mask. Unit tests
cover primary and mixed colors, mask integrity and synchronized commands.

## Direct frames and queries

Direct mode uses `EC 35 <channel> 00 00 FF`, followed by
`EC 40 <channel|apply> <offset> <count> ...RGB`. Each report contains up to 20
LED colors; the final chunk for a channel sets the `0x80` apply bit. Direct
channel 4 is fixed RGB; channels 0, 1 and 2 are addressable. The configured
frame capacity is 120 colors, not a discovered physical LED count.

`EC 82` requests firmware text (`EC 02` reply), `EC B0` requests configuration
(`EC 30` reply), and `EC C1` polls readiness (`EC 41` reply). Diagnostic replies
belong to the device being queried and are not bundled as public fixtures.

## Reference

Packet layout was cross-checked against
[OpenRGB's Aura USB controller implementation](https://github.com/CalcProgrammer1/OpenRGB/tree/fd1bc449ae50eb549a0472efaf72cb33ccef28ab/Controllers/AsusAuraUSBController/AsusAuraUSBController),
particularly `SetMode`, `SendEffect`, `SendColor`, `SetGen1` and `SendDirect`.
See [animation notes](AURA_SPEED_NOTES.md) for software speed and brightness.
