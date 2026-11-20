# BriskMarkdown Signpost

## Purpose

BriskMarkdown is a fast, lightweight Markdown notepad for Windows: a single native executable
that opens `.md` files instantly, renders them in a chosen style, lets you edit prose in place,
and switches to the plain source with `Ctrl+/`. It is open source (MIT) and published on GitHub.

This file is the durable first-door router. Update it only when purpose, authority, safety,
topology, or implemented ownership changes.

## Sources of truth

| Question | Authority |
|---|---|
| Goals, promises, non-goals, architecture | `docs/design.md` |
| Theme format (selectors, properties, palette) | `docs/themes.md`, `src/theme.rs`, `themes/` |
| Public description, install, shortcuts | `README.md` |
| Checked-out behaviour | Code and tests (`cargo test`, `--check`, `tools/ui_test.py`) |
| Project profile and capabilities | `project-foundation.toml` |
| Historical rationale | Git history |

## Safety invariants

- The user's file is never rewritten: saving writes the buffer exactly, keeping encoding and
  line endings; opening without editing never writes.
- Startup stays under 100 ms to the first frame for ordinary files.
- The repository is public: no workspace-private material, people, hosts or credentials in it.

## Implemented concerns

- **Rendering and layout:** `src/view.rs` (DirectWrite layout, Direct2D painting, lazy layout,
  hit testing), `src/gfx.rs`.
- **Markdown model:** `src/markdown.rs` (pulldown-cmark → blocks with a display↔source map),
  `src/highlight.rs`, `src/latex.rs`.
- **Editing and files:** `src/editor.rs` (buffer, undo, encodings, smart Enter, formatting).
- **Window and commands:** `src/app.rs`, `src/platform.rs` (menus, dark mode, clipboard,
  settings, file association), `src/print.rs`.
- **Verification tools:** `src/export.rs` (`--png`, `--check`), `tools/ui_test.py`,
  `tools/readme_images.py`.

## Suggested concerns

These are candidates, not commitments:

- **Typeset math and Mermaid diagrams:** both are shown readably today (Unicode math, diagram
  source). Real rendering needs a small layout engine for each.
- **Accessibility (UI Automation text pattern)** so screen readers can read the document.
- **Code signing** of release builds, to avoid the SmartScreen warning. This costs money, so the
  owner decides.

## Repository and topology conventions

- Canonical source: the maintainer's workspace checkout (`T:/Projects/BriskMarkdown`);
  remote: `github.com/grays42/BriskMarkdown` (public, authorized by the owner 2026-11-19).
- Build output goes to a local disk via `CARGO_TARGET_DIR` (for example
  `F:\Scratch\BriskMarkdown\target` on the build server); nothing generated is committed except
  `docs/images/` and `assets/icon.*`.
- Releases are GitHub releases with `BriskMarkdown.exe` attached.
- `AGENTS.md` is written for public contributors; the maintainer's workspace rules are reached
  through its "Maintainer's workspace" pointer, not copied in (a deliberate, documented
  deviation from the workspace template, because this repository is public).

## Task routing

- Changing how Markdown is parsed or mapped: `src/markdown.rs`, then run `--check` on a large
  corpus.
- Changing the look: `themes/*.css` and `docs/themes.md`; new hooks in `src/theme.rs`.
- Changing input, editing or the window: `src/app.rs`, `src/editor.rs`; run `tools/ui_test.py`.
- Releasing: bump `Cargo.toml` version, build, test, tag `vX.Y.Z`, create a GitHub release with
  the exe.
