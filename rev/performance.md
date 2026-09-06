# UI resource usage

OpenCrate uses egui 0.33, egui-winit, the cached `egui_software_backend` rasterizer and softbuffer. On Windows, softbuffer presents through GDI. The application does not initialize an OpenGL, Vulkan or DirectX rendering device.

The native event loop waits for input, worker notifications or the next egui deadline. It also services tray actions and pending preference writes while the window is hidden; those operations do not depend on Windows delivering a redraw event. Drawing buffers, cached rasterized meshes and the software surface are dropped when the window is hidden, minimized or occluded. A bounded map retains only current texture images, applying font-atlas patches in place, so reopening can reconstruct the renderer without losing glyphs or the logo.

Lighting, fan and power controllers remain owned by the application, with their original shutdown behavior. Closing the window does not destroy those controllers or reset fan curves. Their hardware logic and polling intervals are unchanged.

## CPU behavior

The decorative lighting preview requests a repaint approximately every 50 ms only while its page is drawn and the application is focused. Actual LED output still uses its independent playback worker. The changing LED mesh is separate from the static dashboard so the rasterizer can cache the surrounding interface. Mouse input and other interactive UI changes can repaint immediately.

egui subtracts its predicted frame time from delayed repaint requests. This host supplies measured drawing time instead of the default GPU-vsync estimate, and ignores obsolete repaint callbacks. This avoids accidentally turning a 33 ms repaint request into a roughly 16 ms timer. The event loop has no continuous polling mode.

## Native regression check

Run on Windows with the repository's Rust/MinGW tools:

```powershell
python scripts/test-ui-runtime.py
python scripts/test-ui-runtime.py --start-hidden
```

The script builds `opencrate-ui` with the opt-in `diagnostics` feature. Its diagnostic mode does not start any hardware controller, does not use the normal single-instance names, and uses new temporary preferences. Normal release and installer builds omit that mode. The installed application can remain running.

The 50-second scenario covers idle rendering, closing to the tray, reopening, Turkish and Chinese glyphs, fan and power pages, preview animation, hiding during animation, UI scaling, and normal Quit. It checks that tray phases produce zero paints, that idle phases do not continuously redraw, that the steady animation frame budget is respected, that hidden private commit stays below 32 MiB, and that expected frame captures exist. Frame-rate checks exclude the first 1.5 seconds to allow egui's short scroll/zoom transitions to settle. It fails if the application crashes or does not exit. Both launch modes run in Windows CI before building the normal installer.

Each run writes PNG captures, phase/update/paint counts, and memory/CPU samples to a new `target/ui-runtime-*` directory. Inspect the images when changing the renderer, scale handling or texture lifecycle. Unit tests separately cover partial font-atlas updates, texture frees, bounded texture backing, repaint deadline coalescing, and all translated glyphs. Existing hardware and preference tests remain in the workspace suite.

## Interpreting measurements

`private_resident_mib` is the private working set, similar to the commonly displayed Task Manager process-memory value. `working_set_mib` also includes shareable resident pages. `private_commit_mib` includes private committed allocations whether or not they are currently resident. These values must not be added together.

The script uses read-only Windows process APIs and never trims the working set. CPU percentages divide process CPU time by elapsed time and the logical processor count, matching whole-machine percentage units. Steady samples exclude the first 1.5 seconds of each phase, so startup, screenshot encoding and transitions do not dominate the result. A reported 0.0 means no CPU time was observed in that sample interval, not that the application can never consume CPU.

On a local Windows x64 machine, the old installed OpenGL application had about 102 MiB private resident memory and 8.4% CPU, with the hot thread identified as the main UI thread. The hardware-free software-rendered lifecycle measured about 20–22 MiB private resident memory while open, 5–7 MiB after hiding, and no observed steady idle CPU time. These are different workloads: the installed app had live hardware and a saved lighting animation, while the reproducible harness uses disabled hardware backends. They establish the UI savings direction, not an exact end-to-end speedup or a universal memory guarantee. Window size, DPI, active pages, driver/runtime versions and real hardware activity affect totals.

Earlier isolated experiments found that switching the OpenGL renderer to wgpu/DirectX 12 increased private commit, and Vulkan increased resident memory too. Destroying a graphics context in a still-running process also left substantial native allocations resident. Software rendering removes that driver dependency while retaining one process and the existing hardware ownership model.
