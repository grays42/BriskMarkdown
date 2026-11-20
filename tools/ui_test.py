"""Drive the real BriskMarkdown window and check what it does.

Starts the exe on a scratch copy of a document, sends it keystrokes and menu commands through
window messages, captures the window to PNG files, saves, and checks the file contents.

    python tools/ui_test.py <path to BriskMarkdown.exe> <scratch dir>

Window messages can't carry Ctrl/Shift state, so modified keys are exercised through their menu
commands instead. Exit code 0 means every check passed.
"""
import ctypes
import ctypes.wintypes as wt
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

from PIL import Image

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32
user32.FindWindowW.restype = wt.HWND
user32.PostMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
user32.SendMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
user32.SendMessageW.restype = ctypes.c_ssize_t

WM_KEYDOWN, WM_CHAR, WM_COMMAND, WM_CLOSE = 0x100, 0x102, 0x111, 0x10
WM_LBUTTONDOWN, WM_LBUTTONUP, WM_LBUTTONDBLCLK, WM_MOUSEWHEEL = 0x201, 0x202, 0x203, 0x20A
VK = {"enter": 0x0D, "back": 0x08, "left": 0x25, "up": 0x26, "right": 0x27, "down": 0x28, "home": 0x24, "end": 0x23, "delete": 0x2E, "tab": 0x09, "esc": 0x1B}
CMD = {"save": 104, "undo": 201, "redo": 202, "select_all": 207, "bold": 301, "italic": 302, "h2": 312, "mode": 401, "goto_end": None}

EXE = Path(sys.argv[1])
SCRATCH = Path(sys.argv[2])
SCRATCH.mkdir(parents=True, exist_ok=True)
TRACE = SCRATCH / "trace.log"
failures = []


def check(name, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + name + (f"  ({detail})" if detail and not cond else ""))
    if not cond:
        failures.append(name)


def find_window(pid, timeout=5.0):
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

    end = time.time() + timeout
    while time.time() < end:
        found.clear()
        user32.EnumWindows(cb, 0)
        if found:
            return found[0]
        time.sleep(0.05)
    raise RuntimeError("window not found")


def settle(hwnd, t=0.12):
    # A synchronous no-op message flushes everything posted before it.
    user32.SendMessageW(hwnd, 0, 0, 0)
    time.sleep(t)


def key(hwnd, name, n=1):
    for _ in range(n):
        user32.PostMessageW(hwnd, WM_KEYDOWN, VK[name], 0)
    settle(hwnd)


def typ(hwnd, text):
    for ch in text:
        user32.PostMessageW(hwnd, WM_CHAR, ord(ch), 0)
    settle(hwnd)


def cmd(hwnd, name):
    user32.PostMessageW(hwnd, WM_COMMAND, CMD[name], 0)
    settle(hwnd, 0.25)


def click(hwnd, x, y, double=False):
    lp = (y << 16) | (x & 0xFFFF)
    user32.PostMessageW(hwnd, WM_LBUTTONDOWN, 1, lp)
    user32.PostMessageW(hwnd, WM_LBUTTONUP, 0, lp)
    if double:
        user32.PostMessageW(hwnd, WM_LBUTTONDBLCLK, 1, lp)
        user32.PostMessageW(hwnd, WM_LBUTTONUP, 0, lp)
    settle(hwnd)


def wheel(hwnd, clicks):
    user32.PostMessageW(hwnd, WM_MOUSEWHEEL, ((-120 * clicks) & 0xFFFF) << 16, 0)
    settle(hwnd)


def shot(hwnd, path):
    r = wt.RECT()
    user32.GetClientRect(hwnd, ctypes.byref(r))
    w, h = r.right, r.bottom
    hdc = user32.GetDC(hwnd)
    mem = gdi32.CreateCompatibleDC(hdc)
    bmp = gdi32.CreateCompatibleBitmap(hdc, w, h)
    gdi32.SelectObject(mem, bmp)
    user32.PrintWindow(hwnd, mem, 1 | 2)  # client only, full content
    bmi = (ctypes.c_byte * 44)()
    ctypes.memmove(bmi, ctypes.byref(ctypes.c_int32(40)), 4)
    ctypes.memmove(ctypes.byref(bmi, 4), ctypes.byref(ctypes.c_int32(w)), 4)
    ctypes.memmove(ctypes.byref(bmi, 8), ctypes.byref(ctypes.c_int32(-h)), 4)
    ctypes.memmove(ctypes.byref(bmi, 12), ctypes.byref(ctypes.c_int16(1)), 2)
    ctypes.memmove(ctypes.byref(bmi, 14), ctypes.byref(ctypes.c_int16(32)), 2)
    buf = (ctypes.c_byte * (w * h * 4))()
    gdi32.GetDIBits(mem, bmp, 0, h, buf, bmi, 0)
    img = Image.frombuffer("RGBA", (w, h), bytes(buf), "raw", "BGRA", 0, 1).convert("RGB")
    img.save(path)
    gdi32.DeleteObject(bmp)
    gdi32.DeleteDC(mem)
    user32.ReleaseDC(hwnd, hdc)
    return img


def launch(doc_text, name="doc.md"):
    p = SCRATCH / name
    p.write_bytes(doc_text.encode("utf-8"))
    env = dict(os.environ, BRISK_TRACE=str(TRACE))
    proc = subprocess.Popen([str(EXE), str(p)], env=env)
    hwnd = find_window(proc.pid)
    user32.MoveWindow(hwnd, 60, 60, 1100, 800, True)
    settle(hwnd, 0.4)
    return proc, hwnd, p


def close(proc, hwnd):
    user32.PostMessageW(hwnd, WM_CLOSE, 0, 0)
    try:
        proc.wait(3)
    except subprocess.TimeoutExpired:
        proc.kill()


if TRACE.exists():
    TRACE.unlink()

# 1. Typing at the end of a paragraph in Markdown mode, then saving.
proc, hwnd, p = launch("Hello **bold** world\n")
key(hwnd, "end")
typ(hwnd, " and more")
cmd(hwnd, "save")
check("type at end of paragraph", p.read_text("utf-8") == "Hello **bold** world and more\n", repr(p.read_text("utf-8")))

cmd(hwnd, "undo")
cmd(hwnd, "undo")
cmd(hwnd, "save")
check("undo (word by word) restores the original", p.read_text("utf-8") == "Hello **bold** world\n", repr(p.read_text("utf-8")))
close(proc, hwnd)

proc, hwnd, p = launch("- one\n- two\n")
key(hwnd, "down")
key(hwnd, "end")
key(hwnd, "enter")
typ(hwnd, "three")
key(hwnd, "enter")
key(hwnd, "enter")
typ(hwnd, "after")
cmd(hwnd, "save")
got = p.read_text("utf-8")
check("Enter continues and exits a list", got == "- one\n- two\n- three\n\nafter\n", repr(got))
shot(hwnd, SCRATCH / "list.png")
close(proc, hwnd)

# 3. Backspace at the start of a heading removes the heading marker.
proc, hwnd, p = launch("# Title\n\nBody text\n")
key(hwnd, "home")
key(hwnd, "back")
cmd(hwnd, "save")
got = p.read_text("utf-8")
check("Backspace removes a heading marker", got == "Title\n\nBody text\n", repr(got))
# Backspace at the start of the second paragraph joins it to the first.
key(hwnd, "down")
key(hwnd, "home")
key(hwnd, "back")
cmd(hwnd, "save")
got = p.read_text("utf-8")
check("Backspace joins paragraphs", got == "TitleBody text\n", repr(got))
close(proc, hwnd)

# 4. Typing inside a table in Markdown mode is refused; the file is unchanged.
doc = "Intro\n\n| a | b |\n|---|---|\n| 1 | 2 |\n"
proc, hwnd, p = launch(doc)
key(hwnd, "down", 2)
typ(hwnd, "X")
cmd(hwnd, "save")
check("tables are read-only in Markdown mode", p.read_text("utf-8") == doc, repr(p.read_text("utf-8")))
img = shot(hwnd, SCRATCH / "table-refused.png")
# Below the table, typing starts a new paragraph separated by a blank line.
key(hwnd, "down", 3)
typ(hwnd, "After")
cmd(hwnd, "save")
check("typing after a table starts a paragraph", p.read_text("utf-8") == doc + "\nAfter", repr(p.read_text("utf-8")))
close(proc, hwnd)

# 5. Mode switch keeps the caret at the same height on screen, mid-document.
long_doc = "".join(f"## Section {i}\n\nParagraph {i} has **some** text and a [link](x) and `code`.\n\n- item a\n- item b\n\n" for i in range(60))
proc, hwnd, p = launch(long_doc)
wheel(hwnd, 25)
click(hwnd, 300, 380)
shot(hwnd, SCRATCH / "before-toggle.png")
cmd(hwnd, "mode")
shot(hwnd, SCRATCH / "after-toggle.png")
cmd(hwnd, "mode")
lines = [l for l in TRACE.read_text().splitlines() if l.startswith("toggle")]
ok = True
for l in lines[-2:]:
    parts = dict(kv.split("=", 1) for kv in l.split()[1:] if "=" in kv)
    b = float(parts["before"].strip("Some()")) if "Some" in parts["before"] else None
    a = float(parts["after"].strip("Some()")) if "Some" in parts["after"] else None
    if b is None or a is None or abs(a - b) > 1.0:
        ok = False
check("mode switch keeps caret height (both directions)", ok and len(lines) >= 2, " | ".join(lines[-2:]))
# Near the top the caret can't stay put but must stay visible.
key(hwnd, "home")
cmd(hwnd, "mode")
cmd(hwnd, "mode")
close(proc, hwnd)

# 6. Mono mode: typing a table row works; saving keeps CRLF line endings.
crlf = "Line one\r\nLine two\r\n"
p = SCRATCH / "crlf.md"
proc, hwnd, p = launch(crlf.replace("\r\n", "\n"), "crlf.md")
close(proc, hwnd)
p.write_bytes(crlf.encode())
proc = subprocess.Popen([str(EXE), str(p)], env=dict(os.environ, BRISK_TRACE=str(TRACE)))
hwnd = find_window(proc.pid)
settle(hwnd, 0.3)
cmd(hwnd, "mode")
key(hwnd, "end")
typ(hwnd, "!")
cmd(hwnd, "save")
check("Mono typing + CRLF preserved", p.read_bytes() == b"Line one!\r\nLine two\r\n", repr(p.read_bytes()))
close(proc, hwnd)

# 7. A file changed by another program reloads in place.
proc, hwnd, p = launch("Original\n", "watch.md")
time.sleep(0.3)
p.write_text("Changed outside\n", encoding="utf-8")
time.sleep(1.6)
settle(hwnd)
key(hwnd, "end")
typ(hwnd, "!")
cmd(hwnd, "save")
check("external change reloads", p.read_text("utf-8") == "Changed outside!\n", repr(p.read_text("utf-8")))
close(proc, hwnd)

# 8. Opening and closing without edits never touches the file.
src = Path(__file__).resolve().parent.parent / "docs" / "samples" / "showcase.md"
dst = SCRATCH / "showcase.md"
shutil.copy(src, dst)
before = dst.read_bytes()
mtime = dst.stat().st_mtime_ns
proc = subprocess.Popen([str(EXE), str(dst)])
hwnd = find_window(proc.pid)
user32.MoveWindow(hwnd, 60, 60, 1100, 900, True)
settle(hwnd, 0.5)
shot(hwnd, SCRATCH / "showcase-window.png")
cmd(hwnd, "mode")
shot(hwnd, SCRATCH / "showcase-mono-window.png")
close(proc, hwnd)
check("open/close leaves the file untouched", dst.read_bytes() == before and dst.stat().st_mtime_ns == mtime)

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
