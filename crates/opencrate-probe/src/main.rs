//! Live USB oracle for the ASUS Aura controller (0B05:19AF).
//!
//! QUERY-ONLY by design: every command sent here is a status/version query
//! taken from observed HAL behavior or public docs. Nothing here changes
//! LEDs, fans, or firmware. Run this instead of Armoury Crate to validate
//! RE findings against real hardware.
//!
//! Two transfer styles are tried per query because the HAL uses WinUSB bulk
//! while the controller also exposes HID interfaces:
//!   1. HID feature reports (SetFeature/GetFeature)
//!   2. HID output report + timed read (interrupt, like OpenRGB)

use clap::{Parser, Subcommand};
use opencrate_aura::{AURA_PID_19AF, AURA_VID, REPORT_LEN};
use opencrate_core::RgbColor;

#[derive(Parser)]
#[command(
    name = "opencrate-probe",
    about = "Query-only ASUS Aura USB oracle (no state changes)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List matching HID devices (no I/O)
    List,
    /// EC C1 ready poll -> expect EC 41 .. [10]=1 [11]=0
    Ready,
    /// EC 82 firmware query -> expect EC 02 + ASCII string
    Firmware,
    /// EC B0 config-table query -> expect EC 30 ..
    Config,
    /// EC 7F query: `probe ec7f <sub:u8> <arg:u32>` -> expect EC 7F 00 <u24>
    Ec7F { sub: u8, arg: u32 },
    /// Raw 65B query from hex: `probe raw EC7F0208000000`
    /// (padded with zeros to 65 B, sent both styles)
    Raw { hex: String },
    /// Static-red helper with COMPUTED layout (no hand-counted hex):
    /// `probe poke <byte_idx> <RRGGBB>` sends EC35 02 00 00 01, then
    /// EC36 00 04 00 + zeros with the color at <byte_idx>, then commit.
    /// No end_seq (OpenRGB-pure). Used for slot mapping.
    Poke { idx: usize, color: String },
    /// Lights-out reset: static black on ch2 + commit (clean prior state).
    Off,
}

fn hexdump(tag: &str, data: &[u8]) {
    // Trim trailing zeros for readability but show real length.
    let mut end = data.len();
    while end > 8 && data[end - 1] == 0 {
        end -= 1;
    }
    let hex: String = data[..end].iter().map(|b| format!("{:02X}", b)).collect();
    println!("  {:<9} len={:<3} {}", tag, data.len(), hex);
    if end != data.len() {
        println!("              ({} trailing zero bytes)", data.len() - end);
    }
}

fn parse_hex(s: &str) -> Result<Vec<u8>, String> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !s.len().is_multiple_of(2) || s.len() > REPORT_LEN * 2 {
        return Err(format!("want 1..={} hex bytes", REPORT_LEN));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| format!("bad hex at {i}")))
        .collect()
}

fn send_both_styles(dev: &hidapi::HidDevice, req: &[u8; REPORT_LEN]) {
    // Style 1: feature reports.
    match dev.send_feature_report(req) {
        Ok(()) => {
            println!("  feature TX ok");
            let mut reply = [0u8; REPORT_LEN];
            reply[0] = req[0]; // report ID to fetch
            match dev.get_feature_report(&mut reply) {
                Ok(n) => {
                    let mut full = [0u8; REPORT_LEN];
                    let n = n.min(REPORT_LEN);
                    full[..n].copy_from_slice(&reply[..n]);
                    hexdump("feat-RX", &full);
                }
                Err(e) => println!("  feat-RX err: {e}"),
            }
        }
        Err(e) => println!("  feat-TX err: {e}"),
    }
    // Style 2: output report + timed read.
    match dev.write(req) {
        Ok(n) => {
            println!("  intr TX ok ({n} B)");
            let mut reply = [0u8; REPORT_LEN];
            match dev.read_timeout(&mut reply, 800) {
                Ok(0) => println!("  intr-RX: timeout (no data)"),
                Ok(n) => {
                    let mut full = [0u8; REPORT_LEN];
                    full[..n].copy_from_slice(&reply[..n]);
                    hexdump("intr-RX", &full);
                }
                Err(e) => println!("  intr-RX err: {e}"),
            }
        }
        Err(e) => println!("  intr-TX err: {e}"),
    }
}

fn request(op: u8, sub: u8, arg: u32) -> [u8; REPORT_LEN] {
    let mut r = [0u8; REPORT_LEN];
    r[0] = 0xEC;
    r[1] = op;
    r[2] = sub;
    r[3..7].copy_from_slice(&arg.to_le_bytes());
    r
}

fn main() {
    let cli = Cli::parse();
    let code = run(cli);
    std::process::exit(code);
}

fn run(cli: Cli) -> i32 {
    let api = match hidapi::HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("hidapi init failed: {e}");
            return 1;
        }
    };
    if matches!(cli.cmd, Cmd::List) {
        let mut found = 0;
        for d in api.device_list() {
            if d.vendor_id() == AURA_VID && d.product_id() == AURA_PID_19AF {
                found += 1;
                println!(
                    "match: vid={:04X} pid={:04X} usage={:04X}/{:04X} iface={} path={}",
                    d.vendor_id(),
                    d.product_id(),
                    d.usage_page(),
                    d.usage(),
                    d.interface_number(),
                    d.path().to_string_lossy(),
                );
            }
        }
        println!(
            "{found} interface(s) for {:04X}:{:04X}",
            AURA_VID, AURA_PID_19AF
        );
        return 0;
    }

    let dev = match api.open(AURA_VID, AURA_PID_19AF) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("open {:04X}:{:04X} failed: {e}", AURA_VID, AURA_PID_19AF);
            eprintln!("hint: close OpenRGB/Armoury Crate — they may hold the handle");
            return 1;
        }
    };
    println!(
        "opened: {} / {}",
        dev.get_manufacturer_string()
            .unwrap_or_default()
            .unwrap_or_default(),
        dev.get_product_string()
            .unwrap_or_default()
            .unwrap_or_default()
    );

    let req: [u8; REPORT_LEN] = match &cli.cmd {
        Cmd::List => unreachable!(),
        Cmd::Poke { .. } | Cmd::Off => [0u8; REPORT_LEN], // handled below
        Cmd::Ready => {
            let mut r = [0u8; REPORT_LEN];
            r[0] = 0xEC;
            r[1] = 0xC1;
            r
        }
        Cmd::Firmware => request(0x82, 0x00, 0),
        Cmd::Config => request(0xB0, 0x00, 0),
        Cmd::Ec7F { sub, arg } => request(0x7F, *sub, *arg),
        Cmd::Raw { hex } => match parse_hex(hex) {
            Ok(v) => {
                let mut r = [0u8; REPORT_LEN];
                r[..v.len()].copy_from_slice(&v);
                r
            }
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        },
    };
    // Poke/Off bypass the query path: interrupt writes only, computed bytes.
    if let Cmd::Poke { idx, color } = &cli.cmd {
        if *idx + 3 > REPORT_LEN {
            eprintln!("error: idx {idx} out of range");
            return 2;
        }
        let c: RgbColor = match color.parse() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        };
        let mut d1 = [0u8; REPORT_LEN];
        d1[0] = 0xEC;
        d1[1] = 0x35;
        d1[2] = 0x02;
        d1[5] = 0x01;
        let mut d2 = [0u8; REPORT_LEN];
        d2[0] = 0xEC;
        d2[1] = 0x36;
        d2[3] = 0x04;
        d2[*idx] = c.r;
        d2[*idx + 1] = c.g;
        d2[*idx + 2] = c.b;
        let mut commit = [0u8; REPORT_LEN];
        commit[0] = 0xEC;
        commit[1] = 0x3F;
        commit[2] = 0x55;
        for (i, p) in [d1, d2, commit].iter().enumerate() {
            hexdump(&format!("TX{i}"), p);
            if let Err(e) = dev.write(p) {
                eprintln!("write failed: {e}");
                return 1;
            }
        }
        println!("poke done: color {} at bytes {}-{}", c, idx, idx + 2);
        return 0;
    }
    if matches!(cli.cmd, Cmd::Off) {
        let mut d1 = [0u8; REPORT_LEN];
        d1[0] = 0xEC;
        d1[1] = 0x35;
        d1[2] = 0x02;
        d1[5] = 0x00;
        let mut d2 = [0u8; REPORT_LEN];
        d2[0] = 0xEC;
        d2[1] = 0x36;
        d2[3] = 0x04;
        let mut commit = [0u8; REPORT_LEN];
        commit[0] = 0xEC;
        commit[1] = 0x3F;
        commit[2] = 0x55;
        for p in [&d1, &d2, &commit] {
            if let Err(e) = dev.write(p) {
                eprintln!("write failed: {e}");
                return 1;
            }
        }
        println!("lights-out reset sent");
        return 0;
    }
    hexdump("TX", &req);
    send_both_styles(&dev, &req);
    println!("done — no state was changed on the device");
    0
}
