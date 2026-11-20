//! Windows plumbing: dark title bar and menus, clipboard, a small input box, settings, file
//! association, and opening things with the shell.

use crate::gfx::wstr;
use crate::theme::Color;
use std::path::{Path, PathBuf};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

// ------------------------------------------------------------------ dark mode

/// Title bar colours (Windows 11 caption colour; dark mode flag on Windows 10).
pub fn style_title_bar(hwnd: HWND, dark: bool, caption: &Color, text: &Color) {
    unsafe {
        let v: BOOL = dark.into();
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &v as *const _ as _, 4);
        let c = caption.colorref();
        let t = text.colorref();
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(35), &c as *const _ as _, 4);
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(36), &t as *const _ as _, 4);
    }
}

/// Popup menus follow the theme (uxtheme's undocumented but long-stable SetPreferredAppMode).
pub fn set_menu_dark(dark: bool) {
    unsafe {
        let Ok(h) = LoadLibraryW(w!("uxtheme.dll")) else { return };
        if let Some(f) = GetProcAddress(h, PCSTR(135 as *const u8)) {
            let set: extern "system" fn(i32) -> i32 = std::mem::transmute(f);
            set(if dark { 2 } else { 3 });
        }
        if let Some(f) = GetProcAddress(h, PCSTR(136 as *const u8)) {
            let flush: extern "system" fn() = std::mem::transmute(f);
            flush();
        }
    }
}

pub const WM_UAHDRAWMENU: u32 = 0x0091;
pub const WM_UAHDRAWMENUITEM: u32 = 0x0092;

#[repr(C)]
pub struct UahMenu {
    pub hmenu: HMENU,
    pub hdc: HDC,
    pub flags: u32,
}

#[repr(C)]
pub struct UahMenuItem {
    pub position: i32,
    pub metrics: [u32; 8],
    pub popup: [u32; 5],
}

#[repr(C)]
pub struct UahDrawMenuItem {
    pub dis: DRAWITEMSTRUCT,
    pub um: UahMenu,
    pub umi: UahMenuItem,
}

pub struct MenuColors {
    pub bg: Color,
    pub text: Color,
    pub hot: Color,
    pub accent_bg: Color,
}

fn window_rect_rel(hwnd: HWND) -> Option<RECT> {
    unsafe {
        let mut mbi = MENUBARINFO { cbSize: std::mem::size_of::<MENUBARINFO>() as u32, ..Default::default() };
        GetMenuBarInfo(hwnd, OBJID_MENU, 0, &mut mbi).ok()?;
        let mut wr = RECT::default();
        GetWindowRect(hwnd, &mut wr).ok()?;
        let mut r = mbi.rcBar;
        r.left -= wr.left;
        r.right -= wr.left;
        r.top -= wr.top;
        r.bottom -= wr.top;
        Some(r)
    }
}

pub fn uah_draw_menubar(hwnd: HWND, lp: LPARAM, colors: &MenuColors) {
    unsafe {
        let um = &*(lp.0 as *const UahMenu);
        if let Some(mut r) = window_rect_rel(hwnd) {
            r.bottom += 1;
            let br = CreateSolidBrush(COLORREF(colors.bg.colorref()));
            FillRect(um.hdc, &r, br);
            let _ = DeleteObject(br.into());
        }
    }
}

pub fn uah_draw_menuitem(lp: LPARAM, colors: &MenuColors, is_mode_item: impl Fn(u32) -> bool) {
    unsafe {
        let di = &mut *(lp.0 as *mut UahDrawMenuItem);
        let mut buf = [0u16; 256];
        let mut mii = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_STRING | MIIM_ID,
            dwTypeData: PWSTR(buf.as_mut_ptr()),
            cch: 255,
            ..Default::default()
        };
        let _ = GetMenuItemInfoW(di.um.hmenu, di.umi.position as u32, true, &mut mii);
        let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
        let state = di.dis.itemState.0;
        let hot = state & (ODS_HOTLIGHT.0 | ODS_SELECTED.0) != 0;
        let grayed = state & (ODS_GRAYED.0 | ODS_DISABLED.0 | ODS_INACTIVE.0) != 0;
        let mode = is_mode_item(mii.wID);
        let bg = if hot {
            colors.hot
        } else if mode {
            colors.accent_bg
        } else {
            colors.bg
        };
        let br = CreateSolidBrush(COLORREF(bg.colorref()));
        let mut r = di.dis.rcItem;
        if mode {
            // a pill inside the bar
            let fill = CreateSolidBrush(COLORREF(colors.bg.colorref()));
            FillRect(di.um.hdc, &r, fill);
            let _ = DeleteObject(fill.into());
            r.top += 3;
            r.bottom -= 3;
            r.left += 2;
            r.right -= 2;
            let old = SelectObject(di.um.hdc, br.into());
            let pen = CreatePen(PS_SOLID, 1, COLORREF(bg.colorref()));
            let oldp = SelectObject(di.um.hdc, pen.into());
            let _ = RoundRect(di.um.hdc, r.left, r.top, r.right, r.bottom, 10, 10);
            SelectObject(di.um.hdc, oldp);
            SelectObject(di.um.hdc, old);
            let _ = DeleteObject(pen.into());
        } else {
            FillRect(di.um.hdc, &r, br);
        }
        let _ = DeleteObject(br.into());
        let mut text = colors.text;
        if grayed {
            text = text.mix(&colors.bg, 0.45);
        }
        SetBkMode(di.um.hdc, TRANSPARENT);
        SetTextColor(di.um.hdc, COLORREF(text.colorref()));
        let mut flags = DT_CENTER | DT_SINGLELINE | DT_VCENTER;
        if state & ODS_NOACCEL.0 != 0 {
            flags |= DT_HIDEPREFIX;
        }
        let mut rr = di.dis.rcItem;
        DrawTextW(di.um.hdc, &mut buf[..len], &mut rr, flags);
    }
}

/// Paint over the light line Windows draws under a custom menu bar.
pub fn paint_menubar_line(hwnd: HWND, color: &Color) {
    unsafe {
        let mut mbi = MENUBARINFO { cbSize: std::mem::size_of::<MENUBARINFO>() as u32, ..Default::default() };
        if GetMenuBarInfo(hwnd, OBJID_MENU, 0, &mut mbi).is_err() {
            return;
        }
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let mut pts = [POINT { x: rc.left, y: rc.top }, POINT { x: rc.right, y: rc.bottom }];
        MapWindowPoints(Some(hwnd), None, &mut pts);
        let mut wr = RECT::default();
        let _ = GetWindowRect(hwnd, &mut wr);
        let line = RECT { left: pts[0].x - wr.left, top: pts[0].y - wr.top - 1, right: pts[1].x - wr.left, bottom: pts[0].y - wr.top };
        let hdc = GetWindowDC(Some(hwnd));
        let br = CreateSolidBrush(COLORREF(color.colorref()));
        FillRect(hdc, &line, br);
        let _ = DeleteObject(br.into());
        ReleaseDC(Some(hwnd), hdc);
    }
}

// ------------------------------------------------------------------ clipboard

pub fn clipboard_set(hwnd: HWND, text: &str) {
    let w: Vec<u16> = text.replace('\n', "\r\n").encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) {
            let p = GlobalLock(h) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                let _ = GlobalUnlock(h);
                let _ = SetClipboardData(13, Some(HANDLE(h.0)));
            }
        }
        let _ = CloseClipboard();
    }
}

pub fn clipboard_get(hwnd: HWND) -> Option<String> {
    unsafe {
        OpenClipboard(Some(hwnd)).ok()?;
        let mut out = None;
        if let Ok(h) = GetClipboardData(13) {
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u16;
            if !p.is_null() {
                let mut n = 0;
                while *p.add(n) != 0 {
                    n += 1;
                }
                out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
                let _ = GlobalUnlock(g);
            }
        }
        let _ = CloseClipboard();
        out.map(|s| s.replace("\r\n", "\n").replace('\r', "\n"))
    }
}

// ------------------------------------------------------------------ input box

struct PromptState {
    edit: HWND,
    result: Option<String>,
    done: bool,
    bg: HBRUSH,
    edit_bg: HBRUSH,
    colors: (Color, Color, Color),
}

static mut PROMPT: Option<PromptState> = None;

extern "system" fn prompt_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        #[allow(static_mut_refs)]
        let Some(st) = PROMPT.as_mut() else {
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        match msg {
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
                let hdc = HDC(wp.0 as _);
                SetTextColor(hdc, COLORREF(st.colors.1.colorref()));
                let (bgc, br) = if msg == WM_CTLCOLOREDIT { (st.colors.2, st.edit_bg) } else { (st.colors.0, st.bg) };
                SetBkColor(hdc, COLORREF(bgc.colorref()));
                LRESULT(br.0 as isize)
            }
            WM_COMMAND => {
                let id = (wp.0 & 0xffff) as i32;
                if id == 1 {
                    let n = GetWindowTextLengthW(st.edit);
                    let mut buf = vec![0u16; n as usize + 1];
                    GetWindowTextW(st.edit, &mut buf);
                    st.result = Some(String::from_utf16_lossy(&buf[..n as usize]));
                    st.done = true;
                } else if id == 2 {
                    st.done = true;
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                st.done = true;
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// A small modal input box. Returns None on cancel.
pub fn prompt(owner: HWND, title: &str, label: &str, initial: &str, colors: (Color, Color, Color), dpi: f32) -> Option<String> {
    unsafe {
        let hinst = GetModuleHandleW(None).ok()?;
        let class = w!("BriskPrompt");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(prompt_proc),
            hInstance: hinst.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let s = |v: f32| (v * dpi) as i32;
        let (w, h) = (s(380.0), s(150.0));
        let mut orc = RECT::default();
        let _ = GetWindowRect(owner, &mut orc);
        let x = orc.left + ((orc.right - orc.left) - w) / 2;
        let y = orc.top + s(120.0);
        let bg = CreateSolidBrush(COLORREF(colors.0.colorref()));
        let edit_bg = CreateSolidBrush(COLORREF(colors.2.colorref()));
        PROMPT = Some(PromptState { edit: HWND::default(), result: None, done: false, bg, edit_bg, colors });
        let tw = wstr(title);
        let hwnd = CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            class,
            PCWSTR(tw.as_ptr()),
            WS_POPUP | WS_CAPTION | WS_SYSMENU,
            x,
            y,
            w,
            h,
            Some(owner),
            None,
            Some(hinst.into()),
            None,
        )
        .ok()?;
        let _ = SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, bg.0 as isize);
        style_title_bar(hwnd, colors.0.luminance() < 0.45, &colors.0, &colors.1);
        let font = CreateFontW(
            -s(14.0),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        );
        let lw = wstr(label);
        let lab = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            PCWSTR(lw.as_ptr()),
            WS_CHILD | WS_VISIBLE,
            s(16.0),
            s(14.0),
            w - s(32.0),
            s(20.0),
            Some(hwnd),
            None,
            Some(hinst.into()),
            None,
        )
        .ok()?;
        let iw = wstr(initial);
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            PCWSTR(iw.as_ptr()),
            WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            s(16.0),
            s(40.0),
            w - s(48.0),
            s(26.0),
            Some(hwnd),
            None,
            Some(hinst.into()),
            None,
        )
        .ok()?;
        let ok = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!("OK"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
            w - s(200.0),
            s(78.0),
            s(80.0),
            s(28.0),
            Some(hwnd),
            Some(HMENU(1 as _)),
            Some(hinst.into()),
            None,
        )
        .ok()?;
        let cancel = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!("Cancel"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            w - s(112.0),
            s(78.0),
            s(80.0),
            s(28.0),
            Some(hwnd),
            Some(HMENU(2 as _)),
            Some(hinst.into()),
            None,
        )
        .ok()?;
        for c in [lab, edit, ok, cancel] {
            SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        }
        #[allow(static_mut_refs)]
        if let Some(p) = PROMPT.as_mut() {
            p.edit = edit;
        }
        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetFocus(Some(edit));
        SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        let mut msg = MSG::default();
        loop {
            #[allow(static_mut_refs)]
            if PROMPT.as_ref().map(|p| p.done).unwrap_or(true) {
                break;
            }
            if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                PostQuitMessage(msg.wParam.0 as i32);
                break;
            }
            if msg.message == WM_KEYDOWN && msg.wParam.0 == VK_ESCAPE.0 as usize {
                #[allow(static_mut_refs)]
                if let Some(p) = PROMPT.as_mut() {
                    p.done = true;
                }
                continue;
            }
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let _ = EnableWindow(owner, true);
        let _ = DestroyWindow(hwnd);
        let _ = SetForegroundWindow(owner);
        let _ = DeleteObject(font.into());
        #[allow(static_mut_refs)]
        let st = PROMPT.take()?;
        let _ = DeleteObject(st.bg.into());
        let _ = DeleteObject(st.edit_bg.into());
        st.result
    }
}

// ------------------------------------------------------------------ settings

pub fn app_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir());
    base.join("BriskMarkdown")
}

/// Settings live next to the exe when a `BriskMarkdown.ini` is there (portable use), otherwise in
/// %APPDATA%\BriskMarkdown.
pub fn settings_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let p = exe.with_file_name("BriskMarkdown.ini");
        if p.exists() {
            return p;
        }
    }
    app_dir().join("settings.ini")
}

pub fn themes_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if exe.with_file_name("BriskMarkdown.ini").exists() {
            return exe.with_file_name("themes");
        }
    }
    app_dir().join("themes")
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub theme: String,
    pub zoom: f32,
    pub wrap: bool,
    pub status: bool,
    pub placement: Option<(i32, i32, i32, i32, bool)>,
    pub recent: Vec<String>,
    pub offered_default: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { theme: "Dark".into(), zoom: 1.0, wrap: true, status: true, placement: None, recent: Vec::new(), offered_default: false }
    }
}

impl Settings {
    pub fn load() -> Settings {
        let mut s = Settings::default();
        let Ok(text) = std::fs::read_to_string(settings_path()) else { return s };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "theme" => s.theme = v.to_string(),
                "zoom" => s.zoom = v.parse::<f32>().unwrap_or(1.0).clamp(0.5, 3.0),
                "wrap" => s.wrap = v != "0",
                "status" => s.status = v != "0",
                "offered_default" => s.offered_default = v == "1",
                "placement" => {
                    let p: Vec<i32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if p.len() == 5 {
                        s.placement = Some((p[0], p[1], p[2], p[3], p[4] != 0));
                    }
                }
                "recent" => {
                    if !v.is_empty() && s.recent.len() < 10 {
                        s.recent.push(v.to_string());
                    }
                }
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        let mut out = String::new();
        out.push_str(&format!(
            "theme={}\nzoom={:.2}\nwrap={}\nstatus={}\noffered_default={}\n",
            self.theme, self.zoom, self.wrap as u8, self.status as u8, self.offered_default as u8
        ));
        if let Some((x, y, w, h, m)) = self.placement {
            out.push_str(&format!("placement={x},{y},{w},{h},{}\n", m as u8));
        }
        for r in &self.recent {
            out.push_str(&format!("recent={r}\n"));
        }
        let p = settings_path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, out);
    }

    pub fn add_recent(&mut self, path: &Path) {
        let s = path.to_string_lossy().to_string();
        self.recent.retain(|r| !r.eq_ignore_ascii_case(&s));
        self.recent.insert(0, s);
        self.recent.truncate(10);
    }
}

// ------------------------------------------------------------------ shell

pub fn shell_open(target: &str) {
    let w = wstr(target);
    unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(w.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

pub fn spawn_self(args: &[&str]) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).args(args).spawn();
    }
}

pub fn reveal_in_explorer(path: &Path) {
    let arg = format!("/select,\"{}\"", path.display());
    let w = wstr(&arg);
    unsafe {
        ShellExecuteW(None, w!("open"), w!("explorer.exe"), PCWSTR(w.as_ptr()), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

// ------------------------------------------------------------------ file association

const PROGID: &str = "BriskMarkdown.md";
pub const EXTENSIONS: &[&str] = &[".md", ".markdown", ".mdown", ".mkd", ".mkdn", ".mdx"];

fn reg_set(root: HKEY, sub: &str, name: Option<&str>, value: &str) -> bool {
    unsafe {
        let mut key = HKEY::default();
        let s = wstr(sub);
        if RegCreateKeyExW(root, PCWSTR(s.as_ptr()), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None)
            .is_err()
        {
            return false;
        }
        let n = name.map(wstr);
        let v = wstr(value);
        let bytes = std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2);
        let r = RegSetValueExW(key, n.as_ref().map(|n| PCWSTR(n.as_ptr())).unwrap_or(PCWSTR::null()), None, REG_SZ, Some(bytes));
        let _ = RegCloseKey(key);
        r.is_ok()
    }
}

fn reg_delete_tree(root: HKEY, sub: &str) {
    unsafe {
        let s = wstr(sub);
        let _ = RegDeleteTreeW(root, PCWSTR(s.as_ptr()));
        let _ = RegDeleteKeyW(root, PCWSTR(s.as_ptr()));
    }
}

fn reg_delete_value(root: HKEY, sub: &str, name: &str) {
    unsafe {
        let s = wstr(sub);
        let n = wstr(name);
        let _ = RegDeleteKeyValueW(root, PCWSTR(s.as_ptr()), PCWSTR(n.as_ptr()));
    }
}

/// Register BriskMarkdown for Markdown files (current user only). Windows then lists it under
/// "Open with" and in Settings > Default apps, where the user confirms it as the default.
pub fn register() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe = exe.to_string_lossy().to_string();
    let cmd = format!("\"{exe}\" \"%1\"");
    let icon = format!("\"{exe}\",0");
    let mut ok = true;
    ok &= reg_set(HKEY_CURRENT_USER, &format!("Software\\Classes\\{PROGID}"), None, "Markdown document");
    ok &= reg_set(HKEY_CURRENT_USER, &format!("Software\\Classes\\{PROGID}\\DefaultIcon"), None, &icon);
    ok &= reg_set(HKEY_CURRENT_USER, &format!("Software\\Classes\\{PROGID}\\shell\\open\\command"), None, &cmd);
    ok &= reg_set(HKEY_CURRENT_USER, "Software\\Classes\\Applications\\BriskMarkdown.exe\\shell\\open\\command", None, &cmd);
    ok &= reg_set(HKEY_CURRENT_USER, "Software\\Classes\\Applications\\BriskMarkdown.exe", Some("FriendlyAppName"), "BriskMarkdown");
    ok &= reg_set(HKEY_CURRENT_USER, "Software\\BriskMarkdown\\Capabilities", Some("ApplicationName"), "BriskMarkdown");
    ok &= reg_set(
        HKEY_CURRENT_USER,
        "Software\\BriskMarkdown\\Capabilities",
        Some("ApplicationDescription"),
        "A fast, lightweight Markdown notepad.",
    );
    for ext in EXTENSIONS {
        ok &= reg_set(HKEY_CURRENT_USER, &format!("Software\\Classes\\{ext}\\OpenWithProgids"), Some(PROGID), "");
        ok &= reg_set(HKEY_CURRENT_USER, "Software\\Classes\\Applications\\BriskMarkdown.exe\\SupportedTypes", Some(ext), "");
        ok &= reg_set(HKEY_CURRENT_USER, "Software\\BriskMarkdown\\Capabilities\\FileAssociations", Some(ext), PROGID);
    }
    ok &= reg_set(HKEY_CURRENT_USER, "Software\\RegisteredApplications", Some("BriskMarkdown"), "Software\\BriskMarkdown\\Capabilities");
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
    ok
}

pub fn unregister() {
    reg_delete_tree(HKEY_CURRENT_USER, &format!("Software\\Classes\\{PROGID}"));
    reg_delete_tree(HKEY_CURRENT_USER, "Software\\Classes\\Applications\\BriskMarkdown.exe");
    reg_delete_tree(HKEY_CURRENT_USER, "Software\\BriskMarkdown");
    for ext in EXTENSIONS {
        reg_delete_value(HKEY_CURRENT_USER, &format!("Software\\Classes\\{ext}\\OpenWithProgids"), PROGID);
    }
    reg_delete_value(HKEY_CURRENT_USER, "Software\\RegisteredApplications", "BriskMarkdown");
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
}

/// Open the Settings page where the user picks BriskMarkdown as the default for .md files.
pub fn open_default_apps_settings() {
    shell_open("ms-settings:defaultapps?registeredAppUser=BriskMarkdown");
}

pub fn key_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vk.0 as i32) < 0 }
}
