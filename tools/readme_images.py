"""Regenerate the README screenshots from the live window (docs/images/*.png).

    python tools/readme_images.py <path to BriskMarkdown.exe>

Uses the app's BRISK_SNAPSHOT test hook, which renders the window (document, status bar,
scrollbar) to PNG without needing a visible desktop.
"""
import ctypes
import ctypes.wintypes as wt
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

user32 = ctypes.windll.user32
user32.SendMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
user32.PostMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
ROOT = Path(__file__).resolve().parent.parent
EXE = Path(sys.argv[1])
OUT = ROOT / "docs" / "images"
OUT.mkdir(parents=True, exist_ok=True)


def find_window(pid):
    found = []

    @ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    def cb(hwnd, _):
        p = wt.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(p))
        buf = ctypes.create_unicode_buffer(64)
        user32.GetClassNameW(hwnd, buf, 64)
        if p.value == pid and buf.value == "BriskMarkdownWindow":
            found.append(hwnd)
        return True

    for _ in range(100):
        user32.EnumWindows(cb, 0)
        if found:
            return found[0]
        time.sleep(0.05)
    raise RuntimeError("window not found")


tmp = Path(tempfile.mkdtemp())
doc = tmp / "showcase.md"
shutil.copy(ROOT / "docs" / "samples" / "showcase.md", doc)
# A private settings file next to a private copy of the exe keeps the user's settings untouched.
exe = tmp / "BriskMarkdown.exe"
shutil.copy(EXE, exe)
(tmp / "BriskMarkdown.ini").write_text("theme=Dark\n")
proc = subprocess.Popen([str(exe), str(doc)], env=dict(os.environ, BRISK_SNAPSHOT=str(tmp)))
hwnd = find_window(proc.pid)
user32.MoveWindow(hwnd, 40, 40, 1100, 760, True)
time.sleep(0.6)


def snap(n, name):
    user32.SendMessageW(hwnd, 0x8002, n, 0)
    time.sleep(0.2)
    shutil.copy(tmp / f"snap-{n}.png", OUT / name)


def command(cmd):
    user32.PostMessageW(hwnd, 0x111, cmd, 0)
    user32.SendMessageW(hwnd, 0, 0, 0)
    time.sleep(0.3)


user32.PostMessageW(hwnd, 0x100, 0x26, 0)  # put the caret at the top
time.sleep(0.2)
snap(1, "dark.png")
command(401)  # Mono
snap(2, "mono.png")
command(401)
command(411)  # Light
snap(3, "light.png")
user32.PostMessageW(hwnd, 0x10, 0, 0)
proc.wait(5)
shutil.rmtree(tmp, ignore_errors=True)
print("wrote", sorted(p.name for p in OUT.glob("*.png")))
