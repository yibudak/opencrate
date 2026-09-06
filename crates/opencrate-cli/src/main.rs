//! OpenCrate diagnostic CLI. Lighting commands print reports by default;
//! `aura set --live` explicitly enables hardware writes. Fan commands use mocks.

use clap::{Parser, Subcommand};
use opencrate_aura::{build_effect_packets, AURA_PID_19AF, AURA_VID};
use opencrate_core::{Channel, EffectMode, RgbColor};
use opencrate_fan::{FanController, FanHeader, MockFanController, PwmPreset};

#[derive(Parser)]
#[command(
    name = "opencrate",
    about = "Open Armoury Crate replacement (early v0.1, dry-run)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Aura RGB lighting (ASUS 0B05:19AF effect mode)
    Aura {
        #[command(subcommand)]
        cmd: AuraCmd,
    },
    /// Fan PWM presets (dry-run via mock controller)
    Fan {
        #[command(subcommand)]
        cmd: FanCmd,
    },
    /// Show supported lighting backend and CLI capabilities (no device discovery)
    Info,
}

#[derive(Subcommand)]
enum AuraCmd {
    /// List all known effect modes
    ListModes,
    /// Build (and print) the HID reports for a lighting command
    Set {
        /// led1 | led2 | led3 | led4 | sync
        #[arg(long, default_value = "sync")]
        channel: String,
        /// e.g. static, breathing, rainbow, spectrum_cycle
        #[arg(long, default_value = "static")]
        mode: String,
        /// RRGGBB, only for modes that take a color
        #[arg(long, default_value = "FF0000")]
        color: String,
        /// Actually write to 0B05:19AF over HID (default: print only)
        #[arg(long)]
        live: bool,
    },
}

#[derive(Subcommand)]
enum FanCmd {
    /// List synthetic example headers and presets (no hardware discovery)
    List,
    /// Apply a preset (dry-run printout)
    Apply {
        #[arg(long)]
        header: u8,
        /// auto | off | silent | medium | full
        #[arg(long, default_value = "silent")]
        preset: String,
    },
}

fn example_headers() -> Vec<FanHeader> {
    vec![
        FanHeader::new(0, "CPU_FAN"),
        FanHeader::new(1, "CHA_FAN1"),
        FanHeader::new(2, "CHA_FAN2"),
    ]
}

fn parse_preset(s: &str) -> Result<PwmPreset, String> {
    match s.to_ascii_lowercase().as_str() {
        "auto" => Ok(PwmPreset::Auto),
        "off" => Ok(PwmPreset::Off),
        "silent" => Ok(PwmPreset::Silent),
        "medium" => Ok(PwmPreset::Medium),
        "full" => Ok(PwmPreset::Full),
        _ => Err(format!(
            "unknown preset: {s} (want auto|off|silent|medium|full)"
        )),
    }
}

fn main() {
    let cli = Cli::parse();
    let code = run(cli);
    std::process::exit(code);
}

fn run(cli: Cli) -> i32 {
    match cli.cmd {
        Cmd::Info => {
            println!("opencrate v0.1 — dry-run build");
            println!(
                "aura         : {:04X}:{:04X} (AURA LED Controller)",
                AURA_VID, AURA_PID_19AF
            );
            println!("fans         : synthetic CLI example; use the GUI for ASUS service controls");
            println!("hint         : run `opencrate aura list-modes` to start");
            0
        }
        Cmd::Aura { cmd } => match cmd {
            AuraCmd::ListModes => {
                for m in EffectMode::all() {
                    println!(
                        "{:<24} code=0x{:02X} takes_color={}",
                        m.name(),
                        m.code(),
                        m.takes_color()
                    );
                }
                0
            }
            AuraCmd::Set {
                channel,
                mode,
                color,
                live,
            } => {
                let channel: Channel = match channel.parse() {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 2;
                    }
                };
                let mode: EffectMode = match mode.parse() {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 2;
                    }
                };
                let color: RgbColor = match color.parse() {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 2;
                    }
                };
                if !mode.takes_color() && color != RgbColor::BLACK {
                    println!("note: mode `{}` ignores color; using default", mode.name());
                }
                match build_effect_packets(channel, mode, color) {
                    Ok(pkts) => {
                        let tag = if live { "LIVE" } else { "DRY-RUN" };
                        println!(
                            "{tag}: {} report(s) for {:?} {} {} (VID {:04X} PID {:04X})",
                            pkts.len(),
                            channel,
                            mode.name(),
                            color,
                            AURA_VID,
                            AURA_PID_19AF
                        );
                        for (i, p) in pkts.iter().enumerate() {
                            let hex: String = p.iter().map(|b| format!("{:02X}", b)).collect();
                            // Trim trailing zeros for readability, keep opcode head visible.
                            println!("  [{i:02}] {}", hex.trim_end_matches("00"));
                        }
                        if live {
                            match opencrate_aura::apply_effect_live(channel, mode, color) {
                                Ok(n) => {
                                    println!("wrote {n} report(s) to hardware");
                                    0
                                }
                                Err(e) => {
                                    eprintln!("hardware write failed: {e}");
                                    1
                                }
                            }
                        } else {
                            println!("no hardware writes performed (pass --live to write)");
                            0
                        }
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        1
                    }
                }
            }
        },
        Cmd::Fan { cmd } => match cmd {
            FanCmd::List => {
                for h in example_headers() {
                    println!("header {} {}", h.id, h.name);
                }
                println!("presets: auto off silent(30%) medium(60%) full(100%) + 8-point curve (see FanCurve)");
                0
            }
            FanCmd::Apply { header, preset } => {
                let preset = match parse_preset(&preset) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 2;
                    }
                };
                let mut mock = MockFanController::new(example_headers());
                let res = match preset.fixed_duty() {
                    None => mock.set_auto(header),
                    Some(d) => mock.set_pwm_pct(header, d),
                };
                match res {
                    Ok(()) => {
                        println!(
                            "DRY-RUN: header {header} -> {} (no hardware writes performed)",
                            preset.name()
                        );
                        0
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        1
                    }
                }
            }
        },
    }
}
