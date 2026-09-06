"""Run the opt-in, hardware-free Windows UI lifecycle and resource regression.

Requires the Rust/MinGW build tools and Python 3.10+. Writes measurements and
rendered PNGs to a new target directory; never reads real device settings.
"""

import argparse
import ctypes as c
import json
import os
import statistics
import subprocess
import tempfile
import time
from ctypes import wintypes as w
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


class Counters(c.Structure):
    _fields_ = [("cb", w.DWORD), ("faults", w.DWORD)] + [
        (name, c.c_size_t)
        for name in (
            "peak_ws",
            "ws",
            "peak_paged",
            "paged",
            "peak_nonpaged",
            "nonpaged",
            "pagefile",
            "peak_pagefile",
            "private",
        )
    ]


class Process:
    def __init__(self, pid):
        self.kernel = c.WinDLL("kernel32", use_last_error=True)
        self.psapi = c.WinDLL("psapi", use_last_error=True)
        self.kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
        self.kernel.OpenProcess.restype = w.HANDLE
        self.kernel.CloseHandle.argtypes = [w.HANDLE]
        self.kernel.GetProcessTimes.argtypes = [w.HANDLE] + [c.POINTER(w.FILETIME)] * 4
        self.psapi.GetProcessMemoryInfo.argtypes = [w.HANDLE, c.POINTER(Counters), w.DWORD]
        self.psapi.QueryWorkingSet.argtypes = [w.HANDLE, c.c_void_p, w.DWORD]
        self.handle = self.kernel.OpenProcess(0x410, False, pid)
        if not self.handle:
            raise c.WinError(c.get_last_error())
        self.pages = (c.c_size_t * 262144)()

    def sample(self):
        counters = Counters()
        counters.cb = c.sizeof(counters)
        if not self.psapi.GetProcessMemoryInfo(self.handle, c.byref(counters), counters.cb):
            raise c.WinError(c.get_last_error())
        times = [w.FILETIME() for _ in range(4)]
        if not self.kernel.GetProcessTimes(self.handle, *(c.byref(t) for t in times)):
            raise c.WinError(c.get_last_error())
        if not self.psapi.QueryWorkingSet(self.handle, self.pages, c.sizeof(self.pages)):
            raise c.WinError(c.get_last_error())
        private_pages = sum(not (page & 256) for page in self.pages[1 : self.pages[0] + 1])
        cpu = sum((t.dwHighDateTime << 32) | t.dwLowDateTime for t in times[2:]) / 10_000_000
        return {
            "cpu_seconds": cpu,
            "private_commit_mib": counters.private / 1048576,
            "private_resident_mib": private_pages * 4096 / 1048576,
            "working_set_mib": counters.ws / 1048576,
        }

    def close(self):
        self.kernel.CloseHandle(self.handle)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--start-hidden",
        action="store_true",
        help="Also verify a cold launch directly into the tray",
    )
    args = parser.parse_args()
    if os.name != "nt":
        parser.error("This native lifecycle check requires Windows")
    subprocess.run(
        [
            "cargo",
            "build",
            "--locked",
            "--release",
            "-p",
            "opencrate-ui",
            "--features",
            "diagnostics",
        ],
        cwd=ROOT,
        check=True,
    )
    (ROOT / "target").mkdir(exist_ok=True)
    output = Path(tempfile.mkdtemp(prefix="ui-runtime-", dir=ROOT / "target"))
    env = dict(os.environ, OPENCRATE_DIAGNOSTIC_OUTPUT=str(output))
    env.pop("OPENCRATE_DIAGNOSTIC_START_HIDDEN", None)
    if args.start_hidden:
        env["OPENCRATE_DIAGNOSTIC_START_HIDDEN"] = "1"
    samples = []
    with (output / "stderr.log").open("w", encoding="utf-8") as stderr:
        child = subprocess.Popen(
            [str(ROOT / "target/release/opencrate-ui.exe")],
            cwd=ROOT,
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=stderr,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        process = Process(child.pid)
        started = time.monotonic()
        try:
            while child.poll() is None:
                elapsed = time.monotonic() - started
                if elapsed > 75:
                    raise TimeoutError("UI did not exit after the diagnostic Quit")
                try:
                    sample = process.sample()
                except OSError:
                    if child.poll() is not None:
                        break
                    raise
                sample["seconds"] = elapsed
                samples.append(sample)
                time.sleep(0.5)
            if child.returncode != 0:
                raise RuntimeError(f"Diagnostic failed; see {output / 'stderr.log'}")
        finally:
            process.close()
            if child.poll() is None:
                child.kill()
                child.wait()

    lifecycle = json.loads((output / "lifecycle.json").read_text(encoding="utf-8"))
    (output / "samples.json").write_text(json.dumps(samples, indent=2), encoding="utf-8")
    assert lifecycle["ok"], lifecycle
    phases = lifecycle["phases"]
    assert len(phases) == 11, phases
    assert phases[-1]["phase"] == "tray-quit", phases[-1]
    measurements = []
    previous_end = 0.0
    for phase in phases:
        # Exclude initialization, captures and transitions from steady measurements.
        stable = [
            s for s in samples if previous_end + 1.5 < s["seconds"] < phase["elapsed_seconds"] - 0.3
        ]
        previous_end = phase["elapsed_seconds"]
        assert len(stable) >= 2, f"Not enough samples for {phase['phase']}"
        first, last = stable[0], stable[-1]
        cpu = (
            100
            * (last["cpu_seconds"] - first["cpu_seconds"])
            / (last["seconds"] - first["seconds"])
            / os.cpu_count()
        )
        measurements.append(
            {
                **phase,
                "cpu_percent": round(cpu, 3),
                **{
                    key: round(statistics.median(s[key] for s in stable), 2)
                    for key in (
                        "private_commit_mib",
                        "private_resident_mib",
                        "working_set_mib",
                    )
                },
            }
        )
        if "tray" in phase["phase"]:
            assert phase["paints"] == 0, f"Rendered while hidden: {phase}"
            assert measurements[-1]["private_commit_mib"] < 32, (
                "Hidden frame buffers were not released"
            )
        if phase["phase"] not in ("animation", "scaled-reopen"):
            assert phase["steady_updates"] < 30, f"Unexpected continuous repaint: {phase}"
        else:
            # egui's scroll/zoom transitions can repaint faster while settling.
            # Allow timer jitter around 20 Hz, but reject a 30/60 Hz idle loop.
            fps = phase["steady_paints"] / phase["steady_seconds"]
            assert fps > 5, f"Preview animation stopped: {phase}"
            assert fps < 27, f"Steady animation exceeded its 20 Hz budget: {phase}"
    for index in (0, 2, 3, 4, 5, 6, 7, 9):
        if index == 0 and args.start_hidden:
            continue
        assert (output / f"{index:02}.png").is_file(), f"Missing rendered frame {index}"
    report = {"logical_processors": os.cpu_count(), "phases": measurements, "samples": samples}
    (output / "measurements.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(measurements, indent=2))
    print(f"Native lifecycle passed. Measurements and screenshots: {output}")


if __name__ == "__main__":
    main()
