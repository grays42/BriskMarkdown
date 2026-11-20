//! BriskMarkdown: a fast, lightweight Markdown notepad for Windows.
#![windows_subsystem = "windows"]

mod app;
mod editor;
mod export;
mod gfx;
mod highlight;
mod latex;
mod markdown;
mod platform;
mod print;
mod theme;
mod view;

use std::path::PathBuf;

pub const SHORTCUTS: &str = "\
Mode
  Ctrl+/            Switch between Markdown and Mono mode

Files
  Ctrl+N / Ctrl+Shift+N    New / New window
  Ctrl+O, Ctrl+S, Ctrl+Shift+S    Open, Save, Save as
  Ctrl+R            Reload from disk
  Ctrl+P            Print
  Ctrl+W            Close

Formatting (Markdown mode)
  Ctrl+B / Ctrl+I   Bold / Italic
  Alt+Shift+5       Strikethrough
  Ctrl+`            Inline code
  Ctrl+K            Link (uses a URL on the clipboard)
  Ctrl+1 ... Ctrl+6 Heading level (again to remove)
  Tab / Shift+Tab   Indent / outdent a list item
  Enter             New paragraph; continues lists and quotes
  Shift+Enter       Line break inside a paragraph

Editing
  Ctrl+Z / Ctrl+Y   Undo / Redo
  Ctrl+C            Copy as Markdown
  Ctrl+Shift+C      Copy as plain text
  Ctrl+F, F3, Shift+F3, Ctrl+H   Find, next, previous, replace
  Ctrl+G            Go to line
  F5                Insert time and date

View
  Ctrl+Plus / Ctrl+Minus / Ctrl+0   Zoom in / out / reset
  Ctrl+click        Open a link
";

pub const THEME_EXAMPLE: &str = include_str!("../themes/example.css");

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut file: Option<PathBuf> = None;
    let mut bench: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--register" => {
                let ok = platform::register();
                std::process::exit(if ok { 0 } else { 1 });
            }
            "--unregister" => {
                platform::unregister();
                std::process::exit(0);
            }
            "--bench" => {
                i += 1;
                bench = args.get(i).map(PathBuf::from);
            }
            "--print-test" => {
                let (Some(input), Some(out)) = (args.get(i + 1), args.get(i + 2)) else { std::process::exit(2) };
                let text = std::fs::read(input).map(|b| editor::decode(&b).0.replace("\r\n", "\n")).unwrap_or_default();
                let code = match print::print_test(&text, std::path::Path::new(out)) {
                    Ok(()) => 0,
                    Err(e) => {
                        let _ = std::fs::write(format!("{out}.error.txt"), format!("{e:?}"));
                        4
                    }
                };
                std::process::exit(code);
            }
            "--png" | "--check" => {
                let code = export::cli(&args);
                std::process::exit(code);
            }
            _ => {
                if file.is_none() && !a.starts_with("--") {
                    file = Some(PathBuf::from(a));
                }
            }
        }
        i += 1;
    }
    let file = file.map(|p| std::path::absolute(&p).unwrap_or(p));
    let _ = app::run(app::Opts { file, bench });
}
