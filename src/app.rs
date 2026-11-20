//! The main window: input, commands, menus, painting the status bar and scrollbar, files.

use crate::editor::{self, Doc, EditKind};
use crate::gfx::*;
use crate::markdown::{self, Inline, Kind};
use crate::platform::{self, *};
use crate::theme::{Color, El, Theme};
use crate::view::{Loc, Mode, View};
use std::path::{Path, PathBuf};
use std::time::Instant;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::Ime::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

// Command ids
const CMD_NEW: u16 = 101;
const CMD_NEW_WINDOW: u16 = 102;
const CMD_OPEN: u16 = 103;
const CMD_SAVE: u16 = 104;
const CMD_SAVE_AS: u16 = 105;
const CMD_RELOAD: u16 = 106;
const CMD_REVEAL: u16 = 107;
const CMD_PRINT: u16 = 108;
const CMD_EXIT: u16 = 109;
const CMD_RECENT0: u16 = 150;
const CMD_UNDO: u16 = 201;
const CMD_REDO: u16 = 202;
const CMD_CUT: u16 = 203;
const CMD_COPY: u16 = 204;
const CMD_PASTE: u16 = 205;
const CMD_DELETE: u16 = 206;
const CMD_SELECT_ALL: u16 = 207;
const CMD_FIND: u16 = 208;
const CMD_FIND_NEXT: u16 = 209;
const CMD_FIND_PREV: u16 = 210;
const CMD_REPLACE: u16 = 211;
const CMD_GOTO: u16 = 212;
const CMD_COPY_PLAIN: u16 = 213;
const CMD_TIME_DATE: u16 = 214;
const CMD_BOLD: u16 = 301;
const CMD_ITALIC: u16 = 302;
const CMD_CODE: u16 = 303;
const CMD_STRIKE: u16 = 304;
const CMD_LINK: u16 = 305;
const CMD_H0: u16 = 310;
const CMD_INDENT: u16 = 320;
const CMD_OUTDENT: u16 = 321;
const CMD_MODE: u16 = 401;
const CMD_ZOOM_IN: u16 = 402;
const CMD_ZOOM_OUT: u16 = 403;
const CMD_ZOOM_RESET: u16 = 404;
const CMD_WRAP: u16 = 405;
const CMD_STATUS: u16 = 406;
const CMD_THEME0: u16 = 410;
const CMD_THEMES_FOLDER: u16 = 440;
const CMD_THEME_RELOAD: u16 = 441;
const CMD_SHORTCUTS: u16 = 501;
const CMD_DEFAULT_APP: u16 = 502;
const CMD_ABOUT: u16 = 503;
const CMD_WEBSITE: u16 = 504;

const TIMER_CARET: usize = 1;
const TIMER_FILE: usize = 2;
const TIMER_STATUS: usize = 3;
const WM_APP_IDLE: u32 = WM_APP + 1;
const WM_APP_SNAPSHOT: u32 = WM_APP + 2;

const STATUS_H: f32 = 26.0;
const SCROLLBAR_W: f32 = 12.0;

pub const WEBSITE: &str = "https://github.com/grays42/BriskMarkdown";

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    Select,
    SelectWords(usize, usize),
    Scroll(f32),
}

pub struct App {
    hwnd: HWND,
    gfx: Gfx,
    hrt: Option<ID2D1HwndRenderTarget>,
    theme: Theme,
    doc: Doc,
    view: View,
    scale: f32,
    menu: HMENU,
    view_menu: HMENU,
    file_menu: HMENU,
    accel: HACCEL,
    sticky_x: Option<f32>,
    caret_on: bool,
    drag: Drag,
    status_msg: Option<(String, Instant)>,
    finds: Vec<(usize, usize)>,
    find_cur: Option<usize>,
    find_text: String,
    find_case: bool,
    find_word: bool,
    find_hwnd: HWND,
    fr: Box<FINDREPLACEW>,
    fr_find: Box<[u16; 256]>,
    fr_repl: Box<[u16; 256]>,
    find_msg: u32,
    settings: Settings,
    bench: Option<PathBuf>,
    painted: bool,
    clicks: (Instant, i32, i32, u32),
    disk_note: bool,
    high_surrogate: Option<u16>,
    user_themes: Vec<String>,
    scroll_hover: bool,
    last_blank_line: bool,
}

static mut APP: *mut App = std::ptr::null_mut();

fn app() -> Option<&'static mut App> {
    unsafe {
        if APP.is_null() {
            None
        } else {
            Some(&mut *APP)
        }
    }
}

pub fn load_theme(name: &str) -> Theme {
    if crate::theme::BUILTIN.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
        return Theme::builtin(name);
    }
    let p = themes_dir().join(format!("{name}.css"));
    match std::fs::read_to_string(&p) {
        Ok(css) => Theme::from_css(name, &css),
        Err(_) => Theme::builtin("Dark"),
    }
}

fn user_theme_names() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(themes_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x.eq_ignore_ascii_case("css")).unwrap_or(false) {
                if let Some(s) = p.file_stem() {
                    let s = s.to_string_lossy().to_string();
                    if !crate::theme::BUILTIN.iter().any(|(n, _)| n.eq_ignore_ascii_case(&s)) {
                        v.push(s);
                    }
                }
            }
        }
    }
    v.sort_by_key(|s| s.to_lowercase());
    v
}

fn ms_since_start() -> f64 {
    use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
    use windows::Win32::System::Threading::*;
    unsafe {
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let _ = GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
        let now = GetSystemTimePreciseAsFileTime();
        let a = ((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64;
        let b = ((now.dwHighDateTime as u64) << 32) | now.dwLowDateTime as u64;
        b.saturating_sub(a) as f64 / 10_000.0
    }
}

fn menu_text(s: &str) -> Vec<u16> {
    wstr(s)
}

fn append(m: HMENU, id: u16, text: &str) {
    let w = menu_text(text);
    unsafe {
        let _ = AppendMenuW(m, MF_STRING, id as usize, PCWSTR(w.as_ptr()));
    }
}

fn sep(m: HMENU) {
    unsafe {
        let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
    }
}

fn submenu(parent: HMENU, sub: HMENU, text: &str) {
    let w = menu_text(text);
    unsafe {
        let _ = AppendMenuW(parent, MF_POPUP, sub.0 as usize, PCWSTR(w.as_ptr()));
    }
}

fn build_menus() -> (HMENU, HMENU, HMENU) {
    unsafe {
        let bar = CreateMenu().unwrap();
        let file = CreatePopupMenu().unwrap();
        append(file, CMD_NEW, "&New\tCtrl+N");
        append(file, CMD_NEW_WINDOW, "New &Window\tCtrl+Shift+N");
        append(file, CMD_OPEN, "&Open...\tCtrl+O");
        let recent = CreatePopupMenu().unwrap();
        submenu(file, recent, "Open &Recent");
        sep(file);
        append(file, CMD_SAVE, "&Save\tCtrl+S");
        append(file, CMD_SAVE_AS, "Save &As...\tCtrl+Shift+S");
        append(file, CMD_RELOAD, "Re&load from Disk\tCtrl+R");
        append(file, CMD_REVEAL, "Show in &Folder");
        sep(file);
        append(file, CMD_PRINT, "&Print...\tCtrl+P");
        sep(file);
        append(file, CMD_EXIT, "E&xit\tCtrl+W");
        submenu(bar, file, "&File");

        let edit = CreatePopupMenu().unwrap();
        append(edit, CMD_UNDO, "&Undo\tCtrl+Z");
        append(edit, CMD_REDO, "&Redo\tCtrl+Y");
        sep(edit);
        append(edit, CMD_CUT, "Cu&t\tCtrl+X");
        append(edit, CMD_COPY, "&Copy\tCtrl+C");
        append(edit, CMD_COPY_PLAIN, "Copy as Plain &Text\tCtrl+Shift+C");
        append(edit, CMD_PASTE, "&Paste\tCtrl+V");
        append(edit, CMD_DELETE, "De&lete\tDel");
        sep(edit);
        append(edit, CMD_FIND, "&Find...\tCtrl+F");
        append(edit, CMD_FIND_NEXT, "Find &Next\tF3");
        append(edit, CMD_FIND_PREV, "Find Pre&vious\tShift+F3");
        append(edit, CMD_REPLACE, "R&eplace...\tCtrl+H");
        append(edit, CMD_GOTO, "&Go To Line...\tCtrl+G");
        sep(edit);
        append(edit, CMD_SELECT_ALL, "Select &All\tCtrl+A");
        append(edit, CMD_TIME_DATE, "Time/&Date\tF5");
        submenu(bar, edit, "&Edit");

        let fmt = CreatePopupMenu().unwrap();
        append(fmt, CMD_BOLD, "&Bold\tCtrl+B");
        append(fmt, CMD_ITALIC, "&Italic\tCtrl+I");
        append(fmt, CMD_STRIKE, "&Strikethrough\tAlt+Shift+5");
        append(fmt, CMD_CODE, "Inline &Code\tCtrl+`");
        append(fmt, CMD_LINK, "&Link...\tCtrl+K");
        sep(fmt);
        for n in 1..=6u16 {
            append(fmt, CMD_H0 + n, &format!("Heading &{n}\tCtrl+{n}"));
        }
        sep(fmt);
        append(fmt, CMD_INDENT, "Indent List Item\tTab");
        append(fmt, CMD_OUTDENT, "Outdent List Item\tShift+Tab");
        submenu(bar, fmt, "F&ormat");

        let view = CreatePopupMenu().unwrap();
        append(view, CMD_MODE, "&Mono Mode\tCtrl+/");
        sep(view);
        let themes = CreatePopupMenu().unwrap();
        submenu(view, themes, "&Theme");
        append(view, CMD_ZOOM_IN, "Zoom &In\tCtrl++");
        append(view, CMD_ZOOM_OUT, "Zoom &Out\tCtrl+-");
        append(view, CMD_ZOOM_RESET, "&Reset Zoom\tCtrl+0");
        sep(view);
        append(view, CMD_WRAP, "&Word Wrap (Mono)");
        append(view, CMD_STATUS, "&Status Bar");
        submenu(bar, view, "&View");

        let help = CreatePopupMenu().unwrap();
        append(help, CMD_SHORTCUTS, "&Keyboard Shortcuts\tF1");
        append(help, CMD_DEFAULT_APP, "Make BriskMarkdown the &Default for .md Files...");
        append(help, CMD_WEBSITE, "&Website");
        sep(help);
        append(help, CMD_ABOUT, "&About BriskMarkdown");
        submenu(bar, help, "&Help");

        let w = menu_text("Markdown");
        let _ = AppendMenuW(bar, MF_STRING | MF_RIGHTJUSTIFY, CMD_MODE as usize, PCWSTR(w.as_ptr()));
        (bar, file, view)
    }
}

fn build_accels() -> HACCEL {
    let c = FCONTROL.0 | FVIRTKEY.0;
    let cs = c | FSHIFT.0;
    let v = FVIRTKEY.0;
    let mut a: Vec<(u8, u16, u16)> = vec![
        (c, b'N' as u16, CMD_NEW),
        (cs, b'N' as u16, CMD_NEW_WINDOW),
        (c, b'O' as u16, CMD_OPEN),
        (c, b'S' as u16, CMD_SAVE),
        (cs, b'S' as u16, CMD_SAVE_AS),
        (c, b'R' as u16, CMD_RELOAD),
        (c, b'P' as u16, CMD_PRINT),
        (c, b'W' as u16, CMD_EXIT),
        (c, b'Z' as u16, CMD_UNDO),
        (cs, b'Z' as u16, CMD_REDO),
        (c, b'Y' as u16, CMD_REDO),
        (c, b'X' as u16, CMD_CUT),
        (c, b'C' as u16, CMD_COPY),
        (cs, b'C' as u16, CMD_COPY_PLAIN),
        (c, b'V' as u16, CMD_PASTE),
        (c, b'A' as u16, CMD_SELECT_ALL),
        (c, b'F' as u16, CMD_FIND),
        (v, VK_F3.0, CMD_FIND_NEXT),
        (v | FSHIFT.0, VK_F3.0, CMD_FIND_PREV),
        (c, b'H' as u16, CMD_REPLACE),
        (c, b'G' as u16, CMD_GOTO),
        (v, VK_F5.0, CMD_TIME_DATE),
        (c, b'B' as u16, CMD_BOLD),
        (c, b'I' as u16, CMD_ITALIC),
        (c, VK_OEM_3.0, CMD_CODE),
        (FALT.0 | FSHIFT.0 | FVIRTKEY.0, b'5' as u16, CMD_STRIKE),
        (c, b'K' as u16, CMD_LINK),
        (c, VK_OEM_2.0, CMD_MODE),
        (c, VK_OEM_PLUS.0, CMD_ZOOM_IN),
        (c, VK_ADD.0, CMD_ZOOM_IN),
        (c, VK_OEM_MINUS.0, CMD_ZOOM_OUT),
        (c, VK_SUBTRACT.0, CMD_ZOOM_OUT),
        (c, b'0' as u16, CMD_ZOOM_RESET),
        (c, VK_NUMPAD0.0, CMD_ZOOM_RESET),
        (v, VK_F1.0, CMD_SHORTCUTS),
    ];
    for n in 1..=6u16 {
        a.push((c, b'0' as u16 + n, CMD_H0 + n));
    }
    let accels: Vec<ACCEL> = a.into_iter().map(|(f, k, cmd)| ACCEL { fVirt: ACCEL_VIRT_FLAGS(f), key: k, cmd }).collect();
    unsafe { CreateAcceleratorTableW(&accels).unwrap_or_default() }
}

impl App {
    fn body_bg(&self) -> Color {
        let el = if self.view.mode == Mode::Mono { El::Mono } else { El::Body };
        self.theme.style(el).background.or(self.theme.style(El::Body).background).unwrap_or(Color::rgb(20, 22, 26))
    }

    fn menu_colors(&self) -> MenuColors {
        let mb = self.theme.style(El::Menubar);
        let bg = mb.background.unwrap_or(self.body_bg());
        let text = mb.color;
        let hot = bg.mix(&text, 0.14);
        let btn = self.theme.style(El::StatusButton);
        MenuColors { bg, text, hot, accent_bg: btn.background.unwrap_or(hot) }
    }

    fn apply_window_theme(&mut self) {
        let mc = self.menu_colors();
        style_title_bar(self.hwnd, self.theme.dark, &mc.bg, &mc.text);
        set_menu_dark(self.theme.dark);
        unsafe {
            let br = CreateSolidBrush(COLORREF(self.body_bg().colorref()));
            let old = SetClassLongPtrW(self.hwnd, GCLP_HBRBACKGROUND, br.0 as isize);
            if old != 0 {
                let _ = DeleteObject(HGDIOBJ(old as _));
            }
            let _ = DrawMenuBar(self.hwnd);
        }
    }

    fn title(&self) -> String {
        let name = self
            .doc
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".into());
        format!("{}{} - BriskMarkdown", if self.doc.dirty { "\u{25CF} " } else { "" }, name)
    }

    fn update_title(&self) {
        let w = wstr(&self.title());
        unsafe {
            let _ = SetWindowTextW(self.hwnd, PCWSTR(w.as_ptr()));
        }
    }

    fn update_mode_item(&self) {
        let label = if self.view.mode == Mode::Markdown { "Markdown" } else { "Mono" };
        let w = wstr(label);
        unsafe {
            let count = GetMenuItemCount(Some(self.menu));
            let _ = ModifyMenuW(
                self.menu,
                (count - 1) as u32,
                MF_BYPOSITION | MF_STRING | MF_RIGHTJUSTIFY,
                CMD_MODE as usize,
                PCWSTR(w.as_ptr()),
            );
            let _ = DrawMenuBar(self.hwnd);
        }
    }

    // ------------------------------------------------------------ geometry

    fn client_dips(&self) -> (f32, f32) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        ((rc.right - rc.left) as f32 / self.scale, (rc.bottom - rc.top) as f32 / self.scale)
    }

    fn status_h(&self) -> f32 {
        if self.settings.status {
            STATUS_H
        } else {
            0.0
        }
    }

    fn layout_viewport(&mut self) {
        let (w, h) = self.client_dips();
        let new_w = w;
        let new_h = (h - self.status_h()).max(10.0);
        let width_changed = (self.view.screen_w - new_w).abs() > 0.5;
        self.view.screen_w = new_w;
        self.view.screen_h = new_h;
        if width_changed {
            let anchor = self.top_anchor();
            self.view.invalidate(&self.theme);
            self.restore_anchor(anchor);
        }
    }

    /// Source offset at the top of the viewport and its distance from the top, to keep the
    /// reading position when the layout changes.
    fn top_anchor(&self) -> (usize, f32) {
        let i = self.view.block_at_y(self.view.scroll);
        match (self.view.blocks.get(i), self.view.geo.get(i)) {
            (Some(b), Some(g)) => (b.src.start, g.y - self.view.scroll),
            _ => (0, 0.0),
        }
    }

    fn restore_anchor(&mut self, (src, off): (usize, f32)) {
        let loc = self.view.src_to_loc(src);
        self.view.ensure_laid(loc.block, &self.gfx, &self.theme);
        for _ in 0..4 {
            if let Some(g) = self.view.geo.get(loc.block) {
                self.view.scroll = g.y - off;
            }
            self.view.clamp_scroll();
            self.view.prepare(&self.gfx, &self.theme);
            if let Some(g) = self.view.geo.get(loc.block) {
                if ((g.y - off) - self.view.scroll).abs() < 0.5 || self.view.scroll <= 0.0 || self.view.scroll >= self.view.max_scroll() {
                    break;
                }
            }
        }
    }

    // ------------------------------------------------------------ model

    fn rebuild(&mut self) {
        let blocks = match self.view.mode {
            Mode::Markdown => markdown::build(&self.doc.text, Some(self.doc.head)),
            Mode::Mono => markdown::build_mono(&self.doc.text),
        };
        self.view.set_blocks(blocks, &self.theme);
        self.last_blank_line = self.on_blank_line(self.doc.head);
        self.schedule_idle();
    }

    fn on_blank_line(&self, pos: usize) -> bool {
        let t = &self.doc.text;
        let ls = editor::line_start(t, pos);
        let le = editor::line_end(t, pos);
        t[ls..le].trim().is_empty()
    }

    fn schedule_idle(&self) {
        if self.view.remaining > 0 {
            unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_APP_IDLE, WPARAM(0), LPARAM(0));
            }
        }
    }

    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn caret_loc(&self) -> Loc {
        self.view.src_to_loc(self.doc.head)
    }

    fn caret_rect(&mut self) -> Option<(f32, f32, f32)> {
        let loc = self.caret_loc();
        self.view.ensure_laid(loc.block, &self.gfx, &self.theme);
        self.view.caret_rect(loc)
    }

    fn ensure_caret_visible(&mut self) {
        self.view.prepare(&self.gfx, &self.theme);
        if let Some((_, y, h)) = self.caret_rect() {
            let vh = self.view.view_h();
            let margin = (h * 1.5).min(vh / 4.0);
            if y < self.view.scroll + margin * 0.5 {
                self.view.scroll = y - margin;
            } else if y + h > self.view.scroll + vh - margin * 0.5 {
                self.view.scroll = y + h - vh + margin;
            }
            self.view.clamp_scroll();
            self.view.prepare(&self.gfx, &self.theme);
        }
    }

    /// After the caret moved without editing: rebuild only if a blank-line placeholder must
    /// appear or disappear.
    fn caret_moved(&mut self, keep_sticky: bool) {
        if !keep_sticky {
            self.sticky_x = None;
        }
        if self.view.mode == Mode::Markdown {
            let blank = self.on_blank_line(self.doc.head);
            if blank || self.last_blank_line {
                self.rebuild();
            }
        }
        self.caret_on = true;
        self.restart_blink();
        self.ensure_caret_visible();
        self.invalidate();
    }

    fn edited(&mut self) {
        self.sticky_x = None;
        self.finds.clear();
        self.find_cur = None;
        self.rebuild();
        self.caret_on = true;
        self.restart_blink();
        self.ensure_caret_visible();
        self.update_title();
        self.invalidate();
    }

    fn restart_blink(&self) {
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_CARET, GetCaretBlinkTime(), None);
        }
    }

    fn flash(&mut self, msg: &str) {
        self.status_msg = Some((msg.to_string(), Instant::now()));
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_STATUS, 4500, None);
        }
        self.invalidate();
    }

    /// The block holding the caret, if typing is not allowed there in Markdown mode.
    fn read_only_here(&self) -> Option<&'static str> {
        if self.view.mode == Mode::Mono {
            return None;
        }
        let loc = self.caret_loc();
        let b = self.view.blocks.get(loc.block)?;
        if b.editable() {
            return None;
        }
        Some(match b.kind {
            Kind::Table { .. } => "Tables are edited in Mono mode. Press Ctrl+/ to switch.",
            Kind::MathBlock => "Math is edited in Mono mode. Press Ctrl+/ to switch.",
            Kind::FrontMatter => "Front matter is edited in Mono mode. Press Ctrl+/ to switch.",
            Kind::Html => "HTML is edited in Mono mode. Press Ctrl+/ to switch.",
            _ => "This part is edited in Mono mode. Press Ctrl+/ to switch.",
        })
    }

    fn in_code_block(&self) -> bool {
        if self.view.mode == Mode::Mono {
            // Inside a fence in the source?
            let before = &self.doc.text[..editor::line_start(&self.doc.text, self.doc.head)];
            return before.lines().filter(|l| l.trim_start().starts_with("```") || l.trim_start().starts_with("~~~")).count() % 2 == 1;
        }
        let loc = self.caret_loc();
        matches!(self.view.blocks.get(loc.block).map(|b| &b.kind), Some(Kind::Code { .. } | Kind::Diagram { .. }))
    }

    // ------------------------------------------------------------ editing

    fn insert_text(&mut self, text: &str, kind: EditKind) {
        if !self.doc.has_sel() {
            if let Some(msg) = self.read_only_here() {
                self.flash(msg);
                return;
            }
        }
        let sel = self.doc.sel();
        // Typing on an empty line between blocks starts a new paragraph: pad it with blank lines
        // so Markdown doesn't fold it into the paragraph, list or table next to it.
        if self.view.mode == Mode::Markdown && sel.0 == sel.1 && !self.in_code_block() {
            let loc = self.caret_loc();
            if self.view.blocks.get(loc.block).map(|b| b.virtual_).unwrap_or(false) {
                let t = &self.doc.text;
                let ls = editor::line_start(t, sel.0);
                let le = editor::line_end(t, sel.0);
                let prev_line_blank = ls == 0 || {
                    let pls = editor::line_start(t, ls - 1);
                    t[pls..ls - 1].trim().is_empty()
                };
                let next_line_blank = le >= t.len() || {
                    let nle = editor::line_end(t, le + 1);
                    t[le + 1..nle].trim().is_empty()
                };
                let pre = if prev_line_blank { "" } else { "\n" };
                let post = if next_line_blank { "" } else { "\n" };
                if !pre.is_empty() || !post.is_empty() {
                    let caret = sel.0 + pre.len() + text.len();
                    self.doc.begin_group();
                    self.doc.replace(sel, &format!("{pre}{text}{post}"), EditKind::Other);
                    self.doc.set_sel(caret, caret);
                    self.edited();
                    return;
                }
            }
        }
        self.doc.replace(sel, text, kind);
        self.edited();
    }

    fn apply(&mut self, c: editor::Change, sel_after: Option<(usize, usize)>) {
        self.doc.begin_group();
        self.doc.replace(c.range, &c.text, EditKind::Other);
        match sel_after {
            Some((a, b)) => self.doc.set_sel(a, b),
            None => self.doc.set_sel(c.caret, c.caret),
        }
        self.edited();
    }

    fn backspace(&mut self, word: bool) {
        if self.doc.has_sel() {
            let sel = self.doc.sel();
            self.doc.replace(sel, "", EditKind::Delete);
            self.edited();
            return;
        }
        let head = self.doc.head;
        if head == 0 {
            return;
        }
        if self.view.mode == Mode::Markdown {
            let loc = self.caret_loc();
            if let Some(msg) = self.read_only_here() {
                self.flash(msg);
                return;
            }
            if loc.d == 0 && loc.para == 0 {
                // At the start of a block: remove its prefix (heading, list marker, quote) or
                // join it with the block above.
                let ls = editor::line_start(&self.doc.text, head);
                let line = &self.doc.text[ls..editor::line_end(&self.doc.text, head)];
                let p = editor::parse_prefix(line);
                let hashes = line.bytes().take_while(|&c| c == b'#').count();
                if hashes > 0
                    && hashes <= 6
                    && line[hashes..].starts_with(' ')
                    && head <= ls + hashes + 1 + line[hashes + 1..].len() - line[hashes + 1..].trim_start().len()
                {
                    let c = editor::set_heading(&self.doc.text, head, hashes);
                    self.apply(c, None);
                    return;
                }
                if p.len > 0 && head >= ls + p.len && head <= ls + p.len + 1 {
                    // Strip the innermost marker: list marker first, then one quote level.
                    let prefix = &line[..p.len];
                    let new_prefix = if p.has_list {
                        let quote_part: String = {
                            let mut q = String::new();
                            for ch in prefix.chars() {
                                if ch == '>' || (ch == ' ' && !q.is_empty() && q.ends_with('>')) {
                                    q.push(ch);
                                } else if ch == ' ' && q.is_empty() {
                                } else if ch != '>' && ch != ' ' {
                                    break;
                                }
                            }
                            q
                        };
                        quote_part
                    } else {
                        // drop one '>' level
                        let mut s = prefix.to_string();
                        if let Some(i) = s.rfind('>') {
                            s.replace_range(i..(i + 1 + if s[i + 1..].starts_with(' ') { 1 } else { 0 }).min(s.len()), "");
                        }
                        s
                    };
                    // Keep it from becoming a lazy continuation of the item above.
                    let needs_blank = ls > 0 && !self.doc.text[..ls - 1].ends_with('\n') && !self.doc.text[..ls].ends_with("\n\n");
                    let text = if needs_blank && p.has_list { format!("\n{new_prefix}") } else { new_prefix.clone() };
                    let caret = ls + text.len();
                    self.apply(editor::Change { range: (ls, ls + p.len), text, caret }, None);
                    return;
                }
                // Join with the previous block.
                if loc.block > 0 {
                    let mut pb = loc.block - 1;
                    while pb > 0 && self.view.blocks[pb].virtual_ {
                        pb -= 1;
                    }
                    let prev = &self.view.blocks[pb];
                    if !prev.editable() {
                        self.doc.set_sel(prev.src.end, prev.src.end);
                        self.caret_moved(false);
                        return;
                    }
                    let end = prev.para.src_span().map(|(_, e)| e).unwrap_or(prev.src.end);
                    if end < head {
                        // remove the whitespace between, and this line's own prefix
                        let gap = &self.doc.text[end..head];
                        if gap.trim().is_empty() || gap.trim_matches(|c: char| c.is_whitespace() || c == '>').is_empty() {
                            self.doc.replace((end, head), "", EditKind::Delete);
                            self.edited();
                            return;
                        }
                    }
                }
            }
            // Ordinary: delete the previous display character (or word).
            let para = self.view.blocks[loc.block].para_at(loc.para);
            let text = String::from_utf16_lossy(&para.text);
            let d = if word { word_left_d(&text, loc.d) } else { grapheme_left_d(&text, loc.d) };
            let a = para.d2s(d, false);
            let a = if a >= head { editor::clamp_boundary(&self.doc.text, head - 1) } else { a };
            self.doc.replace((a, head), "", EditKind::Delete);
            self.edited();
            return;
        }
        // Mono: plain text deletion.
        let a = if word { word_left_src(&self.doc.text, head) } else { prev_char(&self.doc.text, head) };
        self.doc.replace((a, head), "", EditKind::Delete);
        self.edited();
    }

    fn delete_forward(&mut self, word: bool) {
        if self.doc.has_sel() {
            let sel = self.doc.sel();
            self.doc.replace(sel, "", EditKind::Delete);
            self.edited();
            return;
        }
        let head = self.doc.head;
        if head >= self.doc.text.len() {
            return;
        }
        if self.view.mode == Mode::Markdown {
            if let Some(msg) = self.read_only_here() {
                self.flash(msg);
                return;
            }
            let loc = self.caret_loc();
            let b = &self.view.blocks[loc.block];
            let para = b.para_at(loc.para);
            if loc.d >= para.len() && loc.para + 1 >= b.para_count() {
                // At the end of a block: join with the next one.
                if let Some(next) = self.view.blocks.get(loc.block + 1) {
                    if !next.editable() {
                        return;
                    }
                    let start = next.para.src_span().map(|(s, _)| s).unwrap_or(next.src.start);
                    if start > head {
                        self.doc.replace(
                            (head, start),
                            if b.kind == Kind::Paragraph && next.kind == Kind::Paragraph { " " } else { "" },
                            EditKind::Delete,
                        );
                        self.doc.set_sel(head, head);
                        self.edited();
                    }
                }
                return;
            }
            let text = String::from_utf16_lossy(&para.text);
            let d = if word { word_right_d(&text, loc.d) } else { grapheme_right_d(&text, loc.d) };
            let e = para.d2s(d, true).max(head + 1).min(self.doc.text.len());
            let e = editor::clamp_boundary(&self.doc.text, e);
            self.doc.replace((head, e.max(next_char(&self.doc.text, head))), "", EditKind::Delete);
            self.doc.set_sel(head, head);
            self.edited();
            return;
        }
        let e = if word { word_right_src(&self.doc.text, head) } else { next_char(&self.doc.text, head) };
        self.doc.replace((head, e), "", EditKind::Delete);
        self.doc.set_sel(head, head);
        self.edited();
    }

    fn enter(&mut self) {
        if !self.doc.has_sel() {
            if let Some(msg) = self.read_only_here() {
                self.flash(msg);
                return;
            }
        }
        let in_code = self.in_code_block();
        let md = self.view.mode == Mode::Markdown;
        let shift = key_down(VK_SHIFT);
        if shift && md && !in_code {
            // Line break inside a paragraph.
            self.insert_text("\\\n", EditKind::Other);
            return;
        }
        let c = editor::smart_newline(&self.doc.text, self.doc.sel(), md, in_code);
        self.apply(c, None);
    }

    fn tab(&mut self, outdent: bool) {
        let sel = self.doc.sel();
        if let Some(c) = editor::indent_lines(&self.doc.text, sel, outdent) {
            let had_sel = self.doc.has_sel();
            let ls = c.range.0;
            let new_end = ls + c.text.len();
            self.apply(c, if had_sel { Some((ls, new_end)) } else { None });
            return;
        }
        if outdent {
            return;
        }
        if self.view.mode == Mode::Markdown && !self.in_code_block() {
            let head = self.doc.head;
            if head == editor::line_start(&self.doc.text, head) {
                return; // a leading tab would turn the line into code
            }
        }
        let col = self.doc.line_col(self.doc.head).1 - 1;
        let n = 4 - (col % 4);
        self.insert_text(&" ".repeat(n), EditKind::Typing);
    }

    fn wrap_with(&mut self, mark: &str) {
        let (c, sel) = editor::toggle_wrap(&self.doc.text, self.doc.sel(), mark);
        self.apply(c, Some(sel));
    }

    fn insert_link(&mut self) {
        let sel = self.doc.sel();
        let selected = self.doc.text[sel.0..sel.1].to_string();
        let clip = clipboard_get(self.hwnd).filter(|c| {
            let c = c.trim();
            (c.starts_with("http://") || c.starts_with("https://") || c.starts_with("mailto:")) && !c.contains(char::is_whitespace)
        });
        let url = match clip {
            Some(u) => Some(u.trim().to_string()),
            None => self.ask("Insert link", "Link address (URL or path):", ""),
        };
        let Some(url) = url else { return };
        let label = if selected.is_empty() { url.clone() } else { selected };
        let text = format!("[{label}]({url})");
        let caret = sel.0 + text.len();
        self.apply(editor::Change { range: sel, text, caret }, None);
    }

    fn ask(&self, title: &str, label: &str, initial: &str) -> Option<String> {
        let bg = self.theme.style(El::Status).background.unwrap_or(self.body_bg());
        let ink = self.theme.style(El::Body).color;
        let field = self.body_bg();
        prompt(self.hwnd, title, label, initial, (bg, ink, field), self.scale)
    }

    // ------------------------------------------------------------ movement

    fn move_to(&mut self, pos: usize, extend: bool, keep_sticky: bool) {
        let pos = editor::clamp_boundary(&self.doc.text, pos);
        if extend {
            self.doc.head = pos;
        } else {
            self.doc.set_sel(pos, pos);
        }
        self.caret_moved(keep_sticky);
    }

    fn horizontal(&mut self, dir: i32, word: bool, extend: bool) {
        if self.doc.has_sel() && !extend {
            let (a, b) = self.doc.sel();
            self.move_to(if dir < 0 { a } else { b }, false, false);
            return;
        }
        let head = self.doc.head;
        if self.view.mode == Mode::Mono {
            let p = match (dir < 0, word) {
                (true, false) => prev_char(&self.doc.text, head),
                (false, false) => next_char(&self.doc.text, head),
                (true, true) => word_left_src(&self.doc.text, head),
                (false, true) => word_right_src(&self.doc.text, head),
            };
            self.move_to(p, extend, false);
            return;
        }
        let loc = self.caret_loc();
        let b = &self.view.blocks[loc.block];
        let para = b.para_at(loc.para);
        let text = String::from_utf16_lossy(&para.text);
        let pos = if dir < 0 {
            if loc.d == 0 {
                // previous paragraph's end
                match self.prev_para(loc) {
                    Some(l) => self.view.loc_to_src(l, true),
                    None => 0,
                }
            } else {
                let d = if word { word_left_d(&text, loc.d) } else { grapheme_left_d(&text, loc.d) };
                para.d2s(d, true)
            }
        } else if loc.d >= para.len() {
            match self.next_para(loc) {
                Some(l) => self.view.loc_to_src(l, false),
                None => self.doc.text.len(),
            }
        } else {
            let d = if word { word_right_d(&text, loc.d) } else { grapheme_right_d(&text, loc.d) };
            let p = para.d2s(d, true);
            if p <= head {
                para.d2s(d, false).max(next_char(&self.doc.text, head))
            } else {
                p
            }
        };
        self.move_to(pos, extend, false);
    }

    fn prev_para(&self, loc: Loc) -> Option<Loc> {
        if loc.para > 0 {
            let p = self.view.blocks[loc.block].para_at(loc.para - 1);
            return Some(Loc { block: loc.block, para: loc.para - 1, d: p.len() });
        }
        let mut i = loc.block;
        while i > 0 {
            i -= 1;
            let b = &self.view.blocks[i];
            if matches!(b.kind, Kind::Rule) {
                continue;
            }
            let k = b.para_count() - 1;
            return Some(Loc { block: i, para: k, d: b.para_at(k).len() });
        }
        None
    }

    fn next_para(&self, loc: Loc) -> Option<Loc> {
        let b = &self.view.blocks[loc.block];
        if loc.para + 1 < b.para_count() {
            return Some(Loc { block: loc.block, para: loc.para + 1, d: 0 });
        }
        let mut i = loc.block + 1;
        while i < self.view.blocks.len() {
            if !matches!(self.view.blocks[i].kind, Kind::Rule) {
                return Some(Loc { block: i, para: 0, d: 0 });
            }
            i += 1;
        }
        None
    }

    fn vertical(&mut self, dir: i32, extend: bool, page: bool) {
        if self.doc.has_sel() && !extend && !page {
            let (a, b) = self.doc.sel();
            self.doc.set_sel(if dir < 0 { a } else { b }, if dir < 0 { a } else { b });
        }
        let Some((cx, cy, ch)) = self.caret_rect() else { return };
        let x = *self.sticky_x.get_or_insert(cx);
        // Move by whole lines: the caret box is the glyph height, the line box is taller.
        let (line_top, line_bottom) = self.view.line_box(self.caret_loc()).unwrap_or((cy, cy + ch));
        let target_y = if page {
            let step = self.view.view_h() * 0.9;
            self.view.scroll = (self.view.scroll + step * dir as f32).clamp(0.0, self.view.max_scroll());
            self.view.prepare(&self.gfx, &self.theme);
            cy + ch / 2.0 + step * dir as f32
        } else if dir < 0 {
            line_top - 1.0
        } else {
            line_bottom + 1.0
        };
        if target_y < 0.0 || (dir < 0 && self.caret_loc().block == 0 && target_y < self.view.geo.first().map(|g| g.y).unwrap_or(0.0)) {
            self.move_to(0, extend, true);
            return;
        }
        if target_y > self.view.doc_h {
            let end = self.doc.text.len();
            self.move_to(end, extend, true);
            return;
        }
        let mut i = self.view.block_at_y(target_y);
        self.view.ensure_laid(i, &self.gfx, &self.theme);
        let mut ty = target_y;
        let g = &self.view.geo[i];
        if dir > 0 && ty < g.y {
            ty = g.y + 1.0;
        }
        if dir < 0 && ty < g.y {
            if i > 0 {
                i -= 1;
                self.view.ensure_laid(i, &self.gfx, &self.theme);
                let g = &self.view.geo[i];
                ty = g.y + g.h - 1.0;
            } else {
                ty = g.y + 1.0;
            }
        }
        // Skip rules: nothing to put the caret on.
        while matches!(self.view.blocks[i].kind, Kind::Rule) {
            let ni = if dir < 0 { i.checked_sub(1) } else { Some(i + 1).filter(|&n| n < self.view.blocks.len()) };
            match ni {
                Some(n) => {
                    i = n;
                    self.view.ensure_laid(i, &self.gfx, &self.theme);
                    let g = &self.view.geo[i];
                    ty = if dir < 0 { g.y + g.h - 1.0 } else { g.y + 1.0 };
                }
                None => break,
            }
        }
        let mut loc = self.view.hit_in_block(i, x, ty);
        // Keep stepping until the hit lands on a different line (table row padding, block gaps).
        if !page {
            for _ in 0..400 {
                let moved = match self.view.line_box(loc) {
                    Some((t, _)) => {
                        if dir > 0 {
                            t > line_top + 0.5
                        } else {
                            t < line_top - 0.5
                        }
                    }
                    None => true,
                };
                if moved {
                    break;
                }
                ty += 4.0 * dir as f32;
                let g = &self.view.geo[i];
                if dir > 0 && ty > g.y + g.h {
                    if i + 1 >= self.view.geo.len() {
                        break;
                    }
                    i += 1;
                    self.view.ensure_laid(i, &self.gfx, &self.theme);
                    ty = self.view.geo[i].y + 1.0;
                } else if dir < 0 && ty < g.y {
                    if i == 0 {
                        break;
                    }
                    i -= 1;
                    self.view.ensure_laid(i, &self.gfx, &self.theme);
                    let g = &self.view.geo[i];
                    ty = g.y + g.h - 1.0;
                }
                loc = self.view.hit_in_block(i, x, ty);
            }
        }
        let pos = self.view.loc_to_src(loc, true);
        trace(&format!(
            "vertical dir={dir} caret=({cx:.1},{cy:.1},{ch:.1}) target={target_y:.1} i={i} ty={ty:.1} loc={loc:?} pos={pos} head={}",
            self.doc.head
        ));
        if pos == self.doc.head && !page {
            // Could not move (e.g. top line): go to the start or end.
            let p = if dir < 0 { 0 } else { self.doc.text.len() };
            let l = self.caret_loc();
            if (dir < 0 && l.block == 0) || (dir > 0 && l.block + 1 >= self.view.blocks.len()) {
                self.move_to(p, extend, true);
            }
            return;
        }
        self.move_to(pos, extend, true);
    }

    fn home_end(&mut self, end: bool, doc: bool, extend: bool) {
        if doc {
            let p = if end { self.doc.text.len() } else { 0 };
            self.move_to(p, extend, false);
            return;
        }
        let loc = self.caret_loc();
        self.view.ensure_laid(loc.block, &self.gfx, &self.theme);
        let (s, e) = self.view.line_bounds(loc);
        let d = if end { e } else { s };
        let pos = self.view.loc_to_src(Loc { d, ..loc }, end);
        self.move_to(pos, extend, false);
    }

    // ------------------------------------------------------------ mode

    fn toggle_mode(&mut self) {
        let screen_y = self.caret_rect().map(|(_, y, _)| y - self.view.scroll);
        self.view.mode = if self.view.mode == Mode::Markdown { Mode::Mono } else { Mode::Markdown };
        self.view.geo.clear();
        self.rebuild();
        self.update_mode_item();
        self.apply_window_theme();
        // Keep the caret at the same height on screen (clamped by the document's ends).
        let loc = self.caret_loc();
        self.view.ensure_laid(loc.block, &self.gfx, &self.theme);
        if let Some(sy) = screen_y {
            for _ in 0..6 {
                let Some((_, y, _)) = self.view.caret_rect(self.caret_loc()) else { break };
                let target = (y - sy).clamp(0.0, self.view.max_scroll());
                let done = (target - self.view.scroll).abs() < 0.5;
                self.view.scroll = target;
                self.view.prepare(&self.gfx, &self.theme);
                if done {
                    break;
                }
            }
        } else {
            self.ensure_caret_visible();
        }
        if let Some(log) = std::env::var_os("BRISK_TRACE") {
            use std::io::Write;
            let after = self.caret_rect().map(|(_, y, _)| y - self.view.scroll);
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
                let _ = writeln!(
                    f,
                    "toggle mode={:?} head={} before={:?} after={:?} scroll={:.1} max={:.1}",
                    self.view.mode,
                    self.doc.head,
                    screen_y,
                    after,
                    self.view.scroll,
                    self.view.max_scroll()
                );
            }
        }
        self.caret_on = true;
        self.restart_blink();
        self.invalidate();
    }

    fn set_zoom(&mut self, z: f32) {
        let anchor = self.top_anchor();
        self.view.zoom = z.clamp(0.5, 3.0);
        self.settings.zoom = self.view.zoom;
        self.view.invalidate(&self.theme);
        self.restore_anchor(anchor);
        self.flash(&format!("Zoom {}%", (self.view.zoom * 100.0).round()));
    }

    fn set_theme(&mut self, name: &str) {
        let anchor = self.top_anchor();
        self.theme = load_theme(name);
        self.settings.theme = self.theme.name.clone();
        self.view.invalidate(&self.theme);
        self.apply_window_theme();
        self.restore_anchor(anchor);
        if !self.theme.warnings.is_empty() {
            let n = self.theme.warnings.len();
            self.flash(&format!(
                "Theme \"{}\": {} rule{} ignored ({})",
                self.theme.name,
                n,
                if n == 1 { "" } else { "s" },
                self.theme.warnings[0]
            ));
        }
        self.invalidate();
    }

    // ------------------------------------------------------------ files

    fn confirm_discard(&mut self) -> bool {
        if !self.doc.dirty {
            return true;
        }
        let name = self
            .doc
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".into());
        let msg = wstr(&format!("Save changes to {name}?"));
        let r = unsafe { MessageBoxW(Some(self.hwnd), PCWSTR(msg.as_ptr()), w!("BriskMarkdown"), MB_YESNOCANCEL | MB_ICONQUESTION) };
        match r {
            IDYES => self.save(false),
            IDNO => true,
            _ => false,
        }
    }

    fn open_path(&mut self, path: &Path) {
        match Doc::load(path) {
            Ok(d) => {
                self.doc = d;
                self.settings.add_recent(path);
                self.view.scroll = 0.0;
                self.view.geo.clear();
                self.rebuild();
                self.update_title();
                self.invalidate();
            }
            Err(e) => {
                let msg = wstr(&format!("Could not open {}:\n{e}", path.display()));
                unsafe {
                    MessageBoxW(Some(self.hwnd), PCWSTR(msg.as_ptr()), w!("BriskMarkdown"), MB_OK | MB_ICONWARNING);
                }
            }
        }
    }

    fn open_dialog(&mut self) {
        if let Some(p) = file_dialog(self.hwnd, false, self.doc.path.as_deref()) {
            if self.doc.path.is_none() && !self.doc.dirty && self.doc.text.is_empty() {
                self.open_path(&p);
            } else if self.confirm_discard() {
                self.open_path(&p);
            }
        }
    }

    fn save(&mut self, as_new: bool) -> bool {
        let path = match (&self.doc.path, as_new) {
            (Some(p), false) => p.clone(),
            _ => match file_dialog(self.hwnd, true, self.doc.path.as_deref()) {
                Some(p) => p,
                None => return false,
            },
        };
        match self.doc.save_to(&path) {
            Ok(()) => {
                self.settings.add_recent(&path);
                self.update_title();
                self.flash("Saved");
                true
            }
            Err(e) => {
                let msg = wstr(&format!("Could not save {}:\n{e}", path.display()));
                unsafe {
                    MessageBoxW(Some(self.hwnd), PCWSTR(msg.as_ptr()), w!("BriskMarkdown"), MB_OK | MB_ICONWARNING);
                }
                false
            }
        }
    }

    fn reload(&mut self, quiet: bool) {
        let Some(path) = self.doc.path.clone() else { return };
        if !quiet && self.doc.dirty {
            let r = unsafe {
                MessageBoxW(
                    Some(self.hwnd),
                    w!("Discard your changes and reload the file from disk?"),
                    w!("BriskMarkdown"),
                    MB_OKCANCEL | MB_ICONQUESTION,
                )
            };
            if r != IDOK {
                return;
            }
        }
        if let Ok(d) = Doc::load(&path) {
            let anchor = self.top_anchor();
            let head = self.doc.head;
            self.doc.reload_text(d.text);
            self.doc.stamp = d.stamp;
            self.doc.crlf = d.crlf;
            self.doc.enc = d.enc;
            self.doc.bom = d.bom;
            let h = head.min(self.doc.text.len());
            self.doc.set_sel(h, h);
            self.rebuild();
            self.restore_anchor(anchor);
            self.disk_note = false;
            self.update_title();
            if !quiet {
                self.flash("Reloaded from disk");
            }
            self.invalidate();
        }
    }

    fn check_disk(&mut self) {
        let Some(path) = self.doc.path.clone() else { return };
        let stamp = editor::file_stamp(&path);
        if stamp.0 == 0 || stamp == self.doc.stamp {
            return;
        }
        if !self.doc.dirty {
            self.reload(true);
        } else if !self.disk_note {
            self.disk_note = true;
            self.flash("The file changed on disk. File > Reload from Disk (Ctrl+R) loads it.");
        }
    }

    // ------------------------------------------------------------ find

    fn compute_finds(&mut self) {
        self.finds.clear();
        if self.find_text.is_empty() {
            return;
        }
        let (hay, needle) = if self.find_case {
            (self.doc.text.clone(), self.find_text.clone())
        } else {
            (self.doc.text.to_lowercase(), self.find_text.to_lowercase())
        };
        if hay.len() != self.doc.text.len() {
            // lowercasing changed byte lengths (rare scripts): fall back to case-sensitive
            return self.compute_finds_exact();
        }
        let mut from = 0;
        while let Some(i) = hay[from..].find(&needle) {
            let a = from + i;
            let b = a + needle.len();
            if !self.find_word || is_word_match(&self.doc.text, a, b) {
                self.finds.push((a, b));
            }
            from = b.max(a + 1);
            while from < hay.len() && !hay.is_char_boundary(from) {
                from += 1;
            }
            if self.finds.len() > 20000 {
                break;
            }
        }
    }

    fn compute_finds_exact(&mut self) {
        let mut from = 0;
        while let Some(i) = self.doc.text[from..].find(&self.find_text) {
            let a = from + i;
            self.finds.push((a, a + self.find_text.len()));
            from = a + self.find_text.len().max(1);
        }
    }

    fn find_step(&mut self, forward: bool) {
        if self.find_text.is_empty() {
            self.open_find(false);
            return;
        }
        self.compute_finds();
        if self.finds.is_empty() {
            self.flash(&format!("\"{}\" not found", self.find_text));
            self.invalidate();
            return;
        }
        let (a, b) = self.doc.sel();
        let idx = if forward {
            self.finds.iter().position(|&(s, _)| s >= b.max(a + if a == b { 0 } else { 1 })).unwrap_or(0)
        } else {
            self.finds.iter().rposition(|&(s, _)| s < a).unwrap_or(self.finds.len() - 1)
        };
        self.find_cur = Some(idx);
        let (s, e) = self.finds[idx];
        self.doc.set_sel(s, e);
        self.caret_moved(false);
        self.flash(&format!("{} of {}", idx + 1, self.finds.len()));
    }

    fn open_find(&mut self, replace: bool) {
        unsafe {
            if !self.find_hwnd.is_invalid() && IsWindow(Some(self.find_hwnd)).as_bool() {
                let _ = DestroyWindow(self.find_hwnd);
            }
            let (a, b) = self.doc.sel();
            if b > a && b - a < 200 && !self.doc.text[a..b].contains('\n') {
                self.find_text = self.doc.text[a..b].to_string();
            }
            let w: Vec<u16> = self.find_text.encode_utf16().take(255).collect();
            self.fr_find.fill(0);
            self.fr_find[..w.len()].copy_from_slice(&w);
            self.fr.lpstrFindWhat = PWSTR(self.fr_find.as_mut_ptr());
            self.fr.lpstrReplaceWith = PWSTR(self.fr_repl.as_mut_ptr());
            self.fr.wFindWhatLen = 256 * 2;
            self.fr.wReplaceWithLen = 256 * 2;
            self.fr.hwndOwner = self.hwnd;
            self.fr.Flags = FR_DOWN
                | if self.find_case { FR_MATCHCASE } else { FINDREPLACE_FLAGS(0) }
                | if self.find_word { FR_WHOLEWORD } else { FINDREPLACE_FLAGS(0) };
            self.find_hwnd = if replace { ReplaceTextW(&mut *self.fr) } else { FindTextW(&mut *self.fr) };
        }
    }

    fn on_find_msg(&mut self) {
        let flags = self.fr.Flags;
        if flags.contains(FR_DIALOGTERM) {
            self.find_hwnd = HWND::default();
            return;
        }
        let len = self.fr_find.iter().position(|&c| c == 0).unwrap_or(0);
        self.find_text = String::from_utf16_lossy(&self.fr_find[..len]);
        self.find_case = flags.contains(FR_MATCHCASE);
        self.find_word = flags.contains(FR_WHOLEWORD);
        let rlen = self.fr_repl.iter().position(|&c| c == 0).unwrap_or(0);
        let repl = String::from_utf16_lossy(&self.fr_repl[..rlen]);
        if flags.contains(FR_REPLACEALL) {
            self.compute_finds();
            if self.finds.is_empty() {
                self.flash("Nothing to replace");
                return;
            }
            let n = self.finds.len();
            let mut out = String::with_capacity(self.doc.text.len());
            let mut last = 0;
            for &(a, b) in &self.finds {
                out.push_str(&self.doc.text[last..a]);
                out.push_str(&repl);
                last = b;
            }
            out.push_str(&self.doc.text[last..]);
            let len = self.doc.text.len();
            self.doc.begin_group();
            self.doc.replace((0, len), &out, EditKind::Other);
            self.doc.set_sel(0, 0);
            self.edited();
            self.flash(&format!("Replaced {n}"));
            return;
        }
        if flags.contains(FR_REPLACE) {
            let (a, b) = self.doc.sel();
            let cur = &self.doc.text[a..b];
            let matches = if self.find_case { cur == self.find_text } else { cur.to_lowercase() == self.find_text.to_lowercase() };
            if matches && b > a {
                self.doc.begin_group();
                self.doc.replace((a, b), &repl, EditKind::Other);
                self.edited();
            }
            self.find_step(true);
            return;
        }
        if flags.contains(FR_FINDNEXT) {
            self.find_step(flags.contains(FR_DOWN));
        }
    }

    // ------------------------------------------------------------ commands

    fn command(&mut self, id: u16) {
        match id {
            CMD_NEW => {
                if self.confirm_discard() {
                    self.doc = Doc::new();
                    self.view.scroll = 0.0;
                    self.rebuild();
                    self.update_title();
                    self.invalidate();
                }
            }
            CMD_NEW_WINDOW => spawn_self(&[]),
            CMD_OPEN => self.open_dialog(),
            CMD_SAVE => {
                self.save(false);
            }
            CMD_SAVE_AS => {
                self.save(true);
            }
            CMD_RELOAD => self.reload(false),
            CMD_REVEAL => {
                if let Some(p) = &self.doc.path {
                    reveal_in_explorer(p);
                }
            }
            CMD_PRINT => crate::print::print(self.hwnd, &self.doc.text, self.doc.path.as_deref()),
            CMD_EXIT => unsafe {
                let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            },
            CMD_UNDO => {
                if self.doc.undo() {
                    self.edited();
                }
            }
            CMD_REDO => {
                if self.doc.redo() {
                    self.edited();
                }
            }
            CMD_CUT => {
                if self.doc.has_sel() {
                    let (a, b) = self.doc.sel();
                    clipboard_set(self.hwnd, &self.doc.text[a..b]);
                    self.doc.replace((a, b), "", EditKind::Other);
                    self.edited();
                }
            }
            CMD_COPY => {
                if self.doc.has_sel() {
                    let (a, b) = self.doc.sel();
                    clipboard_set(self.hwnd, &self.doc.text[a..b]);
                    self.flash("Copied as Markdown");
                }
            }
            CMD_COPY_PLAIN => {
                if self.doc.has_sel() {
                    let t = self.plain_selection();
                    clipboard_set(self.hwnd, &t);
                    self.flash("Copied as plain text");
                }
            }
            CMD_PASTE => {
                if let Some(t) = clipboard_get(self.hwnd) {
                    self.doc.begin_group();
                    self.insert_text(&t, EditKind::Other);
                }
            }
            CMD_DELETE => self.delete_forward(false),
            CMD_SELECT_ALL => {
                let n = self.doc.text.len();
                self.doc.set_sel(0, n);
                self.invalidate();
            }
            CMD_FIND => self.open_find(false),
            CMD_REPLACE => self.open_find(true),
            CMD_FIND_NEXT => self.find_step(true),
            CMD_FIND_PREV => self.find_step(false),
            CMD_GOTO => {
                let (line, _) = self.doc.line_col(self.doc.head);
                let total = self.doc.text.matches('\n').count() + 1;
                if let Some(s) = self.ask("Go To Line", &format!("Line number (1 to {total}):"), &line.to_string()) {
                    if let Ok(n) = s.trim().parse::<usize>() {
                        let off = self.doc.offset_of_line(n.clamp(1, total));
                        self.move_to(off, false, false);
                    }
                }
            }
            CMD_TIME_DATE => {
                let t = now_string();
                self.insert_text(&t, EditKind::Other);
            }
            CMD_BOLD => self.wrap_with("**"),
            CMD_ITALIC => self.wrap_with("*"),
            CMD_STRIKE => self.wrap_with("~~"),
            CMD_CODE => self.wrap_with("`"),
            CMD_LINK => self.insert_link(),
            n if (CMD_H0 + 1..=CMD_H0 + 6).contains(&n) => {
                let c = editor::set_heading(&self.doc.text, self.doc.head, (n - CMD_H0) as usize);
                self.apply(c, None);
            }
            CMD_INDENT => self.tab(false),
            CMD_OUTDENT => self.tab(true),
            CMD_MODE => self.toggle_mode(),
            CMD_ZOOM_IN => self.set_zoom((self.view.zoom * 10.0).round() / 10.0 + 0.1),
            CMD_ZOOM_OUT => self.set_zoom((self.view.zoom * 10.0).round() / 10.0 - 0.1),
            CMD_ZOOM_RESET => self.set_zoom(1.0),
            CMD_WRAP => {
                self.settings.wrap = !self.settings.wrap;
                self.view.wrap = self.settings.wrap;
                let a = self.top_anchor();
                self.view.invalidate(&self.theme);
                self.restore_anchor(a);
                self.invalidate();
            }
            CMD_STATUS => {
                self.settings.status = !self.settings.status;
                self.layout_viewport();
                self.invalidate();
            }
            n if (CMD_THEME0..CMD_THEME0 + 30).contains(&n) => {
                let k = (n - CMD_THEME0) as usize;
                let names: Vec<String> =
                    crate::theme::BUILTIN.iter().map(|(n, _)| n.to_string()).chain(self.user_themes.iter().cloned()).collect();
                if let Some(name) = names.get(k) {
                    let name = name.clone();
                    self.set_theme(&name);
                }
            }
            CMD_THEMES_FOLDER => {
                let dir = themes_dir();
                let _ = std::fs::create_dir_all(&dir);
                let example = dir.join("my-theme.css");
                if std::fs::read_dir(&dir).map(|mut d| d.next().is_none()).unwrap_or(true) {
                    let _ = std::fs::write(&example, crate::THEME_EXAMPLE);
                }
                shell_open(&dir.to_string_lossy());
            }
            CMD_THEME_RELOAD => {
                let name = self.theme.name.clone();
                self.set_theme(&name);
            }
            CMD_SHORTCUTS => unsafe {
                let t = wstr(crate::SHORTCUTS);
                MessageBoxW(Some(self.hwnd), PCWSTR(t.as_ptr()), w!("Keyboard shortcuts"), MB_OK);
            },
            CMD_DEFAULT_APP => {
                if register() {
                    self.settings.offered_default = true;
                    let r = unsafe {
                        MessageBoxW(
                            Some(self.hwnd),
                            w!("BriskMarkdown is now registered for .md files.\n\nWindows asks you to confirm the default yourself. Press OK to open Settings, then choose BriskMarkdown for .md (and .markdown)."),
                            w!("Default app"),
                            MB_OKCANCEL | MB_ICONINFORMATION,
                        )
                    };
                    if r == IDOK {
                        open_default_apps_settings();
                    }
                } else {
                    self.flash("Could not register the file association");
                }
            }
            CMD_WEBSITE => shell_open(WEBSITE),
            CMD_ABOUT => {
                let t = wstr(&format!(
                    "BriskMarkdown {}\n\nA fast, lightweight Markdown notepad for Windows.\nMIT license.\n\n{}",
                    env!("CARGO_PKG_VERSION"),
                    WEBSITE
                ));
                unsafe {
                    MessageBoxW(Some(self.hwnd), PCWSTR(t.as_ptr()), w!("About BriskMarkdown"), MB_OK);
                }
            }
            n if (CMD_RECENT0..CMD_RECENT0 + 10).contains(&n) => {
                if let Some(p) = self.settings.recent.get((n - CMD_RECENT0) as usize).cloned() {
                    if self.confirm_discard() {
                        self.open_path(Path::new(&p));
                    }
                }
            }
            _ => {}
        }
    }

    fn plain_selection(&self) -> String {
        let (a, b) = self.doc.sel();
        if self.view.mode == Mode::Mono {
            return self.doc.text[a..b].to_string();
        }
        let mut out = String::new();
        for blk in &self.view.blocks {
            if blk.src.end < a || blk.src.start > b {
                continue;
            }
            for p in blk.paras() {
                let Some((ps, pe)) = p.src_span() else { continue };
                if pe < a || ps > b {
                    continue;
                }
                let d0 = p.s2d(a.max(ps)) as usize;
                let d1 = p.s2d(b.min(pe)) as usize;
                if d1 > d0 {
                    if !out.is_empty() {
                        out.push_str(if blk.is_table() { "\t" } else { "\n" });
                    }
                    out.push_str(&String::from_utf16_lossy(&p.text[d0..d1]).replace('\u{2009}', "").replace('\u{2028}', "\n"));
                }
            }
            if !blk.is_table() {
                out.push('\n');
            }
        }
        out.trim_end_matches('\n').to_string()
    }

    fn init_menu_popup(&mut self, menu: HMENU) {
        unsafe {
            // Themes submenu
            let themes = GetSubMenu(self.view_menu, 2);
            if menu == themes {
                while GetMenuItemCount(Some(themes)) > 0 {
                    let _ = DeleteMenu(themes, 0, MF_BYPOSITION);
                }
                self.user_themes = user_theme_names();
                let names: Vec<String> =
                    crate::theme::BUILTIN.iter().map(|(n, _)| n.to_string()).chain(self.user_themes.iter().cloned()).collect();
                for (k, n) in names.iter().enumerate().take(30) {
                    if k == crate::theme::BUILTIN.len() && k > 0 {
                        sep(themes);
                    }
                    append(themes, CMD_THEME0 + k as u16, n);
                    if n.eq_ignore_ascii_case(&self.theme.name) {
                        CheckMenuItem(themes, (CMD_THEME0 + k as u16) as u32, MF_CHECKED.0);
                    }
                }
                sep(themes);
                append(themes, CMD_THEME_RELOAD, "Reload Current Theme");
                append(themes, CMD_THEMES_FOLDER, "Open Themes Folder...");
            }
            let recent = GetSubMenu(self.file_menu, 3);
            if menu == recent {
                while GetMenuItemCount(Some(recent)) > 0 {
                    let _ = DeleteMenu(recent, 0, MF_BYPOSITION);
                }
                if self.settings.recent.is_empty() {
                    append(recent, 0, "(none)");
                    let _ = EnableMenuItem(recent, 0, MF_BYPOSITION | MF_GRAYED);
                }
                for (k, r) in self.settings.recent.iter().enumerate() {
                    append(recent, CMD_RECENT0 + k as u16, &format!("&{} {}", (k + 1) % 10, r.replace('&', "&&")));
                }
            }
            let check = |id: u16, on: bool| {
                CheckMenuItem(self.menu, id as u32, if on { MF_CHECKED.0 } else { MF_UNCHECKED.0 });
            };
            check(CMD_MODE, self.view.mode == Mode::Mono);
            check(CMD_WRAP, self.settings.wrap);
            check(CMD_STATUS, self.settings.status);
            let enable = |id: u16, on: bool| {
                let _ = EnableMenuItem(self.menu, id as u32, if on { MF_ENABLED } else { MF_GRAYED });
            };
            enable(CMD_UNDO, self.doc.can_undo());
            enable(CMD_REDO, self.doc.can_redo());
            enable(CMD_CUT, self.doc.has_sel());
            enable(CMD_COPY, self.doc.has_sel());
            enable(CMD_COPY_PLAIN, self.doc.has_sel());
            enable(CMD_RELOAD, self.doc.path.is_some());
            enable(CMD_REVEAL, self.doc.path.is_some());
        }
    }

    // ------------------------------------------------------------ mouse

    fn doc_point(&self, x: i32, y: i32) -> (f32, f32) {
        let (sx, sy) = (x as f32 / self.scale, y as f32 / self.scale);
        (sx / self.view.zoom, sy / self.view.zoom + self.view.scroll)
    }

    fn in_status(&self, y: i32) -> bool {
        self.settings.status && (y as f32 / self.scale) >= self.view.screen_h
    }

    fn scrollbar_geom(&self) -> Option<(f32, f32, f32)> {
        // (x, thumb_top, thumb_h) in screen DIPs
        let vh = self.view.view_h();
        if self.view.doc_h <= vh + 1.0 {
            return None;
        }
        let track = self.view.screen_h;
        let th = (track * vh / self.view.doc_h).max(28.0).min(track);
        let top = (track - th) * (self.view.scroll / self.view.max_scroll().max(1.0));
        Some((self.view.screen_w - SCROLLBAR_W, top, th))
    }

    fn mode_pill_rect(&self) -> (f32, f32, f32, f32) {
        let y = self.view.screen_h;
        (8.0, y + 3.0, 8.0 + 150.0, y + STATUS_H - 3.0)
    }

    fn on_mouse_down(&mut self, x: i32, y: i32, dbl: bool) {
        unsafe {
            SetCapture(self.hwnd);
        }
        let (sx, sy) = (x as f32 / self.scale, y as f32 / self.scale);
        if self.in_status(y) {
            let (l, _, r, _) = self.mode_pill_rect();
            if sx >= l && sx <= r {
                self.toggle_mode();
            }
            return;
        }
        if let Some((bx, top, th)) = self.scrollbar_geom() {
            if sx >= bx - 2.0 {
                if sy >= top && sy <= top + th {
                    self.drag = Drag::Scroll(sy - top);
                } else {
                    let page = self.view.view_h() * 0.9;
                    self.view.scroll = (self.view.scroll + if sy < top { -page } else { page }).clamp(0.0, self.view.max_scroll());
                    self.view.prepare(&self.gfx, &self.theme);
                    self.invalidate();
                }
                return;
            }
        }
        let (dx, dy) = self.doc_point(x, y);
        // Task checkbox?
        if self.view.mode == Mode::Markdown {
            let i = self.view.block_at_y(dy);
            for j in [i.saturating_sub(1), i, i + 1] {
                let Some(g) = self.view.geo.get(j) else { continue };
                let Some(l) = &g.laid else { continue };
                for (r, src) in &l.tasks {
                    if dx >= g.x + r.left && dx <= g.x + r.right && dy >= g.y + r.top && dy <= g.y + r.bottom {
                        if let Some(c) = editor::toggle_task(&self.doc.text, (src.0 as usize, src.1 as usize)) {
                            let head = self.doc.head;
                            self.doc.begin_group();
                            self.doc.replace(c.range, &c.text, EditKind::Other);
                            self.doc.set_sel(head, head);
                            self.edited();
                        }
                        unsafe {
                            let _ = ReleaseCapture();
                        }
                        return;
                    }
                }
            }
        }
        let loc = self.view.hit(dx, dy, &self.gfx, &self.theme);
        // Ctrl+click opens links.
        if key_down(VK_CONTROL) {
            if let Some(url) = self.link_at(loc) {
                unsafe {
                    let _ = ReleaseCapture();
                }
                self.follow_link(&url);
                return;
            }
        }
        let pos = self.view.loc_to_src(loc, true);
        let now = Instant::now();
        let (t, px, py, n) = self.clicks;
        let count =
            if now.duration_since(t).as_millis() < unsafe { GetDoubleClickTime() } as u128 && (px - x).abs() < 5 && (py - y).abs() < 5 {
                n + 1
            } else {
                1
            };
        let count = if dbl { count.max(2) } else { count };
        self.clicks = (now, x, y, count);
        if count >= 3 {
            // paragraph / line
            let (a, b) = if self.view.mode == Mode::Mono {
                (editor::line_start(&self.doc.text, pos), editor::line_end(&self.doc.text, pos))
            } else {
                let blk = &self.view.blocks[loc.block];
                blk.para_at(loc.para).src_span().unwrap_or((blk.src.start, blk.src.end))
            };
            self.doc.set_sel(a, b);
            self.drag = Drag::None;
            self.caret_moved(false);
            return;
        }
        if count == 2 {
            let (a, b) = editor::word_at(&self.doc.text, pos);
            if b > a {
                self.doc.set_sel(a, b);
                self.drag = Drag::SelectWords(a, b);
                self.caret_moved(false);
                return;
            }
        }
        if key_down(VK_SHIFT) {
            self.doc.head = pos;
        } else {
            self.doc.set_sel(pos, pos);
        }
        self.drag = Drag::Select;
        self.caret_moved(false);
    }

    fn on_mouse_move(&mut self, x: i32, y: i32) {
        let (sx, sy) = (x as f32 / self.scale, y as f32 / self.scale);
        match self.drag {
            Drag::Scroll(off) => {
                if let Some((_, _, th)) = self.scrollbar_geom() {
                    let track = self.view.screen_h - th;
                    let frac = ((sy - off) / track.max(1.0)).clamp(0.0, 1.0);
                    self.view.scroll = frac * self.view.max_scroll();
                    self.view.prepare(&self.gfx, &self.theme);
                    self.invalidate();
                }
            }
            Drag::Select | Drag::SelectWords(..) => {
                let (dx, dy) = self.doc_point(x, y.max(0));
                // auto-scroll when dragging past the edges
                if sy < 0.0 {
                    self.view.scroll = (self.view.scroll - 20.0).max(0.0);
                } else if sy > self.view.screen_h {
                    self.view.scroll = (self.view.scroll + 20.0).min(self.view.max_scroll());
                }
                self.view.prepare(&self.gfx, &self.theme);
                let loc = self.view.hit(dx, dy, &self.gfx, &self.theme);
                let pos = self.view.loc_to_src(loc, true);
                if let Drag::SelectWords(a0, b0) = self.drag {
                    let (wa, wb) = editor::word_at(&self.doc.text, pos);
                    if pos < a0 {
                        self.doc.anchor = b0;
                        self.doc.head = wa;
                    } else {
                        self.doc.anchor = a0;
                        self.doc.head = wb.max(b0);
                    }
                } else {
                    self.doc.head = pos;
                }
                self.invalidate();
            }
            Drag::None => {
                let over_bar = self.scrollbar_geom().map(|(bx, _, _)| sx >= bx - 2.0 && sy < self.view.screen_h).unwrap_or(false);
                if over_bar != self.scroll_hover {
                    self.scroll_hover = over_bar;
                    self.invalidate();
                }
            }
        }
    }

    fn on_mouse_up(&mut self) {
        unsafe {
            let _ = ReleaseCapture();
        }
        if matches!(self.drag, Drag::Select | Drag::SelectWords(..)) {
            self.drag = Drag::None;
            self.caret_moved(false);
        }
        self.drag = Drag::None;
    }

    fn link_at(&self, loc: Loc) -> Option<String> {
        let b = self.view.blocks.get(loc.block)?;
        let p = b.para_at(loc.para);
        for s in &p.spans {
            if matches!(s.kind, Inline::Link | Inline::WikiLink | Inline::Image) && loc.d >= s.start && loc.d <= s.end {
                let url = b.links.get(s.link as usize)?.clone();
                if s.kind == Inline::WikiLink {
                    return Some(format!("wiki:{url}"));
                }
                return Some(url);
            }
        }
        None
    }

    fn follow_link(&mut self, url: &str) {
        let url = url.trim();
        if let Some(anchor) = url.strip_prefix('#') {
            // Scroll to the heading whose slug matches.
            let target = slug(anchor);
            for b in &self.view.blocks {
                if let Kind::Heading(_) = b.kind {
                    if slug(&String::from_utf16_lossy(&b.para.text)) == target {
                        let s = b.src.start;
                        self.move_to(s, false, false);
                        return;
                    }
                }
            }
            return;
        }
        if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") {
            shell_open(url);
            return;
        }
        let base = self.doc.path.as_ref().and_then(|p| p.parent().map(|p| p.to_path_buf())).unwrap_or_default();
        let (path_part, is_wiki) = match url.strip_prefix("wiki:") {
            Some(w) => (w.split('|').next().unwrap_or(w).split('#').next().unwrap_or("").to_string(), true),
            None => (url.split('#').next().unwrap_or("").to_string(), false),
        };
        let decoded = path_part.replace("%20", " ");
        let mut cand = base.join(&decoded);
        if is_wiki && cand.extension().is_none() {
            cand.set_extension("md");
        }
        if cand.exists() {
            let is_md = cand
                .extension()
                .map(|e| platform::EXTENSIONS.iter().any(|x| x[1..].eq_ignore_ascii_case(&e.to_string_lossy())))
                .unwrap_or(false);
            if is_md {
                spawn_self(&[&cand.to_string_lossy()]);
            } else {
                shell_open(&cand.to_string_lossy());
            }
        } else {
            self.flash(&format!("Not found: {}", cand.display()));
        }
    }

    // ------------------------------------------------------------ painting

    fn ensure_target(&mut self) -> bool {
        if self.hrt.is_some() {
            return true;
        }
        unsafe {
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            let dpi = self.scale * 96.0;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: if std::env::var("BRISK_GPU").is_ok() { D2D1_RENDER_TARGET_TYPE_DEFAULT } else { D2D1_RENDER_TARGET_TYPE_SOFTWARE },
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                dpiX: dpi,
                dpiY: dpi,
                ..Default::default()
            };
            let hp = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd: self.hwnd,
                pixelSize: D2D_SIZE_U { width: (rc.right - rc.left).max(1) as u32, height: (rc.bottom - rc.top).max(1) as u32 },
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            match self.gfx.d2d.CreateHwndRenderTarget(&props, &hp) {
                Ok(rt) => {
                    rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);
                    self.gfx.set_target(Some(rt.cast().unwrap()));
                    self.hrt = Some(rt);
                    // Layouts hold brushes from the old target.
                    for g in self.view.geo.iter_mut() {
                        g.laid = None;
                    }
                    self.view.remaining = self.view.geo.len();
                    true
                }
                Err(_) => false,
            }
        }
    }

    fn paint(&mut self) {
        if !self.ensure_target() {
            return;
        }
        self.view.prepare(&self.gfx, &self.theme);
        let caret = if self.caret_on && self.drag == Drag::None || self.caret_on { self.caret_rect() } else { None };
        let caret = if self.doc.has_sel() && self.view.mode == Mode::Markdown { None } else { caret };
        let rt: ID2D1RenderTarget = self.gfx.rt.clone().unwrap();
        unsafe {
            rt.BeginDraw();
            self.paint_into(&rt, caret);
            if rt.EndDraw(None, None).is_err() {
                self.hrt = None;
                self.gfx.set_target(None);
            }
        }
        if !self.painted {
            self.painted = true;
            if let Some(log) = &self.bench {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
                    let _ = writeln!(
                        f,
                        "first_frame_ms={:.1} file={} bytes={} blocks={}",
                        ms_since_start(),
                        self.doc.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
                        self.doc.text.len(),
                        self.view.blocks.len()
                    );
                }
                unsafe { PostQuitMessage(0) };
            }
        }
    }

    fn paint_into(&self, rt: &ID2D1RenderTarget, caret: Option<(f32, f32, f32)>) {
        unsafe {
            rt.Clear(Some(&d2color(&self.body_bg())));
        }
        let sel = self.doc.sel();
        let caret = if self.caret_on { caret } else { None };
        self.view.paint(&self.gfx, &self.theme, rt, 0.0, sel, caret, &self.finds, self.find_cur);
        self.paint_scrollbar(rt);
        if self.settings.status {
            self.paint_status(rt);
        }
    }

    /// Test hook: render the window as it is now (document, caret, selection, scrollbar, status
    /// bar) into a PNG. Used where the desktop can't be captured.
    fn snapshot(&mut self, path: &Path) {
        let Ok(f) = crate::export::wic() else { return };
        let (w, h) = self.client_dips();
        let Ok((bmp, rt)) = crate::export::wic_target(&self.gfx, &f, (w * self.scale) as u32, (h * self.scale) as u32, self.scale) else {
            return;
        };
        let old = self.gfx.rt.clone();
        let anchor = self.top_anchor();
        self.gfx.set_target(Some(rt.clone()));
        for g in self.view.geo.iter_mut() {
            g.laid = None;
        }
        self.restore_anchor(anchor);
        let caret = if self.doc.has_sel() && self.view.mode == Mode::Markdown { None } else { self.caret_rect() };
        let was_on = self.caret_on;
        self.caret_on = true;
        unsafe {
            rt.BeginDraw();
            self.paint_into(&rt, caret);
            let _ = rt.EndDraw(None, None);
        }
        self.caret_on = was_on;
        let _ = crate::export::save_png(&f, &bmp, path);
        self.gfx.set_target(old);
        for g in self.view.geo.iter_mut() {
            g.laid = None;
        }
        self.restore_anchor(anchor);
        self.invalidate();
    }

    fn paint_scrollbar(&self, rt: &ID2D1RenderTarget) {
        let Some((x, top, th)) = self.scrollbar_geom() else { return };
        let c = self.theme.style(El::Scrollbar).background.unwrap_or(Color::rgb(90, 90, 90));
        let active = self.scroll_hover || matches!(self.drag, Drag::Scroll(_));
        let w = if active { 8.0 } else { 5.0 };
        let r = rect(x + SCROLLBAR_W - w - 2.0, top + 2.0, x + SCROLLBAR_W - 2.0, top + th - 2.0);
        unsafe {
            rt.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT { rect: r, radiusX: w / 2.0, radiusY: w / 2.0 },
                &self.gfx.brush(&c.with_alpha(if active { 0.95 } else { 0.7 })),
            );
        }
    }

    fn paint_status(&self, rt: &ID2D1RenderTarget) {
        let st = self.theme.style(El::Status);
        let btn = self.theme.style(El::StatusButton);
        let y = self.view.screen_h;
        let w = self.view.screen_w;
        let fam = self.gfx.pick_family(&st.families);
        let size = st.size;
        unsafe {
            let bg = st.background.unwrap_or(self.body_bg());
            rt.FillRectangle(&rect(0.0, y, w, y + STATUS_H), &self.gfx.brush(&bg));
            if let Some(b) = st.border_top {
                rt.FillRectangle(&rect(0.0, y, w, y + b.width), &self.gfx.brush(&b.color));
            }
            // Mode pill: two segments, the current one highlighted.
            let (l, t, r, bt) = self.mode_pill_rect();
            let pill_bg = bg.mix(&btn.color, 0.06);
            let rr = D2D1_ROUNDED_RECT { rect: rect(l, t, r, bt), radiusX: (bt - t) / 2.0, radiusY: (bt - t) / 2.0 };
            rt.FillRoundedRectangle(&rr, &self.gfx.brush(&pill_bg));
            let mid = (l + r) / 2.0 + 10.0;
            let (al, ar) = if self.view.mode == Mode::Markdown { (l, mid) } else { (mid, r) };
            let active_bg = btn.background.unwrap_or(pill_bg);
            let ar_r = D2D1_ROUNDED_RECT {
                rect: rect(al + 1.0, t + 1.0, ar - 1.0, bt - 1.0),
                radiusX: (bt - t) / 2.0 - 1.0,
                radiusY: (bt - t) / 2.0 - 1.0,
            };
            rt.FillRoundedRectangle(&ar_r, &self.gfx.brush(&active_bg));
            if let Some(b) = btn.border_top {
                rt.DrawRoundedRectangle(&ar_r, &self.gfx.brush(&b.color), 1.0, None);
            }
            let label = |text: &str, x0: f32, x1: f32, on: bool| {
                let lay = self.gfx.simple_layout(text, &fam, size, if on { 600 } else { 400 }, false, x1 - x0);
                let _ = lay.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
                let m = layout_metrics(&lay);
                let c = if on { btn.color } else { st.color };
                rt.DrawTextLayout(pt(x0, t + ((bt - t) - m.height) / 2.0), &lay, &self.gfx.brush(&c), D2D1_DRAW_TEXT_OPTIONS_NONE);
            };
            label("Markdown", l, mid, self.view.mode == Mode::Markdown);
            label("Mono", mid, r, self.view.mode == Mode::Mono);
            // Position and counts
            let (line, col) = self.doc.line_col(self.doc.head);
            let mut left = format!("Ln {line}, Col {col}");
            if self.doc.has_sel() {
                let (a, b) = self.doc.sel();
                left.push_str(&format!("  \u{00B7}  {} selected", self.doc.text[a..b].chars().count()));
            }
            left.push_str(&format!("  \u{00B7}  {} words", fmt_thousands(self.doc.word_count())));
            let lay = self.gfx.simple_layout(&left, &fam, size, 400, false, 600.0);
            let m = layout_metrics(&lay);
            rt.DrawTextLayout(pt(r + 14.0, y + (STATUS_H - m.height) / 2.0), &lay, &self.gfx.brush(&st.color), D2D1_DRAW_TEXT_OPTIONS_NONE);
            let mut x_after = r + 14.0 + m.width;
            // Right side
            let right = format!(
                "{}  \u{00B7}  {}  \u{00B7}  {}%  \u{00B7}  {}",
                self.doc.enc.label(self.doc.bom),
                if self.doc.crlf { "CRLF" } else { "LF" },
                (self.view.zoom * 100.0).round(),
                self.theme.name
            );
            let lay = self.gfx.simple_layout(&right, &fam, size, 400, false, 600.0);
            let m = layout_metrics(&lay);
            let rx = w - m.width - 16.0;
            rt.DrawTextLayout(pt(rx, y + (STATUS_H - m.height) / 2.0), &lay, &self.gfx.brush(&st.color), D2D1_DRAW_TEXT_OPTIONS_NONE);
            // Transient message in the middle
            if let Some((msg, _)) = &self.status_msg {
                x_after += 24.0;
                let avail = (rx - 24.0 - x_after).max(0.0);
                if avail > 40.0 {
                    let lay = self.gfx.simple_layout(msg, &fam, size, 600, false, avail);
                    let _ = lay.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
                    let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                    if let Ok(sign) = self.gfx.dw.CreateEllipsisTrimmingSign(&self.gfx.format(&fam, size, 600, false)) {
                        let _ = lay.SetTrimming(&trimming, &sign);
                    }
                    let m = layout_metrics(&lay);
                    let accent = self.theme.caret;
                    rt.DrawTextLayout(
                        pt(x_after, y + (STATUS_H - m.height) / 2.0),
                        &lay,
                        &self.gfx.brush(&accent),
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
            }
        }
    }

    fn update_ime_pos(&mut self) {
        let Some((x, y, h)) = self.caret_rect() else { return };
        let z = self.view.zoom;
        let sx = (x * z * self.scale) as i32;
        let sy = ((y - self.view.scroll) * z * self.scale) as i32;
        unsafe {
            let himc = ImmGetContext(self.hwnd);
            if !himc.is_invalid() {
                let cf = COMPOSITIONFORM { dwStyle: CFS_POINT, ptCurrentPos: POINT { x: sx, y: sy }, rcArea: RECT::default() };
                let _ = ImmSetCompositionWindow(himc, &cf);
                let _ = h;
                let _ = ImmReleaseContext(self.hwnd, himc);
            }
        }
    }

    fn save_placement(&mut self) {
        unsafe {
            let mut wp = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
            if GetWindowPlacement(self.hwnd, &mut wp).is_ok() {
                let r = wp.rcNormalPosition;
                self.settings.placement =
                    Some((r.left, r.top, r.right - r.left, r.bottom - r.top, wp.showCmd == SW_SHOWMAXIMIZED.0 as u32));
            }
        }
    }
}

pub fn trace(msg: &str) {
    if let Some(log) = std::env::var_os("BRISK_TRACE") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
            let _ = writeln!(f, "{msg}");
        }
    }
}

fn fmt_thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().to_lowercase().chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if c == ' ' {
            out.push('-');
        }
    }
    out
}

fn is_word_match(s: &str, a: usize, b: usize) -> bool {
    let before = s[..a].chars().next_back().map(|c| c.is_alphanumeric() || c == '_').unwrap_or(false);
    let after = s[b..].chars().next().map(|c| c.is_alphanumeric() || c == '_').unwrap_or(false);
    !before && !after
}

fn now_string() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    let (h12, ampm) = match t.wHour {
        0 => (12, "AM"),
        h if h < 12 => (h, "AM"),
        12 => (12, "PM"),
        h => (h - 12, "PM"),
    };
    format!("{}:{:02} {} {}/{}/{}", h12, t.wMinute, ampm, t.wMonth, t.wDay, t.wYear)
}

fn prev_char(s: &str, i: usize) -> usize {
    s[..i.min(s.len())].char_indices().next_back().map(|(k, _)| k).unwrap_or(0)
}

fn next_char(s: &str, i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    i + s[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1)
}

fn is_wordish(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn word_left_src(s: &str, i: usize) -> usize {
    let mut k = i;
    let mut it = s[..i].char_indices().rev().peekable();
    while let Some(&(p, c)) = it.peek() {
        if is_wordish(c) {
            break;
        }
        k = p;
        it.next();
        if c == '\n' {
            return k;
        }
    }
    while let Some(&(p, c)) = it.peek() {
        if !is_wordish(c) {
            break;
        }
        k = p;
        it.next();
    }
    k
}

fn word_right_src(s: &str, i: usize) -> usize {
    let mut k = i;
    let mut it = s[i..].char_indices().map(|(p, c)| (p + i, c)).peekable();
    while let Some(&(p, c)) = it.peek() {
        if !is_wordish(c) {
            break;
        }
        k = p + c.len_utf8();
        it.next();
    }
    while let Some(&(p, c)) = it.peek() {
        if is_wordish(c) || c == '\n' {
            if k == i && c == '\n' {
                k = p + 1;
            }
            break;
        }
        k = p + c.len_utf8();
        it.next();
    }
    k
}

/// Grapheme-aware movement inside display text (UTF-16 positions).
fn utf16_to_byte(s: &str, d: u32) -> usize {
    let mut u = 0u32;
    for (b, c) in s.char_indices() {
        if u >= d {
            return b;
        }
        u += c.len_utf16() as u32;
    }
    s.len()
}

fn byte_to_utf16(s: &str, b: usize) -> u32 {
    s[..b.min(s.len())].chars().map(|c| c.len_utf16() as u32).sum()
}

fn grapheme_left_d(s: &str, d: u32) -> u32 {
    use unicode_segmentation::UnicodeSegmentation;
    let b = utf16_to_byte(s, d);
    let prev = s[..b].grapheme_indices(true).next_back().map(|(i, _)| i).unwrap_or(0);
    byte_to_utf16(s, prev)
}

fn grapheme_right_d(s: &str, d: u32) -> u32 {
    use unicode_segmentation::UnicodeSegmentation;
    let b = utf16_to_byte(s, d);
    let next = s[b..].graphemes(true).next().map(|g| b + g.len()).unwrap_or(s.len());
    byte_to_utf16(s, next)
}

fn word_left_d(s: &str, d: u32) -> u32 {
    let b = utf16_to_byte(s, d);
    byte_to_utf16(s, word_left_src(s, b))
}

fn word_right_d(s: &str, d: u32) -> u32 {
    let b = utf16_to_byte(s, d);
    byte_to_utf16(s, word_right_src(s, b))
}

fn file_dialog(owner: HWND, save: bool, current: Option<&Path>) -> Option<PathBuf> {
    unsafe {
        let filters = [
            COMDLG_FILTERSPEC { pszName: w!("Markdown"), pszSpec: w!("*.md;*.markdown;*.mdown;*.mkd;*.mkdn;*.mdx") },
            COMDLG_FILTERSPEC { pszName: w!("Text"), pszSpec: w!("*.txt") },
            COMDLG_FILTERSPEC { pszName: w!("All files"), pszSpec: w!("*.*") },
        ];
        let dlg: IFileDialog = if save {
            CoCreateInstance::<_, IFileSaveDialog>(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?.cast().ok()?
        } else {
            CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?.cast().ok()?
        };
        let _ = dlg.SetFileTypes(&filters);
        if save {
            let _ = dlg.SetDefaultExtension(w!("md"));
        }
        if let Some(cur) = current {
            if let Some(name) = cur.file_name() {
                let w = wstr(&name.to_string_lossy());
                if save {
                    let _ = dlg.SetFileName(PCWSTR(w.as_ptr()));
                }
            }
            if let Some(dir) = cur.parent() {
                let w = wstr(&dir.to_string_lossy());
                if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(w.as_ptr()), None) {
                    let _ = dlg.SetFolder(&item);
                }
            }
        }
        dlg.Show(Some(owner)).ok()?;
        let item = dlg.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s.map(PathBuf::from)
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let Some(a) = app() else { return DefWindowProcW(hwnd, msg, wp, lp) };
        if msg == a.find_msg && a.find_msg != 0 {
            a.on_find_msg();
            return LRESULT(0);
        }
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                a.paint();
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_SIZE => {
                if let Some(rt) = &a.hrt {
                    let w = (lp.0 & 0xffff) as u32;
                    let h = ((lp.0 >> 16) & 0xffff) as u32;
                    let _ = rt.Resize(&D2D_SIZE_U { width: w.max(1), height: h.max(1) });
                }
                a.layout_viewport();
                a.invalidate();
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let dpi = (wp.0 & 0xffff) as f32;
                a.scale = dpi / 96.0;
                if let Some(rt) = &a.hrt {
                    rt.SetDpi(dpi, dpi);
                }
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                a.layout_viewport();
                a.invalidate();
                LRESULT(0)
            }
            WM_APP_SNAPSHOT => {
                if let Some(base) = std::env::var_os("BRISK_SNAPSHOT") {
                    let p = PathBuf::from(base).join(format!("snap-{}.png", wp.0));
                    a.snapshot(&p);
                }
                LRESULT(0)
            }
            WM_APP_IDLE => {
                let before = (a.view.doc_h, a.view.scroll);
                let more = a.view.idle(&a.gfx, &a.theme, 8.0);
                if (a.view.doc_h, a.view.scroll) != before {
                    a.invalidate();
                }
                if more {
                    let _ = PostMessageW(Some(hwnd), WM_APP_IDLE, WPARAM(0), LPARAM(0));
                }
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xffff) as i16 as f32;
                if key_down(VK_CONTROL) {
                    let z = a.view.zoom + if delta > 0.0 { 0.1 } else { -0.1 };
                    a.set_zoom((z * 10.0).round() / 10.0);
                } else {
                    let mut lines = 3u32;
                    let _ = SystemParametersInfoW(
                        SPI_GETWHEELSCROLLLINES,
                        0,
                        Some(&mut lines as *mut _ as _),
                        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                    );
                    let step = a.theme.style(El::Body).line_height * lines.clamp(1, 10) as f32;
                    a.view.scroll = (a.view.scroll - delta / 120.0 * step).clamp(0.0, a.view.max_scroll());
                    a.view.prepare(&a.gfx, &a.theme);
                }
                a.invalidate();
                LRESULT(0)
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let x = (lp.0 & 0xffff) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                let _ = SetFocus(Some(hwnd));
                a.on_mouse_down(x, y, msg == WM_LBUTTONDBLCLK);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let x = (lp.0 & 0xffff) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                a.on_mouse_move(x, y);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                a.on_mouse_up();
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                let menu = CreatePopupMenu().unwrap();
                append(menu, CMD_CUT, "Cu&t\tCtrl+X");
                append(menu, CMD_COPY, "&Copy\tCtrl+C");
                append(menu, CMD_COPY_PLAIN, "Copy as Plain &Text\tCtrl+Shift+C");
                append(menu, CMD_PASTE, "&Paste\tCtrl+V");
                sep(menu);
                append(menu, CMD_BOLD, "&Bold\tCtrl+B");
                append(menu, CMD_ITALIC, "&Italic\tCtrl+I");
                append(menu, CMD_LINK, "&Link...\tCtrl+K");
                sep(menu);
                append(menu, CMD_SELECT_ALL, "Select &All\tCtrl+A");
                append(
                    menu,
                    CMD_MODE,
                    if a.view.mode == Mode::Markdown { "Switch to &Mono\tCtrl+/" } else { "Switch to &Markdown\tCtrl+/" },
                );
                if !a.doc.has_sel() {
                    let _ = EnableMenuItem(menu, CMD_CUT as u32, MF_GRAYED);
                    let _ = EnableMenuItem(menu, CMD_COPY as u32, MF_GRAYED);
                    let _ = EnableMenuItem(menu, CMD_COPY_PLAIN as u32, MF_GRAYED);
                }
                let mut p = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
                let _ = ClientToScreen(hwnd, &mut p);
                let _ = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_RIGHTBUTTON, p.x, p.y, None, hwnd, None);
                let _ = DestroyMenu(menu);
                LRESULT(0)
            }
            WM_SETCURSOR => {
                if (lp.0 & 0xffff) as u32 == HTCLIENT {
                    let mut p = POINT::default();
                    let _ = GetCursorPos(&mut p);
                    let _ = ScreenToClient(hwnd, &mut p);
                    let sx = p.x as f32 / a.scale;
                    let on_bar = a.scrollbar_geom().map(|(bx, _, _)| sx >= bx - 2.0).unwrap_or(false);
                    let cursor = if a.in_status(p.y) || on_bar { IDC_ARROW } else { IDC_IBEAM };
                    let _ = SetCursor(LoadCursorW(None, cursor).ok());
                    return LRESULT(1);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                let ctrl = key_down(VK_CONTROL);
                let shift = key_down(VK_SHIFT);
                match vk {
                    VK_LEFT => a.horizontal(-1, ctrl, shift),
                    VK_RIGHT => a.horizontal(1, ctrl, shift),
                    VK_UP => {
                        if ctrl {
                            a.view.scroll = (a.view.scroll - a.theme.style(El::Body).line_height).max(0.0);
                            a.view.prepare(&a.gfx, &a.theme);
                            a.invalidate();
                        } else {
                            a.vertical(-1, shift, false)
                        }
                    }
                    VK_DOWN => {
                        if ctrl {
                            a.view.scroll = (a.view.scroll + a.theme.style(El::Body).line_height).min(a.view.max_scroll());
                            a.view.prepare(&a.gfx, &a.theme);
                            a.invalidate();
                        } else {
                            a.vertical(1, shift, false)
                        }
                    }
                    VK_PRIOR => a.vertical(-1, shift, true),
                    VK_NEXT => a.vertical(1, shift, true),
                    VK_HOME => a.home_end(false, ctrl, shift),
                    VK_END => a.home_end(true, ctrl, shift),
                    VK_BACK => a.backspace(ctrl),
                    VK_DELETE => a.delete_forward(ctrl),
                    VK_RETURN => a.enter(),
                    VK_TAB => {
                        if !ctrl {
                            a.tab(shift)
                        }
                    }
                    VK_ESCAPE => {
                        if !a.finds.is_empty() {
                            a.finds.clear();
                            a.find_cur = None;
                            a.invalidate();
                        } else if a.doc.has_sel() {
                            let h = a.doc.head;
                            a.doc.set_sel(h, h);
                            a.invalidate();
                        }
                    }
                    VK_INSERT => {
                        if ctrl {
                            a.command(CMD_COPY);
                        } else if shift {
                            a.command(CMD_PASTE);
                        }
                    }
                    _ => return DefWindowProcW(hwnd, msg, wp, lp),
                }
                a.update_ime_pos();
                LRESULT(0)
            }
            WM_CHAR => {
                let c = wp.0 as u16;
                if (0xD800..0xDC00).contains(&c) {
                    a.high_surrogate = Some(c);
                    return LRESULT(0);
                }
                let s = if (0xDC00..0xE000).contains(&c) {
                    match a.high_surrogate.take() {
                        Some(h) => String::from_utf16_lossy(&[h, c]),
                        None => return LRESULT(0),
                    }
                } else {
                    if c < 0x20 || c == 0x7f {
                        return LRESULT(0);
                    }
                    String::from_utf16_lossy(&[c])
                };
                a.insert_text(&s, EditKind::Typing);
                LRESULT(0)
            }
            WM_IME_STARTCOMPOSITION => {
                a.update_ime_pos();
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_COMMAND => {
                let id = (wp.0 & 0xffff) as u16;
                a.command(id);
                LRESULT(0)
            }
            WM_INITMENUPOPUP => {
                a.init_menu_popup(HMENU(wp.0 as _));
                LRESULT(0)
            }
            WM_TIMER => {
                match wp.0 {
                    TIMER_CARET => {
                        a.caret_on = !a.caret_on;
                        a.invalidate();
                    }
                    TIMER_FILE => a.check_disk(),
                    TIMER_STATUS => {
                        a.status_msg = None;
                        let _ = KillTimer(Some(hwnd), TIMER_STATUS);
                        a.invalidate();
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_SETFOCUS => {
                a.caret_on = true;
                a.restart_blink();
                a.invalidate();
                LRESULT(0)
            }
            WM_KILLFOCUS => {
                let _ = KillTimer(Some(hwnd), TIMER_CARET);
                a.caret_on = false;
                a.invalidate();
                LRESULT(0)
            }
            WM_ACTIVATE => {
                if (wp.0 & 0xffff) != 0 {
                    a.check_disk();
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_DROPFILES => {
                let hdrop = HDROP(wp.0 as _);
                let n = DragQueryFileW(hdrop, 0xFFFFFFFF, None);
                for i in 0..n {
                    let len = DragQueryFileW(hdrop, i, None);
                    let mut buf = vec![0u16; len as usize + 1];
                    DragQueryFileW(hdrop, i, Some(&mut buf));
                    let p = PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
                    if i == 0 && a.doc.path.is_none() && !a.doc.dirty && a.doc.text.is_empty() {
                        a.open_path(&p);
                    } else {
                        spawn_self(&[&p.to_string_lossy()]);
                    }
                }
                DragFinish(hdrop);
                LRESULT(0)
            }
            WM_UAHDRAWMENU => {
                uah_draw_menubar(hwnd, lp, &a.menu_colors());
                LRESULT(1)
            }
            WM_UAHDRAWMENUITEM => {
                let mc = a.menu_colors();
                uah_draw_menuitem(lp, &mc, |id| id == CMD_MODE as u32);
                LRESULT(1)
            }
            WM_NCPAINT | WM_NCACTIVATE => {
                let r = DefWindowProcW(hwnd, msg, wp, lp);
                paint_menubar_line(hwnd, &a.menu_colors().bg);
                r
            }
            WM_CLOSE => {
                if a.confirm_discard() {
                    a.save_placement();
                    a.settings.save();
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

pub struct Opts {
    pub file: Option<PathBuf>,
    pub bench: Option<PathBuf>,
}

pub fn run(opts: Opts) -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let settings = Settings::load();
        let theme = load_theme(&settings.theme);
        let mut doc = match &opts.file {
            Some(p) => match Doc::load(p) {
                Ok(d) => d,
                Err(_) => {
                    let mut d = Doc::new();
                    d.path = Some(p.clone());
                    d
                }
            },
            None => Doc::new(),
        };
        if doc.path.is_none() {
            doc.crlf = true;
        }
        let gfx = Gfx::new()?;
        let hinst = GetModuleHandleW(None)?;
        let bg = theme.style(El::Body).background.unwrap_or(Color::rgb(20, 22, 26));
        let class = w!("BriskMarkdownWindow");
        let icon = LoadIconW(Some(hinst.into()), PCWSTR(1 as _)).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_IBEAM)?,
            hbrBackground: CreateSolidBrush(COLORREF(bg.colorref())),
            hIcon: icon,
            hIconSm: icon,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let (menu, file_menu, view_menu) = build_menus();
        let dpi = GetDpiForSystem() as f32;
        let (x, y, w, h, maximized) = match settings.placement {
            Some((x, y, w, h, m)) if w > 200 && h > 150 => (x, y, w, h, m),
            _ => (CW_USEDEFAULT, CW_USEDEFAULT, (1000.0 * dpi / 96.0) as i32, (820.0 * dpi / 96.0) as i32, false),
        };
        let title = wstr("BriskMarkdown");
        let hwnd = CreateWindowExW(
            WS_EX_ACCEPTFILES,
            class,
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            w,
            h,
            None,
            Some(menu),
            Some(hinst.into()),
            None,
        )?;
        let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
        let mut view = View::new();
        view.zoom = settings.zoom;
        view.wrap = settings.wrap;
        let find_msg = RegisterWindowMessageW(w!("commdlg_FindReplace"));
        let app = Box::new(App {
            hwnd,
            gfx,
            hrt: None,
            theme,
            doc,
            view,
            scale,
            menu,
            view_menu,
            file_menu,
            accel: build_accels(),
            sticky_x: None,
            caret_on: true,
            drag: Drag::None,
            status_msg: None,
            finds: Vec::new(),
            find_cur: None,
            find_text: String::new(),
            find_case: false,
            find_word: false,
            find_hwnd: HWND::default(),
            fr: Box::new(FINDREPLACEW { lStructSize: std::mem::size_of::<FINDREPLACEW>() as u32, ..Default::default() }),
            fr_find: Box::new([0u16; 256]),
            fr_repl: Box::new([0u16; 256]),
            find_msg,
            settings,
            bench: opts.bench,
            painted: false,
            clicks: (Instant::now(), 0, 0, 0),
            disk_note: false,
            high_surrogate: None,
            user_themes: Vec::new(),
            scroll_hover: false,
            last_blank_line: false,
        });
        APP = Box::into_raw(app);
        let a = &mut *APP;
        if let Some(p) = a.doc.path.clone() {
            if p.exists() {
                a.settings.add_recent(&p);
            }
        }
        a.apply_window_theme();
        a.update_title();
        a.layout_viewport();
        a.rebuild();
        let _ = ShowWindow(hwnd, if maximized { SW_SHOWMAXIMIZED } else { SW_SHOW });
        let _ = UpdateWindow(hwnd);
        SetTimer(Some(hwnd), TIMER_FILE, 1000, None);
        a.restart_blink();
        let accel = a.accel;
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let fh = (*APP).find_hwnd;
            if !fh.is_invalid() && IsDialogMessageW(fh, &msg).as_bool() {
                continue;
            }
            if TranslateAcceleratorW(hwnd, accel, &msg) != 0 {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}
