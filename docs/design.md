# Design

## What BriskMarkdown is for

A Markdown notepad: double-click a `.md` file and it is on screen, styled, before you notice a
wait. Most Markdown today is written by AI tools, so the reading view has to handle everything
they produce. Editing is direct but deliberately modest in the styled view; anything structural
happens in the plain-text view, one keystroke away.

### Goals, in priority order

1. **Opens instantly.** One small native executable, no runtime to install, no browser engine.
   Target: the styled document is on screen within 100 ms of launch. Measured: about 50 ms on an
   idle machine and 75 ms on a busy desktop, for files from 2 KB to 290 KB.
2. **Reads beautifully.** A calm, considered page by default, with alternative themes and a CSS
   theme format for making your own.
3. **Two modes, one keystroke apart.** *Markdown mode* (the default) shows the document styled,
   with the syntax hidden, and lets you type prose, format with shortcuts, continue lists and
   quotes. *Mono mode* shows the exact source text in a monospace font for everything else
   (tables, code fences, math, front matter). `Ctrl+/` switches, keeping the caret at the same
   height on screen unless the top or bottom of the document makes that impossible.
4. **A notepad's menus.** New, Open, Save, Find, Replace, Go To, Print, Zoom, Word Wrap, Status
   Bar. Nothing more.
5. **One file, set as the default.** `BriskMarkdown.exe` is the whole program. It registers
   itself for `.md` files (per user) so Windows can make it the default.

### Promises

- **Your file is never rewritten.** Both modes are views of the same source text. Saving writes
  exactly what is in the buffer, in the file's original encoding and line endings. Opening and
  closing a file without typing never touches it.
- **No surprises.** No network access, no telemetry, no auto-update, no plugins, no session
  restore. Each file opens in its own window.
- **It follows the file.** When another program changes the open file and you have no unsaved
  edits, the window reloads it in place. If you do have edits, the status bar tells you instead.

### Non-goals

- A WYSIWYG word processor. In Markdown mode, structure (tables, fences, math) is edited in Mono
  mode.
- Remote images, embedded web content, scripts. Images show as their alt text.
- Tabs, sidebars, file trees, projects.

## Architecture

```
main.rs        command line: open a file, --register, --png, --check, --bench
app.rs         the window: input, commands, menus, status bar, scrollbar, find, files
view.rs        layout (DirectWrite), painting (Direct2D), hit testing; lazy, per block
markdown.rs    Markdown → display blocks with a display↔source map (pulldown-cmark)
editor.rs      the buffer: undo/redo, encodings, smart Enter, formatting toggles
theme.rs       the CSS subset → resolved styles
highlight.rs   small hand-written code highlighter (no regex, no grammar files)
latex.rs       LaTeX math → readable Unicode
platform.rs    dark title bar and menus, clipboard, input box, settings, file association
print.rs       printing
export.rs      off-screen rendering to PNG, corpus checks
```

### The display ↔ source map

The source text is the only truth. `markdown::build` parses it with pulldown-cmark and produces
blocks (paragraph, heading, code, table, ...). Each block carries its display text (UTF-16, as
DirectWrite wants it) and a list of segments mapping display ranges back to source byte ranges:

- an **exact** segment is display text identical to the source (prose, code): the caret moves
  through it character by character;
- an **atomic** segment is anything synthesized (a decoded entity, converted math, the thin
  space that stands in for an inline-code backtick): the caret steps over it.

Markup that has no display (`**`, `[`, `](url)`, `#`) simply falls between segments. Typing at a
boundary goes to the end of the segment on the left, so typing after a bold word stays bold.

Because both modes are built from the same source text, switching modes only rebuilds the view.
The caret is a source offset, so it maps exactly into the other mode.

### Speed

- **Software Direct2D rendering.** Creating a hardware device costs about 25 ms at startup. For
  text, software rendering is faster to start and plenty fast to draw. Set `BRISK_GPU=1` to
  compare.
- **Draw first, activate second.** The window is shown without activating it, the first frame
  is drawn, and only then does it take focus: activation (focus, input method, other programs'
  hooks) can cost tens of milliseconds on a busy desktop.
- **Delay-loaded DLLs.** Shell, dialog, COM, input-method and registry DLLs load on first use.
- **Lazy layout.** Only the blocks on screen are laid out before the first frame; the rest are
  laid out in idle time, while the view stays anchored.
- **Layout reuse.** After an edit, every block whose display text and style are unchanged keeps
  its layout. Re-parsing a 300 KB file takes about 3 ms.
- **No startup I/O beyond the file itself** and one small settings file. Theme files are read
  only when chosen.

### Verification

- `cargo test`: unit tests for the map, the editor, themes, highlighting and math.
- `BriskMarkdown.exe --check <folder> <report.txt> --layout` walks every Markdown file under a
  folder and checks the display↔source map of every block (in both modes, and with the caret
  placed throughout the file). It also lays every block out and reports timings.
- `BriskMarkdown.exe --png <file.md> <out.png> [--width 900] [--theme Light] [--mono]` renders a
  document off screen, for screenshots and visual review.
- `BriskMarkdown.exe --bench <log> <file.md>` opens the window, writes the time to first frame
  to the log, and exits.
