//! Opt-in native regression scenario; never opens hardware or real preferences.
//! Build with --features diagnostics and set OPENCRATE_DIAGNOSTIC_OUTPUT to an
//! empty scratch directory. Normal release/installer builds omit this module.

use crate::{preferences, runtime, windows_startup};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        LazyLock, Mutex,
    },
    time::{Duration, Instant},
};

static OUTPUT: LazyLock<Option<PathBuf>> =
    LazyLock::new(|| std::env::var_os("OPENCRATE_DIAGNOSTIC_OUTPUT").map(PathBuf::from));
static UPDATES: AtomicUsize = AtomicUsize::new(0);
static PAINTS: AtomicUsize = AtomicUsize::new(0);
static STEADY_UPDATES: AtomicUsize = AtomicUsize::new(0);
static STEADY_PAINTS: AtomicUsize = AtomicUsize::new(0);
static STEP: AtomicUsize = AtomicUsize::new(0);
static LAST_CAPTURE: AtomicUsize = AtomicUsize::new(usize::MAX);
static START: LazyLock<Instant> = LazyLock::new(Instant::now);
static REPORT: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());
static CAUSES: Mutex<BTreeMap<String, usize>> = Mutex::new(BTreeMap::new());
static EVENTS: Mutex<BTreeMap<&'static str, usize>> = Mutex::new(BTreeMap::new());
const INTERVAL: Duration = Duration::from_secs(5);
const SETTLE: Duration = Duration::from_millis(1500);
const PHASES: [&str; 10] = [
    "visible-idle",
    "tray-idle",
    "reopened",
    "turkish",
    "chinese",
    "fans",
    "power",
    "animation",
    "tray-animation",
    "scaled-reopen",
];

pub fn enabled() -> bool {
    OUTPUT.is_some()
}

fn start_hidden() -> bool {
    std::env::var_os("OPENCRATE_DIAGNOSTIC_START_HIDDEN").is_some()
}

pub fn run() {
    let output = OUTPUT.as_ref().unwrap();
    std::fs::create_dir_all(output).expect("diagnostic output directory");
    // Never deserialize a real user's configuration or apply saved lighting.
    let mut store = preferences::Store::load_path(output.join("preferences.json"));
    store.preferences = preferences::Preferences::default();
    store.preferences.restore_lighting = false;
    let instance = windows_startup::Instance::diagnostic().expect("isolated diagnostic instance");
    let _ = *START;
    let result = runtime::run(store, instance, start_hidden());
    let report = serde_json::json!({ "ok": result.is_ok(), "error": result.err().map(|e| e.to_string()), "phases": &*REPORT.lock().unwrap() });
    std::fs::write(
        output.join("lifecycle.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .expect("diagnostic report");
    if report["ok"] != true {
        std::process::exit(1);
    }
}

pub fn updated(ctx: &egui::Context) {
    if enabled() {
        UPDATES.fetch_add(1, Ordering::Relaxed);
        if steady() {
            STEADY_UPDATES.fetch_add(1, Ordering::Relaxed);
        }
        for cause in ctx.repaint_causes() {
            // Keep reports portable and avoid recording build-machine paths.
            let source = cause.to_string();
            let location = source.rsplit(['/', '\\']).next().unwrap_or(&source);
            *CAUSES
                .lock()
                .unwrap()
                .entry(location.to_owned())
                .or_default() += 1;
        }
    }
}

fn steady() -> bool {
    START.elapsed() >= INTERVAL * STEP.load(Ordering::Relaxed) as u32 + SETTLE
}

pub fn window_event(event: &winit::event::WindowEvent) {
    if !enabled() {
        return;
    }
    use winit::event::WindowEvent;
    let category = match event {
        WindowEvent::CursorMoved { .. }
        | WindowEvent::MouseInput { .. }
        | WindowEvent::MouseWheel { .. } => "pointer",
        WindowEvent::KeyboardInput { .. } | WindowEvent::Ime(_) => "keyboard",
        WindowEvent::Focused(_) => "focus",
        WindowEvent::RedrawRequested => "redraw",
        WindowEvent::Resized(_) => "resize",
        _ => "other",
    };
    *EVENTS.lock().unwrap().entry(category).or_default() += 1;
}

pub fn painted(buffer: &[u32], size: [u32; 2]) {
    if !enabled() {
        return;
    }
    PAINTS.fetch_add(1, Ordering::Relaxed);
    if steady() {
        STEADY_PAINTS.fetch_add(1, Ordering::Relaxed);
    }
    let step = STEP.load(Ordering::Relaxed);
    // Capture after the first frame so egui's first-pass layout has settled.
    if PAINTS.load(Ordering::Relaxed) < 2 || LAST_CAPTURE.load(Ordering::Relaxed) == step {
        return;
    }
    LAST_CAPTURE.store(step, Ordering::Relaxed);
    let rgb: Vec<u8> = buffer
        .iter()
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
        .collect();
    image::save_buffer(
        OUTPUT.as_ref().unwrap().join(format!("{step:02}.png")),
        &rgb,
        size[0],
        size[1],
        image::ColorType::Rgb8,
    )
    .expect("diagnostic screenshot");
}

pub fn deadline() -> Option<Instant> {
    let step = STEP.load(Ordering::Relaxed);
    (step < PHASES.len()).then(|| *START + INTERVAL * (step as u32 + 1))
}

pub fn advance(running: &mut runtime::Running) {
    if deadline().is_none_or(|due| Instant::now() < due) {
        return;
    }
    let step = STEP.fetch_add(1, Ordering::Relaxed);
    let report = serde_json::json!({
        "phase": if step == 0 && start_hidden() { "startup-tray" } else { PHASES[step] },
        "updates": UPDATES.swap(0, Ordering::Relaxed), "paints": PAINTS.swap(0, Ordering::Relaxed),
        "steady_updates": STEADY_UPDATES.swap(0, Ordering::Relaxed),
        "steady_paints": STEADY_PAINTS.swap(0, Ordering::Relaxed),
        "steady_seconds": START.elapsed().saturating_sub(INTERVAL * step as u32 + SETTLE).as_secs_f64(),
        "elapsed_seconds": START.elapsed().as_secs_f64(),
        "repaint_causes": std::mem::take(&mut *CAUSES.lock().unwrap()),
        "window_events": std::mem::take(&mut *EVENTS.lock().unwrap()),
    });
    REPORT.lock().unwrap().push(report);
    std::fs::write(
        OUTPUT.as_ref().unwrap().join("progress.json"),
        serde_json::to_vec_pretty(&*REPORT.lock().unwrap()).unwrap(),
    )
    .expect("diagnostic progress");
    running.diagnostic_step(step + 1);
}
