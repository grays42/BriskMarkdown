# BriskMarkdown Agent Instructions

## Hydration

Read `SIGNPOST.md` first, then `docs/design.md` (goals, promises, architecture). This repository
is public: everything committed here is visible to anyone.

## Maintainer's workspace

When this checkout lives in the maintainer's project workspace (`T:/Projects/BriskMarkdown`),
the workspace rules apply as well: read `T:/Projects/AGENTS.md` and the people, privacy and
engagement standards it routes to (`T:/Projects/_FOUNDATION/`). They govern commits, privacy and
the hard stops there. Nothing about that workspace (people, hosts, paths beyond this pointer,
decisions) goes into this public repository; it belongs in the workspace's own records.

## Commands

```powershell
cargo build --release                      # target\release\BriskMarkdown.exe
cargo test --release                       # unit tests
target\release\BriskMarkdown.exe --check <folder of .md files> <report.txt> --layout
target\release\BriskMarkdown.exe --png docs\samples\showcase.md out.png --width 1000 [--theme Light] [--mono]
target\release\BriskMarkdown.exe --bench <log.txt> <file.md>       # time to first frame
python tools\ui_test.py target\release\BriskMarkdown.exe <scratch dir>   # drives the real window
python tools\readme_images.py target\release\BriskMarkdown.exe         # refresh docs\images
```

In the maintainer's workspace, build with `CARGO_TARGET_DIR` on a local disk (the source lives on
a network share; building there is slow).

## Rules that keep the product what it is

- **Startup is the product.** Nothing may add work before the first frame that isn't needed to
  draw it. Measure with `--bench` before and after any change to startup, parsing or layout.
  Budget: under 100 ms to first frame for any ordinary file.
- **Never rewrite the user's file.** Both modes are views of `Doc::text`. Don't normalize,
  reformat or "fix" Markdown on load or save. Encoding and line endings round-trip.
- **One executable.** No runtime downloads, no DLLs beside the exe, no installer. Themes are the
  only optional files.
- **Markdown-mode editing stays modest.** Prose typing, formatting shortcuts, list and quote
  continuation. Structural editing (tables, fences, math) is Mono mode's job; don't grow a
  WYSIWYG table editor.
- **The display↔source map is the core invariant.** Any change to `markdown.rs` must keep
  `--check` clean over a large corpus (thousands of real files), in both modes.
- **Themes are CSS-shaped.** New styling hooks are added as selectors and properties in
  `theme.rs`, documented in `docs/themes.md`, and used by the built-in themes in `themes/`.

## Commits

Commit finished, tested work as you complete it: behaviour, tests and docs together. Stage your
own paths explicitly (`git add -- <path>`) and leave paths you didn't touch as you found them.
Commit messages describe the change for a public reader.

## Completion

- `cargo test --release` passes, and `--check` is clean when parsing or layout changed.
- `tools/ui_test.py` passes when input handling, editing or the window changed.
- `--bench` stays under budget when startup, parsing or layout changed.
- README and `docs/` describe what the code now does.
