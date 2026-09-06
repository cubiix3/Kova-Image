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
env = dict(os.environ, KOVA_TEST_CAPTURE=str(capture), LOCALAPPDATA=str(OUT / "local-settings"), KOVA_TEST_MUTE="1")
if state_file.exists():
    state_file.unlink()
log = open(OUT / "viewer.log", "w", encoding="utf-8")
arguments = [str(ROOT / "target/debug/kova-image.exe")]
scenario = next((arg.split("=", 1)[1] for arg in sys.argv if arg.startswith("--state=")), "")
if "--software" in sys.argv:
    arguments.append("--software")
if scenario == "video":
    settings=Path(env["LOCALAPPDATA"]) / "Kova Image/settings.conf"
    settings.parent.mkdir(parents=True, exist_ok=True)
    settings.write_text("video_autoplay=" + ("false" if "--no-autoplay" in sys.argv else "true") + "\n")
    (ROOT / "artifacts/video-fixtures/clip0.png").write_bytes((fixture.parent / "image2.png").read_bytes())
    arguments.append(str(ROOT / "artifacts/video-fixtures/clip1.mp4"))
elif scenario == "missing":
    arguments.append(str(OUT / "missing-image.png"))
elif scenario == "corrupted":
    damaged = OUT / "damaged-image.png"
    damaged.write_bytes(b"\x89PNG\r\n\x1a\ninvalid-header")
    arguments.append(str(damaged))
elif scenario == "long-name":
    named = OUT / ("Kova image with a deliberately long filename for titlebar truncation " * 2 + ".png")
    named.write_bytes((fixture.parent / "image2.png").read_bytes())
    arguments.append(str(named))
elif scenario != "empty":
    arguments.append(str(fixture))
process = subprocess.Popen(arguments, env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
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
    elif vk in (0x09, 0x0D, 0x20):
        user.PostMessageW(hwnd, 0x102, vk, 1 | (scan << 16))
    user.PostMessageW(hwnd, 0x101, vk, 1 | (scan << 16) | 0xC0000000)


def click(x, y):
    point = x | (y << 16)
    user.PostMessageW(hwnd, 0x200, 0, point)
    user.PostMessageW(hwnd, 0x201, 1, point)
    user.PostMessageW(hwnd, 0x202, 0, point)


def keep_background():
    # The harness posts input to its own HWND. Keep it below the user's windows
    # so real pointer movement does not invalidate an inactivity measurement.
    user.SetWindowPos(hwnd, 1, 0, 0, 0, 0, 0x0013)


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
    keep_background()
    if scenario == "video":
        wait_for(state_file.exists, seconds=25)
        values=snapshot("video-playing")
        assert values["video"] == "true" and float(values["video_duration"]) > 5
        if "--no-autoplay" in sys.argv:
            assert values["paused"] == "true"
        else:
            key(0x20)
        values=snapshot("video-paused")
        assert values["paused"] == "true"
        position=float(values["video_position"])
        time.sleep(0.4)
        assert abs(float(snapshot("video-paused-stable")["video_position"])-position)<0.15
        click(600,710)
        assert float(snapshot("video-seek")["video_position"]) > 3
        click(540,370)
        key(ord("M"))
        assert snapshot("video-unmuted")["muted"] == "false"
        key(ord("M"))
        assert snapshot("video-muted")["muted"] == "true"
        key(ord("I"))
        assert snapshot("video-info")["info"] == "true"
        key(0x1B)
        user.SetWindowPos(hwnd,None,0,0,640,420,0x0006)
        snapshot("video-small")
        user.SetWindowPos(hwnd,None,0,0,1080,740,0x0006)
        key(0x7A)
        time.sleep(0.3)
        keep_background()
        user.PostMessageW(hwnd,0x200,0,540 | (370 << 16))
        time.sleep(2.5)
        assert snapshot("video-fullscreen-hidden")["chrome"] == "false"
        user.PostMessageW(hwnd,0x200,0,541 | (370 << 16))
        assert snapshot("video-fullscreen-awake")["chrome"] == "true"
        key(0x1B)
        key(0x25)
        values=snapshot("video-to-image")
        assert values["filename"]=="clip0.png" and values["video"]=="false"
        key(0x27)
        assert snapshot("image-to-video")["video"]=="true"
        key(0x27)
        key(0x27)
        key(0x27)
        values=snapshot("video-rapid-navigation")
        assert values["filename"]=="clip4.mkv" and values["video"]=="true"
        time.sleep(0.5)
        assert snapshot("video-stale-protection")["filename"]=="clip4.mkv"
        print("PASS: native video frame, pause, stable clock, timeline seek, mute, info, compact layout, fullscreen auto-hide")
    elif scenario:
        time.sleep(0.5)
        values = snapshot("state-" + scenario)
        if scenario in ("missing", "corrupted"):
            assert values["error"], "Expected a dedicated error state"
        else:
            assert not values["error"]
        user.SetWindowPos(hwnd, None, 0, 0, 640, 420, 0x0006)
        snapshot("state-" + scenario + "-small")
        key(0x09)
        assert snapshot("state-" + scenario + "-focus")["focused"] == "true"
        print("PASS: state", scenario)
    else:
        wait_for(state_file.exists)
        assert snapshot("01-fit")["filename"] == "image1.jpg"
        key(0x27)
        assert snapshot("02-next")["filename"] == "image2.png"
        user.PostMessageW(hwnd, 0x20B, 1 << 16, 540 | (370 << 16))
        user.PostMessageW(hwnd, 0x20C, 1 << 16, 540 | (370 << 16))
        assert snapshot("02a-mouse-back")["filename"] == "image1.jpg"
        user.PostMessageW(hwnd, 0x20B, 2 << 16, 540 | (370 << 16))
        user.PostMessageW(hwnd, 0x20C, 2 << 16, 540 | (370 << 16))
        assert snapshot("02b-mouse-forward")["filename"] == "image2.png"
        key(0x23)
        assert snapshot("03-last")["filename"] == "image20-large.png"
        key(0x24)
        assert snapshot("04-first")["filename"] == "image1.jpg"
        key(ord("1"))
        assert snapshot("05-actual")["zoom"] == "100%"
        user.PostMessageW(hwnd, 0x200, 0, 540 | (370 << 16))
        user.PostMessageW(hwnd, 0x201, 1, 540 | (370 << 16))
        user.PostMessageW(hwnd, 0x200, 1, 640 | (420 << 16))
        user.PostMessageW(hwnd, 0x202, 0, 640 | (420 << 16))
        pan = snapshot("05a-pan")
        assert float(pan["pan_x"]) == 100 and float(pan["pan_y"]) == 50
        key(0xBB)
        assert snapshot("06-zoom")["zoom"] == "120%"
        user.PostMessageW(hwnd, 0x20A, 120 << 16, 640 | (420 << 16))
        assert snapshot("06a-wheel")["zoom"] == "144%"
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
        assert snapshot("15-info")["info"] == "true"
        key(0x1B)
        user.SetWindowPos(hwnd, None, 0, 0, 660, 460, 0x0006)
        snapshot("16-small-window")
        user.SetWindowPos(hwnd, None, 0, 0, 1080, 740, 0x0006)
        time.sleep(0.3)
        click(1040, 714)
        assert snapshot("17-more")["more"] == "true"
        click(960, 616)
        assert snapshot("18-settings")["settings"] == "true"
        user.PostMessageW(hwnd,0x200,0,600 | (450 << 16))
        user.PostMessageW(hwnd,0x20A,((-720) & 0xffff) << 16,600 | (450 << 16))
        snapshot("18a-associations")
        key(0x1B)
        if "--clipboard" in sys.argv:
            click(1040, 714)
            time.sleep(0.2)
            click(940, 444)
            assert snapshot("19-copy-path")["status"] == "Path copied"
            click(1040, 714)
            time.sleep(0.2)
            click(940, 410)
            assert snapshot("20-copy-image")["status"] == "Image copied"
            print("PASS: native Copy Path and Copy Image (clipboard now contains the generated fixture)")
        print("PASS: CLI, next/first/last, natural order, zoom, rotation, flip, fullscreen, animation pause/resume, info, resize")
        # Focused toolbar controls activate with Space, rather than pausing the GIF.
        click(556, 704)  # Fit
        key(0x09)        # Tab -> 100%
        assert snapshot("21-keyboard-focus")["focused"] == "true"
        key(0x20)
        assert snapshot("22-keyboard-actual")["actual_active"] == "true"
        click(540, 370)  # Back to the canvas
        key(0x7A)
        time.sleep(0.4)
        keep_background()
        time.sleep(0.2)
        user.PostMessageW(hwnd, 0x200, 0, 540 | (370 << 16))
        time.sleep(2.5)
        assert snapshot("23-fullscreen-hidden")["chrome"] == "false"
        user.PostMessageW(hwnd, 0x200, 0, 541 | (370 << 16))
        assert snapshot("24-fullscreen-awake")["chrome"] == "true"
        key(ord("I"))
        key(0x1B)
        assert snapshot("25-dismiss-info-in-fullscreen")["fullscreen"] == "true"
        key(0x1B)
        click(1040, 704)
        click(400, 180)
        assert snapshot("26-dismiss-more")["more"] == "false"
        user.SetWindowPos(hwnd, None, 0, 0, 640, 420, 0x0006)
        snapshot("27-minimum-size")
        click(606, 384)
        assert snapshot("28-small-more")["more"] == "true"
        key(0x1B)
        print("PASS: focus activation, fullscreen auto-hide/wake, layered Escape, dismissable menu, 640x420")
finally:
    if hwnd:
        user.PostMessageW(hwnd, 0x10, 0, 0)
    try:
        process.wait(timeout=4)
    except subprocess.TimeoutExpired:
        process.terminate()
    log.close()
