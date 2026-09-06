"""Manual Windows GUI regression check. Requires Python + Pillow and a debug build.

Uses only generated fixtures and the debug-only F12 renderer capture hook.
Does not read the desktop, open other apps, or inspect the user's clipboard.
"""
import ctypes
from ctypes import wintypes as w
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "artifacts" / "ui-smoke"
OUT.mkdir(parents=True, exist_ok=True)
capture = OUT / "current.png"
state_file = capture.with_suffix(".txt")
user = ctypes.WinDLL("user32", use_last_error=True)
user.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
user.GetWindowThreadProcessId.argtypes = [w.HWND, ctypes.POINTER(w.DWORD)]
user.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, ctypes.c_int]
user.SetWindowPos.argtypes = [w.HWND, w.HWND, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int, w.UINT]
callback_type = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
user.EnumWindows.argtypes = [callback_type, w.LPARAM]

fixture = ROOT / "artifacts" / "fixtures" / "image1.jpg"
if not fixture.exists():
    raise SystemExit("Run the fixtures example first (see docs/PERFORMANCE.md).")
env = dict(os.environ, KOVA_TEST_CAPTURE=str(capture), LOCALAPPDATA=str(OUT / "local-settings"))
if state_file.exists():
    state_file.unlink()
log = open(OUT / "viewer.log", "w", encoding="utf-8")
process = subprocess.Popen([str(ROOT / "target/debug/kova-image.exe"), str(fixture)], env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
hwnd = None


def wait_for(predicate, seconds=10):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if predicate():
            return
        if process.poll() is not None:
            raise AssertionError(f"Viewer exited: {process.returncode}")
        time.sleep(0.05)
    raise AssertionError("Timed out waiting for the viewer")


def key(vk):
    scan = user.MapVirtualKeyW(vk, 0)
    user.PostMessageW(hwnd, 0x100, vk, 1 | (scan << 16))
    if 0x30 <= vk <= 0x39 or 0x41 <= vk <= 0x5A:
        user.PostMessageW(hwnd, 0x102, ord(chr(vk).lower()), 1 | (scan << 16))
    elif vk == 0xBB:
        user.PostMessageW(hwnd, 0x102, ord("="), 1 | (scan << 16))
    user.PostMessageW(hwnd, 0x101, vk, 1 | (scan << 16) | 0xC0000000)


def click(x, y):
    point = x | (y << 16)
    user.PostMessageW(hwnd, 0x200, 0, point)
    user.PostMessageW(hwnd, 0x201, 1, point)
    user.PostMessageW(hwnd, 0x202, 0, point)


def snapshot(name):
    time.sleep(0.18)
    old = state_file.stat().st_mtime_ns if state_file.exists() else 0
    key(0x7B)  # F12, debug-only capture
    wait_for(lambda: state_file.exists() and state_file.stat().st_mtime_ns != old)
    values = dict(line.split("=", 1) for line in state_file.read_text().splitlines() if "=" in line)
    if values.get("status", "").startswith("Loading"):
        deadline = time.monotonic() + 20
        while values.get("status", "").startswith("Loading"):
            if time.monotonic() > deadline:
                raise AssertionError("Image did not finish loading")
            time.sleep(0.2)
            old = state_file.stat().st_mtime_ns
            key(0x7B)
            wait_for(lambda: state_file.stat().st_mtime_ns != old)
            values = dict(line.split("=", 1) for line in state_file.read_text().splitlines() if "=" in line)
    (OUT / (name + ".png")).write_bytes(capture.read_bytes())
    (OUT / (name + ".txt")).write_text(state_file.read_text())
    return values


try:
    windows = []

    @callback_type
    def enum(window, _):
        pid = w.DWORD()
        user.GetWindowThreadProcessId(window, ctypes.byref(pid))
        title = ctypes.create_unicode_buffer(256)
        user.GetWindowTextW(window, title, len(title))
        if pid.value == process.pid and title.value == "Kova Image":
            windows.append(window)
        return True

    wait_for(lambda: (user.EnumWindows(enum, 0), bool(windows))[1])
    hwnd = windows[0]
    wait_for(state_file.exists)
    assert snapshot("01-fit")["filename"] == "image1.jpg"
    key(0x27)
    assert snapshot("02-next")["filename"] == "image2.png"
    key(0x23)
    assert snapshot("03-last")["filename"] == "image20-large.png"
    key(0x24)
    assert snapshot("04-first")["filename"] == "image1.jpg"
    key(ord("1"))
    assert snapshot("05-actual")["zoom"] == "100%"
    key(0xBB)
    assert snapshot("06-zoom")["zoom"] == "120%"
    key(ord("R"))
    assert snapshot("07-rotate")["rotation"] == "90"
    key(ord("H"))
    assert snapshot("08-flip")["flip_h"] == "true"
    key(ord("0"))
    key(0x7A)
    assert snapshot("09-fullscreen")["fullscreen"] == "true"
    key(0x1B)
    assert snapshot("10-windowed")["fullscreen"] == "false"
    key(0x27)
    key(0x27)
    wait_for(lambda: "image3.gif" in snapshot("11-animation")["filename"])
    key(0x20)
    paused = snapshot("12-paused")
    assert paused["paused"] == "true"
    time.sleep(0.4)
    assert snapshot("13-still-paused")["frame"] == paused["frame"]
    key(0x20)
    assert snapshot("14-resumed")["paused"] == "false"
    key(ord("I"))
    snapshot("15-info")
    key(0x1B)
    user.SetWindowPos(hwnd, None, 0, 0, 660, 460, 0x0006)
    snapshot("16-small-window")
    user.SetWindowPos(hwnd, None, 0, 0, 1080, 740, 0x0006)
    time.sleep(0.3)
    click(1040, 714)
    snapshot("17-more")
    click(960, 597)
    snapshot("18-settings")
    key(0x1B)
    if "--clipboard" in sys.argv:
        click(1040, 714)
        time.sleep(0.2)
        click(940, 432)
        assert snapshot("19-copy-path")["status"] == "Path copied"
        click(1040, 714)
        time.sleep(0.2)
        click(940, 399)
        assert snapshot("20-copy-image")["status"] == "Image copied"
        print("PASS: native Copy Path and Copy Image (clipboard now contains the generated fixture)")
    print("PASS: CLI, next/first/last, natural order, zoom, rotation, flip, fullscreen, animation pause/resume, info, resize")
finally:
    if hwnd:
        user.PostMessageW(hwnd, 0x10, 0, 0)
    try:
        process.wait(timeout=4)
    except subprocess.TimeoutExpired:
        process.terminate()
    log.close()
