//! ASUS Aura USB effect/direct-mode driver for the supported 0B05:19AF layout.
//!
//! Protocol layout cross-checked against OpenRGB's AuraMainboardController
//! with a supported layout of one fixed LED followed by three logical
//! addressable channels. Effect channel indices, LED masks, and RGB payload
//! offsets are distinct fields. See `rev/AACMB_HAL_NOTES.md`.
//! The controller uses HID interrupt reports, not feature reports.
//!
//! Packet builders are pure logic (unit-tested); `apply_effect_live`
//! performs the actual HID writes.

use opencrate_core::{Channel, EffectMode, RgbColor};

pub mod animation;
#[cfg(feature = "hid")]
pub mod playback;

/// USB vendor/product IDs of the Aura motherboard controllers.
pub const AURA_VID: u16 = 0x0B05;
/// Product ID supported by the live motherboard lighting backend.
pub const AURA_PID_19AF: u16 = 0x19AF;
pub const AURA_PID_1939: u16 = 0x1939;
pub const AURA_PID_18F3: u16 = 0x18F3;

/// Every HID report is exactly this long, zero-padded.
pub const REPORT_LEN: usize = 65;
/// Frame capacity used by the supported direct-mode backend.
/// This is not a count of the user's physically connected LEDs.
pub const DIRECT_LED_COUNT: usize = 120;

/// Effect-mode channel descriptor.
///
/// `effect_channel`: zero-based channel index for `EC 35` (not a bus ID).
/// `color_mask`: LED selection mask in bytes 2..4 of `EC 36`.
/// `color_start`: RGB byte index after the five-byte `EC 36` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChannelDesc {
    effect_channel: u8,
    color_mask: u16,
    color_start: usize,
}

fn describe(channel: Channel) -> Option<ChannelDesc> {
    match channel {
        Channel::Led1 => Some(ChannelDesc {
            effect_channel: 0,
            color_mask: 0x01,
            color_start: 5,
        }),
        Channel::Led2 => Some(ChannelDesc {
            effect_channel: 1,
            color_mask: 0x02,
            color_start: 8,
        }),
        Channel::Led3 => Some(ChannelDesc {
            effect_channel: 2,
            color_mask: 0x04,
            color_start: 11,
        }),
        Channel::Led4 => Some(ChannelDesc {
            effect_channel: 3,
            color_mask: 0x08,
            color_start: 14,
        }),
        Channel::Sync => None,
    }
}

fn blank_report() -> [u8; REPORT_LEN] {
    [0u8; REPORT_LEN]
}

/// `EC 82` — ask the controller for its firmware version string (`EC 02` reply).
pub fn firmware_version_request() -> [u8; REPORT_LEN] {
    let mut r = blank_report();
    r[0] = 0xEC;
    r[1] = 0x82;
    r
}

/// `EC B0` — ask the controller for its config table (`EC 30` reply).
pub fn config_table_request() -> [u8; REPORT_LEN] {
    let mut r = blank_report();
    r[0] = 0xEC;
    r[1] = 0xB0;
    r
}

/// `EC C1` — ready poll, per HAL behavior; reply `EC 41 ..`, status `[10]=1,[11]=0`.
pub fn ready_poll_request() -> [u8; REPORT_LEN] {
    let mut r = blank_report();
    r[0] = 0xEC;
    r[1] = 0xC1;
    r
}

/// Parse an `EC 02` firmware reply into its ASCII version string.
pub fn parse_firmware_version(reply: &[u8]) -> Option<String> {
    if reply.len() < 2 || reply[0] != 0xEC || reply[1] != 0x02 {
        return None;
    }
    let bytes: Vec<u8> = reply[2..].iter().cloned().take_while(|&b| b != 0).collect();
    String::from_utf8(bytes).ok().filter(|s| !s.is_empty())
}

/// Build the effect-mode command sequence for one channel.
///
/// Initialize effect-compatible ARGB operation (`EC 52 53 00 01`), then
/// send effect-select (`EC 35 <channel> 00 00 <mode>`) and color
/// (`EC 36 <mask_hi> <mask_lo> 00 … RGB`) for each selected channel.
/// Runtime changes do not need an NVRAM commit. Byte 4 selects shutdown
/// settings, so it stays zero: the former "close-out" changed shutdown RGB.
/// Returns the ordered reports to write via HID interrupt OUT.
pub fn build_effect_packets(
    channel: Channel,
    mode: EffectMode,
    color: RgbColor,
) -> Result<Vec<[u8; REPORT_LEN]>, AuraError> {
    let channels: Vec<Channel> = match channel {
        Channel::Sync => vec![Channel::Led1, Channel::Led2, Channel::Led3, Channel::Led4],
        c => vec![c],
    };

    let mut init = blank_report();
    init[..5].copy_from_slice(&[0xEC, 0x52, 0x53, 0x00, 0x01]);
    let mut out = vec![init];
    for ch in channels {
        let desc = describe(ch).expect("Sync expanded above");
        let single = if mode.takes_color() {
            color
        } else {
            RgbColor::BLACK
        };

        // EC 35 <effect channel> 00 00 <mode>
        let mut sel = blank_report();
        sel[0] = 0xEC;
        sel[1] = 0x35;
        sel[2] = desc.effect_channel;
        sel[5] = mode.code();
        out.push(sel);

        // EC 36 <16-bit LED mask, big endian> 00 … RGB
        let mut col = blank_report();
        col[0] = 0xEC;
        col[1] = 0x36;
        col[2..4].copy_from_slice(&desc.color_mask.to_be_bytes());
        col[desc.color_start] = single.r;
        col[desc.color_start + 1] = single.g;
        col[desc.color_start + 2] = single.b;
        out.push(col);
    }

    Ok(out)
}

/// Select direct per-LED operation on the board's four logical channels.
pub fn build_direct_init_packets() -> Vec<[u8; REPORT_LEN]> {
    let mut init = blank_report();
    init[..5].copy_from_slice(&[0xEC, 0x52, 0x53, 0, 1]);
    let mut packets = vec![init];
    for channel in 0..4 {
        let mut packet = blank_report();
        packet[..6].copy_from_slice(&[0xEC, 0x35, channel, 0, 0, 0xFF]);
        packets.push(packet);
    }
    packets
}

/// One synchronized direct frame. The fixed RGB header uses the first color;
/// each ARGB header gets the same per-LED colors (up to DIRECT_LED_COUNT).
/// Direct channel IDs differ from effect channel indices: fixed = 4,
/// ARGB = 0, 1, 2. The final chunk for each channel carries the apply bit.
pub fn build_direct_frame_packets(colors: &[RgbColor]) -> Result<Vec<[u8; REPORT_LEN]>, AuraError> {
    if colors.is_empty() || colors.len() > DIRECT_LED_COUNT {
        return Err(AuraError::InvalidFrame(
            "direct frame requires 1..=120 LED colors".into(),
        ));
    }
    let mut packets = Vec::new();
    for channel in [4, 0, 1, 2] {
        let channel_colors = if channel == 4 { &colors[..1] } else { colors };
        for (chunk_index, chunk) in channel_colors.chunks(20).enumerate() {
            let offset = chunk_index * 20;
            let last = offset + chunk.len() == channel_colors.len();
            let mut packet = blank_report();
            packet[..5].copy_from_slice(&[
                0xEC,
                0x40,
                channel | if last { 0x80 } else { 0 },
                offset as u8,
                chunk.len() as u8,
            ]);
            for (index, color) in chunk.iter().enumerate() {
                let start = 5 + index * 3;
                packet[start..start + 3].copy_from_slice(&[color.r, color.g, color.b]);
            }
            packets.push(packet);
        }
    }
    Ok(packets)
}

/// Write an effect sequence to the matching Aura lighting interface via HID
/// interrupt OUT (the transfer style the firmware answers; feature
/// reports are rejected by 19AF). Fire-and-forget: effect commands
/// produce no reply.
#[cfg(feature = "hid")]
pub fn apply_effect_live(
    channel: Channel,
    mode: EffectMode,
    color: RgbColor,
) -> Result<usize, AuraError> {
    AuraDevice::open()?.apply_effect(channel, mode, color)
}

/// A reusable handle to the lighting interface, kept open during animation.
#[cfg(feature = "hid")]
pub struct AuraDevice(hidapi::HidDevice);

#[cfg(feature = "hid")]
impl AuraDevice {
    pub fn open() -> Result<Self, AuraError> {
        let api = hidapi::HidApi::new().map_err(|e| AuraError::Transport(e.to_string()))?;
        let info = api
            .device_list()
            .find(|d| {
                d.vendor_id() == AURA_VID
                    && d.product_id() == AURA_PID_19AF
                    && d.usage_page() == 0xFF72
                    && d.usage() == 0xA1
            })
            .ok_or_else(|| {
                AuraError::Transport("Aura 0B05:19AF lighting interface not found".into())
            })?;
        info.open_device(&api)
            .map(Self)
            .map_err(|e| AuraError::Transport(e.to_string()))
    }

    fn write_packets(&self, packets: &[[u8; REPORT_LEN]]) -> Result<usize, AuraError> {
        for packet in packets {
            let written = self
                .0
                .write(packet)
                .map_err(|e| AuraError::Transport(e.to_string()))?;
            if written != REPORT_LEN {
                return Err(AuraError::Transport(format!(
                    "short HID write: {written}/{REPORT_LEN}"
                )));
            }
        }
        Ok(packets.len())
    }

    pub fn apply_effect(
        &self,
        channel: Channel,
        mode: EffectMode,
        color: RgbColor,
    ) -> Result<usize, AuraError> {
        self.write_packets(&build_effect_packets(channel, mode, color)?)
    }

    pub fn start_direct(&self) -> Result<(), AuraError> {
        self.write_packets(&build_direct_init_packets()).map(|_| ())
    }

    pub fn write_frame(&self, colors: &[RgbColor]) -> Result<(), AuraError> {
        self.write_packets(&build_direct_frame_packets(colors)?)
            .map(|_| ())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AuraError {
    UnsupportedChannel(String),
    InvalidFrame(String),
    Transport(String),
}

impl std::fmt::Display for AuraError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedChannel(s) => write!(f, "unsupported channel: {s}"),
            Self::InvalidFrame(s) => write!(f, "invalid frame: {s}"),
            Self::Transport(s) => write!(f, "transport error: {s}"),
        }
    }
}

impl std::error::Error for AuraError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(p: &[u8; REPORT_LEN], n: usize) -> Vec<u8> {
        p[..n].to_vec()
    }

    #[test]
    fn report_lengths_and_opcodes() {
        let fw = firmware_version_request();
        assert_eq!(fw.len(), REPORT_LEN);
        assert_eq!(&fw[..2], &[0xEC, 0x82]);

        let cfg = config_table_request();
        assert_eq!(&cfg[..2], &[0xEC, 0xB0]);

        let rd = ready_poll_request();
        assert_eq!(&rd[..2], &[0xEC, 0xC1]);
    }

    /// Independent primary and mixed colors catch misplaced RGB components;
    /// the previous red-only test blessed a wrong channel and payload offset.
    #[test]
    fn led3_static_color_vectors() {
        for rgb in [[255, 0, 0], [0, 255, 0], [0, 0, 255], [18, 52, 86]] {
            let pkts = build_effect_packets(
                Channel::Led3,
                EffectMode::Static,
                RgbColor::new(rgb[0], rgb[1], rgb[2]),
            )
            .unwrap();
            assert_eq!(pkts.len(), 3);
            assert_eq!(head(&pkts[0], 5), vec![0xEC, 0x52, 0x53, 0x00, 0x01]);
            assert_eq!(head(&pkts[1], 6), vec![0xEC, 0x35, 0x02, 0x00, 0x00, 0x01]);
            let mut expected = [0u8; REPORT_LEN];
            expected[..5].copy_from_slice(&[0xEC, 0x36, 0x00, 0x04, 0x00]);
            expected[11..14].copy_from_slice(&rgb);
            assert_eq!(pkts[2], expected);
        }
    }

    #[test]
    fn channel_headers_and_rgb_payloads_do_not_overlap() {
        for (channel, index, mask, offset) in [
            (Channel::Led1, 0, 1, 5),
            (Channel::Led2, 1, 2, 8),
            (Channel::Led3, 2, 4, 11),
            (Channel::Led4, 3, 8, 14),
        ] {
            let pkts = build_effect_packets(channel, EffectMode::Static, RgbColor::new(18, 52, 86))
                .unwrap();
            assert_eq!(head(&pkts[1], 6), vec![0xEC, 0x35, index, 0, 0, 1]);
            assert_eq!(head(&pkts[2], 5), vec![0xEC, 0x36, 0, mask, 0]);
            assert_eq!(&pkts[2][offset..offset + 3], &[18, 52, 86]);
            assert!(pkts[2][5..offset].iter().all(|&b| b == 0));
            assert!(pkts[2][offset + 3..].iter().all(|&b| b == 0));
        }
    }

    #[test]
    fn sync_expands_to_four_channels() {
        let pkts =
            build_effect_packets(Channel::Sync, EffectMode::Rainbow, RgbColor::BLACK).unwrap();
        assert_eq!(pkts.len(), 1 + 4 * 2);
        // Rainbow takes no color: all color slots stay zero.
        for (index, chunk) in pkts[1..].as_chunks::<2>().0.iter().enumerate() {
            assert_eq!(chunk[0][2], index as u8);
            assert_eq!(chunk[1][3], 1 << index);
            assert!(chunk[1][5..].iter().all(|&b| b == 0));
            // Runtime apply must not change shutdown settings or persist to flash.
            assert_eq!(chunk[0][4], 0);
            assert_eq!(chunk[1][4], 0);
            assert_eq!(chunk[0][1], 0x35);
            assert_eq!(chunk[1][1], 0x36);
        }
    }

    #[test]
    fn firmware_parse() {
        let mut reply = vec![0u8; 32];
        reply[0] = 0xEC;
        reply[1] = 0x02;
        reply[2..2 + 15].copy_from_slice(b"TEST-FIRMWARE01");
        assert_eq!(
            parse_firmware_version(&reply).as_deref(),
            Some("TEST-FIRMWARE01")
        );
        assert_eq!(parse_firmware_version(&[0x00, 0x01]), None);
    }

    #[test]
    fn direct_chunks_preserve_colors_and_apply_only_at_channel_end() {
        for count in [1, 20, 21, DIRECT_LED_COUNT] {
            let colors: Vec<_> = (0..count)
                .map(|i| RgbColor::new(i as u8, 255 - i as u8, 42))
                .collect();
            let packets = build_direct_frame_packets(&colors).unwrap();
            assert_eq!(&packets[0][..8], &[0xEC, 0x40, 0x84, 0, 1, 0, 255, 42]);
            assert!(packets[0][8..].iter().all(|&b| b == 0));
            for channel in 0..3 {
                let group: Vec<_> = packets.iter().filter(|p| p[2] & 0x7F == channel).collect();
                assert_eq!(group.len(), count.div_ceil(20));
                let mut reconstructed = Vec::new();
                for (index, packet) in group.iter().enumerate() {
                    assert_eq!(&packet[..2], &[0xEC, 0x40]);
                    assert_eq!(packet[3] as usize, reconstructed.len());
                    assert_eq!(packet[2] & 0x80 != 0, index == group.len() - 1);
                    let end = 5 + packet[4] as usize * 3;
                    for rgb in packet[5..end].as_chunks::<3>().0 {
                        reconstructed.push(RgbColor::new(rgb[0], rgb[1], rgb[2]));
                    }
                    assert!(packet[end..].iter().all(|&b| b == 0));
                }
                assert_eq!(reconstructed, colors);
            }
        }
        assert!(build_direct_frame_packets(&[]).is_err());
        assert!(build_direct_frame_packets(&[RgbColor::BLACK; DIRECT_LED_COUNT + 1]).is_err());
    }

    #[test]
    fn direct_init_uses_runtime_effect_indices_without_flash_commits() {
        let packets = build_direct_init_packets();
        assert_eq!(packets.len(), 5);
        assert_eq!(&packets[0][..5], &[0xEC, 0x52, 0x53, 0, 1]);
        for (channel, packet) in packets[1..].iter().enumerate() {
            assert_eq!(&packet[..6], &[0xEC, 0x35, channel as u8, 0, 0, 0xFF]);
            assert!(packet[6..].iter().all(|&b| b == 0));
        }
    }
}
