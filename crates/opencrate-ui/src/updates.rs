//! GitHub release checks and verified, browser-free installer downloads.
//! Network and file work runs off the UI thread. Setup starts after App is dropped.

use reqwest::{blocking::Client, StatusCode};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

const API: &str = "https://api.github.com/repos/yibudak/opencrate/releases/latest";
const DOWNLOAD_ROOT: &str = "https://github.com/yibudak/opencrate/releases/download/";
const MAX_INSTALLER: u64 = 512 * 1024 * 1024;
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

pub type PendingInstall = Option<(Installer, crate::i18n::Language)>;

#[derive(Clone, Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    state: String,
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    body: Option<String>,
    assets: Vec<Asset>,
}

#[derive(Clone, Debug)]
pub struct Update {
    pub version: Version,
    pub notes: String,
    setup: Asset,
    checksum: Asset,
}

fn select_release(release: Release, current: &Version) -> Result<Option<Update>, String> {
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let tag = &release.tag_name;
    let version = Version::parse(tag.strip_prefix('v').unwrap_or(tag))
        .map_err(|_| "GitHub returned an invalid release version.")?;
    if !version.pre.is_empty() || version.cmp_precedence(current).is_le() {
        return Ok(None);
    }
    // Match the publisher's numeric tags and exact x64 installer/checksum pair.
    if tag != &format!("v{version}") || !version.build.is_empty() {
        return Err("Unsupported release tag format.".into());
    }
    let name = format!("OpenCrate-{version}-windows-x64-setup.exe");
    let find = |name: &str, maximum: u64| -> Result<Asset, String> {
        let matches: Vec<_> = release.assets.iter().filter(|a| a.name == name).collect();
        let [asset] = matches.as_slice() else {
            return Err(format!("Release must contain exactly one {name}."));
        };
        if asset.state != "uploaded" || asset.size == 0 || asset.size > maximum {
            return Err(format!("Invalid or incomplete release asset: {name}"));
        }
        if asset.browser_download_url != format!("{DOWNLOAD_ROOT}{tag}/{name}") {
            return Err("Release download URL does not belong to OpenCrate.".into());
        }
        Ok((*asset).clone())
    };
    Ok(Some(Update {
        version,
        setup: find(&name, MAX_INSTALLER)?,
        checksum: find(&format!("{name}.sha256"), 4096)?,
        notes: release.body.unwrap_or_default(),
    }))
}

fn client(timeout: Duration) -> Result<Client, String> {
    Client::builder()
        .user_agent(concat!("OpenCrate/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            if attempt.previous().len() >= 5 {
                return attempt.error("Too many download redirects");
            }
            // GitHub redirects release assets to its signed object-storage URLs.
            if url.scheme() == "https"
                && matches!(
                    url.host_str(),
                    Some(
                        "github.com"
                            | "api.github.com"
                            | "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                    )
                )
                && url.port_or_known_default() == Some(443)
                && url.username().is_empty()
                && url.password().is_none()
            {
                attempt.follow()
            } else {
                attempt.error("Untrusted release redirect")
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}

fn read_limited(mut reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Release response exceeded its size limit.".into());
    }
    Ok(bytes)
}

fn check(current: &Version) -> Result<Option<Update>, String> {
    let response = client(Duration::from_secs(30))?
        .get(API)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .map_err(|e| e.to_string())?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if matches!(
        response.status(),
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
    ) {
        return Err("GitHub is limiting requests. Try again later.".into());
    }
    let response = response.error_for_status().map_err(|e| e.to_string())?;
    let release = serde_json::from_slice(&read_limited(response, 2 * 1024 * 1024)?)
        .map_err(|e| format!("Invalid GitHub release response: {e}"))?;
    select_release(release, current)
}

fn parse_checksum(bytes: &[u8], filename: &str) -> Result<[u8; 32], String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "Invalid checksum encoding.")?;
    let fields: Vec<_> = text
        .trim_start_matches('\u{feff}')
        .split_whitespace()
        .collect();
    if fields.len() != 2
        || fields[1] != filename
        || fields[0].len() != 64
        || !fields[0].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Invalid release checksum or filename.".into());
    }
    let mut digest = [0; 32];
    for (i, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&fields[0][i * 2..i * 2 + 2], 16).map_err(|e| e.to_string())?;
    }
    Ok(digest)
}

/// The open read handle denies writes/deletion on Windows until Setup is launched.
pub struct Installer {
    verified_file: File,
    path: PathBuf,
    directory: tempfile::TempDir,
}

impl Installer {
    /// Called only after the native runtime has dropped App, its workers and its instance mutex.
    pub fn launch(self, language: crate::i18n::Language) -> Result<(), String> {
        let Self {
            verified_file,
            path,
            directory,
        } = self;
        let language = match language {
            crate::i18n::Language::English => "/LANG=en",
            crate::i18n::Language::Chinese => "/LANG=zh_CN",
            crate::i18n::Language::Turkish => "/LANG=tr",
        };
        // No shell, elevation, silent install, or forced reboot. Inno preserves the
        // previous install directory and offers to reopen OpenCrate on completion.
        let result = std::process::Command::new(&path)
            .args(["/SP-", "/NORESTART", language])
            .current_dir(directory.path())
            .spawn()
            .map_err(|e| format!("Could not start the installer: {e}"));
        drop(verified_file);
        result?;
        // Setup needs its source after this process exits. Old downloads are
        // removed on a subsequent download, after a seven-day grace period.
        let _ = directory.keep();
        Ok(())
    }
}

fn download_directory() -> Result<tempfile::TempDir, String> {
    let base =
        std::env::var_os("LOCALAPPDATA").ok_or("Cannot locate the update download folder.")?;
    let root = PathBuf::from(base).join("opencrate").join("updates");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    if let Ok(entries) = fs::read_dir(&root) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("download-")
                && entry
                    .file_type()
                    .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
                && entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|time| time.elapsed().ok())
                    .is_some_and(|age| age > Duration::from_secs(7 * 86400))
            {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
    tempfile::Builder::new()
        .prefix("download-")
        .tempdir_in(root)
        .map_err(|e| e.to_string())
}

fn verify_stream(
    mut source: impl Read,
    mut destination: impl Write,
    size: u64,
    expected: [u8; 32],
    cancelled: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<(), String> {
    let mut hasher = Sha256::new();
    let mut received = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Err("Download cancelled.".into());
        }
        let count = source.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if received > size || received > MAX_INSTALLER {
            return Err("Installer exceeded the expected size.".into());
        }
        destination
            .write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
        hasher.update(&buffer[..count]);
        progress(received);
    }
    if received != size || size == 0 {
        return Err("Installer download is incomplete.".into());
    }
    if hasher.finalize().as_slice() != expected {
        return Err("Installer checksum mismatch. Download the update again.".into());
    }
    Ok(())
}

fn download(
    update: &Update,
    cancelled: &AtomicBool,
    progress: impl FnMut(u64),
) -> Result<Installer, String> {
    let client = client(Duration::from_secs(300))?;
    let checksum = client
        .get(&update.checksum.browser_download_url)
        .timeout(Duration::from_secs(30))
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let digest = parse_checksum(&read_limited(checksum, 4096)?, &update.setup.name)?;
    if cancelled.load(Ordering::Relaxed) {
        return Err("Download cancelled.".into());
    }
    let directory = download_directory()?;
    let partial = directory.path().join("setup.part");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|e| e.to_string())?;
    let response = client
        .get(&update.setup.browser_download_url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    verify_stream(
        response,
        &mut file,
        update.setup.size,
        digest,
        cancelled,
        progress,
    )?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    let path = directory.path().join(&update.setup.name);
    fs::rename(&partial, &path).map_err(|e| e.to_string())?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1); // FILE_SHARE_READ: no writes or replacement while ready.
    }
    let mut verified_file = options.open(&path).map_err(|e| e.to_string())?;
    // Verify the locked on-disk file as well as the bytes received over HTTPS.
    verify_stream(
        &mut verified_file,
        std::io::sink(),
        update.setup.size,
        digest,
        cancelled,
        |_| {},
    )?;
    Ok(Installer {
        verified_file,
        path,
        directory,
    })
}

#[derive(PartialEq, Eq)]
pub enum Status {
    Idle,
    Checking,
    Current,
    Available,
    Downloading,
    Ready,
    Failed,
}

enum Event {
    Checked(Result<Option<Update>, String>),
    Progress(u64),
    Downloaded(Result<Installer, String>),
}

pub struct State {
    pub status: Status,
    pub release: Option<Update>,
    pub error: Option<String>,
    pub received: u64,
    pub cancelling: bool,
    ready: Option<Installer>,
    events: Option<mpsc::Receiver<Event>>,
    cancelled: Arc<AtomicBool>,
    next_check: Instant,
}

impl Default for State {
    fn default() -> Self {
        Self {
            status: Status::Idle,
            release: None,
            error: None,
            received: 0,
            cancelling: false,
            ready: None,
            events: None,
            cancelled: Arc::new(AtomicBool::new(false)),
            next_check: Instant::now(),
        }
    }
}

impl State {
    pub fn busy(&self) -> bool {
        matches!(self.status, Status::Checking | Status::Downloading)
    }
    pub fn available(&self) -> bool {
        self.release.is_some()
    }

    pub fn check(&mut self, ctx: &egui::Context) {
        if self.busy() || self.ready.is_some() {
            return;
        }
        self.status = Status::Checking;
        self.error = None;
        self.release = None;
        self.next_check = Instant::now() + CHECK_INTERVAL;
        self.start(ctx, |sender, ctx, _| {
            let result =
                check(&Version::parse(env!("CARGO_PKG_VERSION")).expect("package version"));
            let _ = sender.send(Event::Checked(result));
            ctx.request_repaint();
        });
    }

    pub fn download(&mut self, ctx: &egui::Context) {
        if self.busy() || self.ready.is_some() {
            return;
        }
        let Some(update) = self.release.clone() else {
            return;
        };
        self.status = Status::Downloading;
        self.received = 0;
        self.error = None;
        self.cancelling = false;
        self.start(ctx, move |sender, ctx, cancelled| {
            let mut last = Instant::now();
            let result = download(&update, &cancelled, |received| {
                if last.elapsed() >= Duration::from_millis(100) || received == update.setup.size {
                    let _ = sender.send(Event::Progress(received));
                    ctx.request_repaint();
                    last = Instant::now();
                }
            });
            let _ = sender.send(Event::Downloaded(result));
            ctx.request_repaint();
        });
    }

    fn start(
        &mut self,
        ctx: &egui::Context,
        work: impl FnOnce(mpsc::Sender<Event>, egui::Context, Arc<AtomicBool>) + Send + 'static,
    ) {
        self.cancelled = Arc::new(AtomicBool::new(false));
        let cancelled = self.cancelled.clone();
        let ctx = ctx.clone();
        let (sender, events) = mpsc::channel();
        match std::thread::Builder::new()
            .name("opencrate-updates".into())
            .spawn(move || work(sender, ctx, cancelled))
        {
            Ok(_) => self.events = Some(events),
            Err(error) => {
                self.status = Status::Failed;
                self.error = Some(error.to_string());
            }
        }
    }

    pub fn cancel(&mut self) {
        self.cancelling = true;
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn poll(&mut self, ctx: &egui::Context, automatic: bool) {
        while let Some(events) = &self.events {
            let event = match events.try_recv() {
                Ok(event) => event,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.events = None;
                    self.status = Status::Failed;
                    self.error = Some("Update worker stopped unexpectedly.".into());
                    break;
                }
            };
            match event {
                Event::Progress(received) => self.received = received,
                Event::Checked(result) => {
                    self.events = None;
                    match result {
                        Ok(release) => {
                            self.status = if release.is_some() {
                                Status::Available
                            } else {
                                Status::Current
                            };
                            self.release = release;
                        }
                        Err(error) => {
                            self.status = Status::Failed;
                            self.error = Some(error);
                        }
                    }
                }
                Event::Downloaded(result) => {
                    self.events = None;
                    if self.cancelling {
                        // A late cancel also discards a completed download.
                        drop(result);
                        self.status = Status::Available;
                        self.cancelling = false;
                    } else {
                        match result {
                            Ok(installer) => {
                                self.ready = Some(installer);
                                self.status = Status::Ready;
                            }
                            Err(error) => {
                                self.status = Status::Failed;
                                self.error = Some(error);
                            }
                        }
                    }
                }
            }
        }
        if automatic && !self.available() && !self.busy() && Instant::now() >= self.next_check {
            self.check(ctx);
        }
        if self.busy() {
            ctx.request_repaint_after(Duration::from_millis(200));
        } else if automatic && !self.available() {
            ctx.request_repaint_after(self.next_check.saturating_duration_since(Instant::now()));
        }
    }

    pub fn take_installer(&mut self) -> Option<Installer> {
        self.ready.take()
    }
    pub fn total(&self) -> u64 {
        self.release.as_ref().map_or(0, |r| r.setup.size)
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl crate::App {
    pub(crate) fn updates_card(&mut self, ui: &mut egui::Ui) {
        use crate::{
            i18n::{self, t},
            theme::{card, palette, subtitle},
        };
        use egui::{ProgressBar, RichText, ViewportCommand};
        let colors = palette(ui);
        card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            subtitle(ui, t("Updates"));
            ui.label(i18n::f("Installed version: {version}", &[("version", env!("CARGO_PKG_VERSION").into())]));
            if ui.checkbox(&mut self.store.preferences.check_updates, t("Automatically check for updates")).changed() {
                self.store.changed();
            }
            ui.label(RichText::new(t("Checks GitHub on launch and every 24 hours. Downloads start only when you choose.")).small().color(colors.muted));
            ui.add_space(8.0);
            match self.updates.status {
                Status::Checking => { ui.horizontal(|ui| { ui.spinner(); ui.label(t("Checking for updates…")); }); }
                Status::Current => { ui.colored_label(colors.green, t("No newer stable release is available.")); }
                Status::Downloading => {
                    let total = self.updates.total();
                    let fraction = self.updates.received as f32 / total.max(1) as f32;
                    let text = i18n::f("Downloading: {received} / {total} MB", &[
                        ("received", format!("{:.1}", self.updates.received as f64 / 1_048_576.0)),
                        ("total", format!("{:.1}", total as f64 / 1_048_576.0)),
                    ]);
                    ui.add(ProgressBar::new(fraction).text(text));
                    if self.updates.cancelling { ui.label(t("Cancelling download…")); }
                    else if ui.button(t("Cancel download")).clicked() { self.updates.cancel(); }
                }
                Status::Ready => { ui.colored_label(colors.green, t("Download verified. Ready to install.")); }
                _ => {}
            }
            if let Some(release) = &self.updates.release {
                ui.label(RichText::new(i18n::f("Version {version} is available", &[("version", release.version.to_string())])).strong());
                if !release.notes.trim().is_empty() {
                    ui.collapsing(t("Release notes"), |ui| {
                        egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| { ui.label(&release.notes); });
                    });
                }
            }
            if let Some(error) = &self.updates.error {
                ui.colored_label(colors.red, i18n::f("Update failed: {details}", &[("details", t(error).into())]));
            }
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!self.updates.busy() && self.updates.status != Status::Ready,
                    egui::Button::new(t("Check for updates"))).clicked() { self.updates.check(ui.ctx()); }
                if self.updates.available() && !self.updates.busy() && self.updates.status != Status::Ready
                    && ui.button(t("Download update")).clicked() { self.updates.download(ui.ctx()); }
                if self.updates.status == Status::Ready && ui.button(t("Install update")).clicked() {
                    self.store.flush();
                    if self.store.error.is_some() {
                        self.updates.error = Some("Save your settings successfully before installing the update.".into());
                    } else if let Some(installer) = self.updates.take_installer() {
                        self.install_after_exit = Some((installer, self.store.preferences.language));
                        self.quit_requested = true;
                        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                    }
                }
            });
            if self.updates.available() {
                ui.label(RichText::new(t("Installing closes OpenCrate and opens Setup. Your preferences are kept; Setup can reopen the app when finished.")).small().color(colors.muted));
            }
        });
    }
}

pub fn show_launch_error(error: &str, language: crate::i18n::Language) {
    let message = crate::i18n::format_for(
        language,
        "Update failed: {details}",
        &[("details", error.into())],
    );
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "OpenCrate".encode_utf16().chain(Some(0)).collect();
        // SAFETY: Both strings are NUL-terminated and live through the modal call.
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
    #[cfg(not(windows))]
    eprintln!("{message}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn release(version: &str) -> Release {
        let name = format!("OpenCrate-{version}-windows-x64-setup.exe");
        Release {
            tag_name: format!("v{version}"),
            draft: false,
            prerelease: false,
            body: Some("Release notes".into()),
            assets: [&name, &format!("{name}.sha256")]
                .into_iter()
                .map(|name| Asset {
                    name: name.clone(),
                    state: "uploaded".into(),
                    size: 100,
                    browser_download_url: format!("{DOWNLOAD_ROOT}v{version}/{name}"),
                })
                .collect(),
        }
    }

    #[test]
    fn versions_use_numeric_order_and_never_downgrade() {
        let current = Version::parse("0.9.0").unwrap();
        assert_eq!(
            select_release(release("0.10.0"), &current)
                .unwrap()
                .unwrap()
                .version,
            Version::new(0, 10, 0)
        );
        for version in ["0.8.0", "0.9.0", "0.9.0+build", "1.0.0-beta.1"] {
            assert!(select_release(release(version), &current)
                .unwrap()
                .is_none());
        }
        assert!(
            select_release(release("0.10.0"), &Version::parse("0.10.0-beta.1").unwrap())
                .unwrap()
                .is_some()
        );
        let mut draft = release("1.0.0");
        draft.draft = true;
        assert!(select_release(draft, &current).unwrap().is_none());
        let mut preview = release("1.0.0");
        preview.prerelease = true;
        assert!(select_release(preview, &current).unwrap().is_none());
        assert!(select_release(release("broken"), &current).is_err());
    }

    #[test]
    fn only_complete_exact_publisher_assets_are_accepted() {
        let current = Version::new(0, 1, 0);
        let mut fixtures = Vec::new();
        let mut missing = release("1.0.0");
        missing.assets.pop();
        fixtures.push(missing);
        let mut duplicate = release("1.0.0");
        duplicate.assets.push(duplicate.assets[0].clone());
        fixtures.push(duplicate);
        let mut pending = release("1.0.0");
        pending.assets[0].state = "new".into();
        fixtures.push(pending);
        let mut empty = release("1.0.0");
        empty.assets[0].size = 0;
        fixtures.push(empty);
        let mut large = release("1.0.0");
        large.assets[0].size = MAX_INSTALLER + 1;
        fixtures.push(large);
        let mut large_checksum = release("1.0.0");
        large_checksum.assets[1].size = 4097;
        fixtures.push(large_checksum);
        for url in [
            "http://github.com/yibudak/opencrate/releases/download/v1.0.0/OpenCrate-1.0.0-windows-x64-setup.exe",
            "https://github.com/another/project/releases/download/v1.0.0/setup.exe",
            "https://example.com/setup.exe",
        ] {
            let mut external = release("1.0.0"); external.assets[0].browser_download_url = url.into(); fixtures.push(external);
        }
        for fixture in fixtures {
            assert!(select_release(fixture, &current).is_err());
        }
        let mut extra = release("1.0.0");
        extra.assets.push(Asset {
            name: "source.zip".into(),
            size: 1,
            state: "uploaded".into(),
            browser_download_url: "unused".into(),
        });
        assert!(select_release(extra, &current).unwrap().is_some());
    }

    #[test]
    fn checksum_must_name_the_exact_installer() {
        let hash = format!("{:x}", Sha256::digest(b"setup"));
        let expected: [u8; 32] = Sha256::digest(b"setup").into();
        assert_eq!(
            parse_checksum(
                format!("\u{feff}{}  setup.exe\r\n", hash.to_uppercase()).as_bytes(),
                "setup.exe"
            )
            .unwrap(),
            expected
        );
        for text in [
            hash.clone(),
            format!("{hash} other.exe"),
            format!("{hash} ../setup.exe"),
            format!("{hash} setup.exe extra"),
            format!("{} setup.exe", "z".repeat(64)),
        ] {
            assert!(parse_checksum(text.as_bytes(), "setup.exe").is_err());
        }
        assert!(parse_checksum(&[255], "setup.exe").is_err());
        assert!(read_limited(Cursor::new(b"12345"), 4).is_err());
        assert_eq!(read_limited(Cursor::new(b"1234"), 4).unwrap(), b"1234");
    }

    #[test]
    fn corrupt_truncated_oversized_cancelled_or_failed_downloads_cannot_be_ready() {
        let bytes = b"verified installer bytes";
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        let cancelled = AtomicBool::new(false);
        let mut output = Vec::new();
        let mut progress = 0;
        verify_stream(
            Cursor::new(bytes),
            &mut output,
            bytes.len() as u64,
            digest,
            &cancelled,
            |value| progress = value,
        )
        .unwrap();
        assert_eq!(output, bytes);
        assert_eq!(progress, bytes.len() as u64);
        for (input, size, hash) in [
            (&bytes[..bytes.len() - 1], bytes.len() as u64, digest),
            (&bytes[..], bytes.len() as u64 - 1, digest),
            (&bytes[..], bytes.len() as u64, [0; 32]),
            (&bytes[..0], 0, digest),
        ] {
            assert!(verify_stream(
                Cursor::new(input),
                std::io::sink(),
                size,
                hash,
                &cancelled,
                |_| {}
            )
            .is_err());
        }
        cancelled.store(true, Ordering::Relaxed);
        let mut output = Vec::new();
        assert!(verify_stream(
            Cursor::new(bytes),
            &mut output,
            bytes.len() as u64,
            digest,
            &cancelled,
            |_| {}
        )
        .is_err());
        assert!(output.is_empty());
        struct BrokenReader;
        impl Read for BrokenReader {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("connection interrupted"))
            }
        }
        cancelled.store(false, Ordering::Relaxed);
        assert!(verify_stream(
            BrokenReader,
            std::io::sink(),
            10,
            digest,
            &cancelled,
            |_| {}
        )
        .is_err());
        struct FullDisk;
        impl Write for FullDisk {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert!(verify_stream(
            Cursor::new(bytes),
            FullDisk,
            bytes.len() as u64,
            digest,
            &cancelled,
            |_| {}
        )
        .is_err());
    }

    fn fake_installer() -> Installer {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("not-an-executable.exe");
        fs::write(&path, b"not executable").unwrap();
        let verified_file = File::open(&path).unwrap();
        Installer {
            verified_file,
            path,
            directory,
        }
    }

    #[test]
    fn worker_completion_cancel_race_and_disconnect_are_handled() {
        let ctx = egui::Context::default();
        let mut state = State::default();
        state.poll(&ctx, false);
        assert!(state.status == Status::Idle);
        let (tx, rx) = mpsc::channel();
        state.events = Some(rx);
        state.status = Status::Checking;
        tx.send(Event::Checked(Ok(select_release(
            release("1.0.0"),
            &Version::new(0, 1, 0),
        )
        .unwrap())))
            .unwrap();
        state.poll(&ctx, false);
        assert!(state.status == Status::Available);
        let (tx, rx) = mpsc::channel();
        state.events = Some(rx);
        state.status = Status::Downloading;
        let installer = fake_installer();
        let path = installer.path.clone();
        state.cancel();
        tx.send(Event::Downloaded(Ok(installer))).unwrap();
        state.poll(&ctx, false);
        assert!(state.status == Status::Available);
        assert!(state.take_installer().is_none());
        assert!(!path.exists());
        let (tx, rx) = mpsc::channel();
        state.events = Some(rx);
        state.status = Status::Downloading;
        drop(tx);
        state.poll(&ctx, false);
        assert!(state.status == Status::Failed);
        assert!(state.error.is_some());
    }

    #[test]
    #[allow(clippy::field_reassign_with_default)] // State implements Drop, so struct update is unavailable.
    fn ready_installer_is_transferred_once_and_removed_if_unused() {
        let ctx = egui::Context::default();
        let (tx, rx) = mpsc::channel();
        let mut state = State::default();
        state.status = Status::Downloading;
        state.events = Some(rx);
        let installer = fake_installer();
        let path = installer.path.clone();
        tx.send(Event::Downloaded(Ok(installer))).unwrap();
        state.poll(&ctx, false);
        assert!(state.status == Status::Ready);
        state.check(&ctx); // Ready downloads must not be replaced by a second check.
        assert!(state.status == Status::Ready);
        let installer = state.take_installer().unwrap();
        assert!(state.take_installer().is_none());
        assert!(path.exists());
        drop(installer);
        assert!(!path.exists());
    }

    #[test]
    fn failed_installer_launch_keeps_no_download() {
        let installer = fake_installer();
        let path = installer.path.clone();
        assert!(installer.launch(crate::i18n::Language::English).is_err());
        assert!(!path.exists());
    }

    #[test]
    #[ignore = "Contacts public GitHub and downloads the real installer; never executes it"]
    fn github_download_smoke() {
        let update = check(&Version::new(0, 0, 0))
            .unwrap()
            .expect("published stable release");
        let installer = download(&update, &AtomicBool::new(false), |_| {}).unwrap();
        let path = installer.path.clone();
        assert_eq!(fs::metadata(&path).unwrap().len(), update.setup.size);
        #[cfg(windows)]
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        drop(installer);
        assert!(!path.exists());
    }
}
