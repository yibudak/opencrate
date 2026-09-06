//! Reveal the root window even when hidden-window redraws are suspended.

use eframe::egui;

#[cfg(windows)]
use std::sync::{
    atomic::{AtomicIsize, Ordering},
    Arc,
};

#[derive(Clone)]
pub struct WindowActivation {
    context: egui::Context,
    #[cfg(windows)]
    handle: Arc<AtomicIsize>,
}

impl WindowActivation {
    pub fn new(creation: &eframe::CreationContext<'_>) -> std::io::Result<Self> {
        #[cfg(windows)]
        let handle = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            match creation
                .window_handle()
                .map_err(std::io::Error::other)?
                .as_raw()
            {
                RawWindowHandle::Win32(window) => Arc::new(AtomicIsize::new(window.hwnd.get())),
                _ => return Err(std::io::Error::other("Expected the opencrate Win32 window")),
            }
        };
        Ok(Self {
            context: creation.egui_ctx.clone(),
            #[cfg(windows)]
            handle,
        })
    }

    /// Called directly by the tray callback / instance listener, not App::update.
    pub fn show(&self) {
        #[cfg(windows)]
        {
            use windows_sys::Win32::{
                Foundation::HWND,
                UI::WindowsAndMessaging::{
                    IsIconic, IsWindow, SetForegroundWindow, ShowWindowAsync, SW_RESTORE, SW_SHOW,
                },
            };
            let handle = self.handle.load(Ordering::Acquire) as HWND;
            if handle.is_null() {
                return;
            }
            // SAFETY: This is eframe's own root HWND, never an enumerated window.
            // App::drop invalidates it. Asynchronous show posts to the owning
            // message loop so no egui redraw or cross-thread blocking is needed.
            unsafe {
                if IsWindow(handle) == 0 {
                    return;
                }
                let command = if IsIconic(handle) != 0 {
                    SW_RESTORE
                } else {
                    SW_SHOW
                };
                ShowWindowAsync(handle, command);
                SetForegroundWindow(handle);
            }
        }
        self.context
            .send_viewport_cmd(egui::ViewportCommand::Visible(true));
        self.context
            .send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        self.context.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.context.request_repaint();
    }

    /// Static menu callbacks can outlive App; prevent them using a stale HWND.
    pub fn deactivate(&self) {
        #[cfg(windows)]
        self.handle.store(0, Ordering::Release);
    }
}
