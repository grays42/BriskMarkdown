//! Themes: a small CSS subset (see docs/themes.md) resolved into per-element styles.
//!
//! A theme is a list of rules `selector, selector { property: value; }`. Selectors are element
//! keys (`h1`, `pre`, `.callout.note`, `.tok-keyword`, ...), not general CSS selectors. Custom
//! properties (`--name: value`) and `var(--name, fallback)` work anywhere. `@import "dark";`
//! starts from a built-in theme. Unknown selectors and properties are collected as warnings
//! instead of failing, so a theme never stops the app from opening.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
    }
    pub const TRANSPARENT: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    pub fn luminance(&self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }
    pub fn key(&self) -> u32 {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
        (c(self.r) << 24) | (c(self.g) << 16) | (c(self.b) << 8) | c(self.a)
    }
    pub fn with_alpha(&self, a: f32) -> Color {
        Color { a, ..*self }
    }
    pub fn mix(&self, other: &Color, t: f32) -> Color {
        Color {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }
    pub fn colorref(&self) -> u32 {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
        c(self.r) | (c(self.g) << 8) | (c(self.b) << 16)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Len {
    Px(f32),
    Em(f32),
}

impl Len {
    pub fn px(&self, em: f32) -> f32 {
        match *self {
            Len::Px(v) => v,
            Len::Em(v) => v * em,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeight {
    Mult(f32),
    Px(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    pub width: f32,
    pub color: Color,
}

/// Declared properties for one element, after the cascade. `None` means "not set here".
#[derive(Clone, Debug, Default)]
pub struct Props {
    pub font_family: Option<Vec<String>>,
    pub font_size: Option<Len>,
    pub font_weight: Option<u16>,
    pub italic: Option<bool>,
    pub color: Option<Color>,
    pub background: Option<Color>,
    pub line_height: Option<LineHeight>,
    pub border_top: Option<Border>,
    pub border_right: Option<Border>,
    pub border_bottom: Option<Border>,
    pub border_left: Option<Border>,
    pub radius: Option<Len>,
    pub padding: [Option<Len>; 4],
    pub margin: [Option<Len>; 4],
    pub max_width: Option<Len>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub caret_color: Option<Color>,
    pub uppercase: Option<bool>,
    pub letter_spacing: Option<Len>,
}

impl Props {
    fn merge_from(&mut self, o: &Props) {
        macro_rules! m {
            ($($f:ident),*) => { $( if o.$f.is_some() { self.$f = o.$f.clone(); } )* };
        }
        m!(
            font_family,
            font_size,
            font_weight,
            italic,
            color,
            background,
            line_height,
            border_top,
            border_right,
            border_bottom,
            border_left,
            radius,
            max_width,
            underline,
            strike,
            caret_color,
            uppercase,
            letter_spacing
        );
        for i in 0..4 {
            if o.padding[i].is_some() {
                self.padding[i] = o.padding[i];
            }
            if o.margin[i].is_some() {
                self.margin[i] = o.margin[i];
            }
        }
    }
}

/// Every element a theme can style. The CSS selector for each is in `El::selectors`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum El {
    Body,
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
    P,
    A,
    Strong,
    Em,
    Del,
    Code,
    Pre,
    CodeLang,
    Blockquote,
    Li,
    Marker,
    Table,
    Th,
    Td,
    Hr,
    Math,
    MathBlock,
    Kbd,
    Mark,
    Sup,
    Selection,
    Mono,
    Status,
    StatusButton,
    Menubar,
    FrontMatter,
    Html,
    Task,
    TaskDone,
    Footnote,
    Note,
    Tip,
    Important,
    Warning,
    Caution,
    CalloutTitle,
    Scrollbar,
    Find,
    FindCurrent,
    Image,
    WikiLink,
    Diagram,
    TokKeyword,
    TokString,
    TokComment,
    TokNumber,
    TokType,
    TokFunction,
    TokConstant,
    TokProperty,
    TokTag,
    TokMeta,
    TokInserted,
    TokDeleted,
    TokVariable,
}

pub const EL_COUNT: usize = El::TokVariable as usize + 1;

impl El {
    pub fn all() -> impl Iterator<Item = El> {
        (0..EL_COUNT as u8).map(|i| unsafe { std::mem::transmute::<u8, El>(i) })
    }
    pub fn selectors(self) -> &'static [&'static str] {
        use El::*;
        match self {
            Body => &[":root", "body", "html"],
            H1 => &["h1"],
            H2 => &["h2"],
            H3 => &["h3"],
            H4 => &["h4"],
            H5 => &["h5"],
            H6 => &["h6"],
            P => &["p"],
            A => &["a"],
            Strong => &["strong", "b"],
            Em => &["em", "i"],
            Del => &["del", "s"],
            Code => &["code"],
            Pre => &["pre", "precode"],
            CodeLang => &[".code-lang"],
            Blockquote => &["blockquote"],
            Li => &["li", "ul", "ol"],
            Marker => &["li::marker", "::marker"],
            Table => &["table"],
            Th => &["th"],
            Td => &["td"],
            Hr => &["hr"],
            Math => &[".math"],
            MathBlock => &[".math-block"],
            Kbd => &["kbd"],
            Mark => &["mark"],
            Sup => &["sup", "sub"],
            Selection => &["::selection"],
            Mono => &[".mono"],
            Status => &[".status"],
            StatusButton => &[".status-button"],
            Menubar => &[".menubar"],
            FrontMatter => &[".front-matter"],
            Html => &[".html"],
            Task => &[".task"],
            TaskDone => &[".task-done"],
            Footnote => &[".footnote"],
            Note => &[".callout.note", ".note"],
            Tip => &[".callout.tip", ".tip"],
            Important => &[".callout.important", ".important"],
            Warning => &[".callout.warning", ".warning"],
            Caution => &[".callout.caution", ".caution"],
            CalloutTitle => &[".callout-title"],
            Scrollbar => &[".scrollbar"],
            Find => &[".find"],
            FindCurrent => &[".find-current"],
            Image => &[".image"],
            WikiLink => &[".wikilink"],
            Diagram => &[".diagram"],
            TokKeyword => &[".tok-keyword"],
            TokString => &[".tok-string"],
            TokComment => &[".tok-comment"],
            TokNumber => &[".tok-number"],
            TokType => &[".tok-type"],
            TokFunction => &[".tok-function"],
            TokConstant => &[".tok-constant"],
            TokProperty => &[".tok-property"],
            TokTag => &[".tok-tag"],
            TokMeta => &[".tok-meta"],
            TokInserted => &[".tok-inserted"],
            TokDeleted => &[".tok-deleted"],
            TokVariable => &[".tok-variable"],
        }
    }
    /// Block-level elements resolve against the body; inline ones overlay their container.
    pub fn is_block(self) -> bool {
        use El::*;
        matches!(
            self,
            Body | H1
                | H2
                | H3
                | H4
                | H5
                | H6
                | P
                | Pre
                | Blockquote
                | Li
                | Table
                | Th
                | Td
                | Hr
                | MathBlock
                | Mono
                | Status
                | Menubar
                | FrontMatter
                | Html
                | Footnote
                | Note
                | Tip
                | Important
                | Warning
                | Caution
                | CalloutTitle
                | Diagram
        )
    }
}

/// A fully resolved block style: everything a layout needs, in DIPs at 100% zoom.
#[derive(Clone, Debug)]
pub struct Style {
    pub families: Vec<String>,
    pub size: f32,
    pub weight: u16,
    pub italic: bool,
    pub color: Color,
    pub background: Option<Color>,
    pub line_height: f32, // absolute DIPs
    pub border_top: Option<Border>,
    pub border_right: Option<Border>,
    pub border_bottom: Option<Border>,
    pub border_left: Option<Border>,
    pub radius: f32,
    pub padding: [f32; 4],
    pub margin: [f32; 4],
    pub underline: bool,
    pub strike: bool,
    #[allow(dead_code)]
    pub uppercase: bool,
    #[allow(dead_code)]
    pub letter_spacing: f32,
}

pub struct Theme {
    pub name: String,
    pub props: Vec<Props>,
    pub styles: Vec<Style>,
    pub max_width: f32,
    pub caret: Color,
    pub dark: bool,
    pub warnings: Vec<String>,
}

pub const BUILTIN: &[(&str, &str)] = &[
    ("Dark", include_str!("../themes/dark.css")),
    ("Light", include_str!("../themes/light.css")),
    ("Sepia", include_str!("../themes/sepia.css")),
];

/// The base every theme starts from, so a user theme only needs to say what differs.
const BASE: &str = include_str!("../themes/base.css");

fn builtin_source(name: &str) -> Option<&'static str> {
    BUILTIN.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, s)| *s)
}

impl Theme {
    pub fn builtin(name: &str) -> Theme {
        let src = builtin_source(name).unwrap_or(BUILTIN[0].1);
        let name = BUILTIN.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(n, _)| *n).unwrap_or("Dark");
        Theme::from_css(name, src)
    }

    pub fn from_css(name: &str, css: &str) -> Theme {
        let mut sheet = Sheet::default();
        sheet.load(BASE, 0);
        sheet.load(css, 0);
        sheet.resolve(name)
    }

    pub fn style(&self, el: El) -> &Style {
        &self.styles[el as usize]
    }
    pub fn props(&self, el: El) -> &Props {
        &self.props[el as usize]
    }
}

#[derive(Default)]
struct Sheet {
    vars: HashMap<String, String>,
    rules: Vec<(String, Vec<(String, String)>)>, // normalized selector, declarations (raw values)
    warnings: Vec<String>,
}

fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        match rest[i + 2..].find("*/") {
            Some(j) => rest = &rest[i + 2 + j + 2..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

impl Sheet {
    fn load(&mut self, css: &str, depth: u32) {
        let css = strip_comments(css);
        let mut rest: &str = &css;
        loop {
            rest = rest.trim_start();
            if rest.is_empty() {
                break;
            }
            if rest.starts_with('@') {
                // @import "name";  (built-in themes only)
                let end = rest.find(';').map(|i| i + 1).unwrap_or(rest.len());
                let stmt = rest[..end].trim_end_matches(';').trim();
                if let Some(arg) = stmt.strip_prefix("@import") {
                    let arg = arg.trim().trim_start_matches("url(").trim_end_matches(')');
                    let arg = arg.trim().trim_matches(|c| c == '"' || c == '\'');
                    let arg = arg.trim_start_matches("builtin:").trim_end_matches(".css");
                    match builtin_source(arg) {
                        Some(src) if depth < 4 => self.load(src, depth + 1),
                        _ => self.warnings.push(format!("@import \"{arg}\": only built-in themes can be imported")),
                    }
                } else {
                    self.warnings.push(format!("ignored at-rule: {stmt}"));
                    // Skip a braced at-rule body if present.
                    if let Some(b) = rest[..end].find('{') {
                        let _ = b;
                    }
                }
                rest = &rest[end..];
                continue;
            }
            let Some(open) = rest.find('{') else {
                self.warnings.push(format!("stray text: {}", rest.chars().take(40).collect::<String>()));
                break;
            };
            let sel = rest[..open].trim().to_string();
            let close = rest[open + 1..].find('}').map(|i| open + 1 + i).unwrap_or(rest.len());
            let body = &rest[open + 1..close.min(rest.len())];
            let mut decls = Vec::new();
            for d in split_decls(body) {
                let Some(colon) = d.find(':') else { continue };
                let prop = d[..colon].trim().to_ascii_lowercase();
                let mut val = d[colon + 1..].trim().to_string();
                if let Some(v) = val.strip_suffix("!important") {
                    val = v.trim().to_string();
                }
                if prop.is_empty() {
                    continue;
                }
                decls.push((prop, val));
            }
            for s in sel.split(',') {
                let key: String = s.split_whitespace().collect::<Vec<_>>().join("");
                if key.is_empty() {
                    continue;
                }
                let key = key.to_ascii_lowercase();
                if key == ":root" || key == "html" || key == "body" {
                    for (p, v) in &decls {
                        if p.starts_with("--") {
                            self.vars.insert(p.clone(), v.clone());
                        }
                    }
                }
                self.rules.push((key, decls.clone()));
            }
            rest = if close < rest.len() { &rest[close + 1..] } else { "" };
        }
    }

    fn subst(&self, v: &str, depth: u32) -> String {
        if depth > 8 || !v.contains("var(") {
            return v.to_string();
        }
        let mut out = String::new();
        let mut rest = v;
        while let Some(i) = rest.find("var(") {
            out.push_str(&rest[..i]);
            let after = &rest[i + 4..];
            // find matching paren
            let mut level = 1;
            let mut end = after.len();
            for (j, ch) in after.char_indices() {
                match ch {
                    '(' => level += 1,
                    ')' => {
                        level -= 1;
                        if level == 0 {
                            end = j;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let inner = &after[..end];
            let (name, fallback) = match inner.find(',') {
                Some(c) => (inner[..c].trim(), Some(inner[c + 1..].trim())),
                None => (inner.trim(), None),
            };
            match self.vars.get(name) {
                Some(val) => out.push_str(&self.subst(val, depth + 1)),
                None => out.push_str(&self.subst(fallback.unwrap_or(""), depth + 1)),
            }
            rest = if end < after.len() { &after[end + 1..] } else { "" };
        }
        out.push_str(rest);
        out
    }

    fn resolve(mut self, name: &str) -> Theme {
        let mut index: HashMap<&'static str, El> = HashMap::new();
        for el in El::all() {
            for s in el.selectors() {
                index.insert(s, el);
            }
        }
        let mut props: Vec<Props> = vec![Props::default(); EL_COUNT];
        let rules = std::mem::take(&mut self.rules);
        for (sel, decls) in &rules {
            let Some(&el) = index.get(sel.as_str()) else {
                self.warnings.push(format!("unknown selector: {sel}"));
                continue;
            };
            let mut p = Props::default();
            for (prop, raw) in decls {
                if prop.starts_with("--") {
                    continue;
                }
                let val = self.subst(raw, 0);
                if !apply_prop(&mut p, prop, &val) {
                    self.warnings.push(format!("{sel}: unsupported `{prop}: {raw}`"));
                }
            }
            // `ul`/`ol` padding-left sets the list indent like `li` does.
            props[el as usize].merge_from(&p);
        }

        // Resolve block styles.
        let body = &props[El::Body as usize];
        let body_size = body.font_size.map(|l| l.px(15.0)).unwrap_or(15.0);
        let body_style = resolve_style(body, None, body_size);
        let mut styles = Vec::with_capacity(EL_COUNT);
        for el in El::all() {
            if el == El::Body {
                styles.push(body_style.clone());
                continue;
            }
            let p = &props[el as usize];
            let parent = match el {
                El::Th | El::Td => Some(&props[El::Table as usize]),
                El::Note | El::Tip | El::Important | El::Warning | El::Caution => Some(&props[El::Blockquote as usize]),
                El::MathBlock => Some(&props[El::Math as usize]),
                _ => None,
            };
            let mut merged = body.clone();
            // Block boxes don't inherit box properties from the body.
            merged.background = None;
            merged.border_top = None;
            merged.border_right = None;
            merged.border_bottom = None;
            merged.border_left = None;
            merged.radius = None;
            merged.padding = [None; 4];
            merged.margin = [None; 4];
            merged.max_width = None;
            if let Some(pp) = parent {
                let mut pp = pp.clone();
                if matches!(el, El::Th | El::Td) {
                    // Cells inherit text properties from the table, not its box.
                    pp.background = None;
                    pp.border_top = None;
                    pp.border_right = None;
                    pp.border_bottom = None;
                    pp.border_left = None;
                    pp.padding = [None; 4];
                    pp.margin = [None; 4];
                    pp.radius = None;
                }
                merged.merge_from(&pp);
            }
            merged.merge_from(p);
            if !el.is_block() {
                // Inline elements: keep only what they declare; the body is used for size math.
                merged = p.clone();
            }
            styles.push(resolve_style(&merged, Some(body_size), body_size));
        }
        let max_width = body.max_width.map(|l| l.px(body_size)).unwrap_or(820.0);
        let caret = body.caret_color.unwrap_or(body_style.color);
        let bg = body.background.unwrap_or(Color::rgb(255, 255, 255));
        Theme { name: name.to_string(), props, styles, max_width, caret, dark: bg.luminance() < 0.45, warnings: self.warnings }
    }
}

fn split_decls(body: &str) -> Vec<&str> {
    // Split on ';' outside parentheses and quotes.
    let mut out = Vec::new();
    let mut level = 0;
    let mut quote: Option<char> = None;
    let mut start = 0;
    for (i, ch) in body.char_indices() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                }
            }
            None => match ch {
                '"' | '\'' => quote = Some(ch),
                '(' => level += 1,
                ')' => level -= 1,
                ';' if level <= 0 => {
                    out.push(&body[start..i]);
                    start = i + 1;
                }
                _ => {}
            },
        }
    }
    out.push(&body[start..]);
    out.into_iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect()
}

fn resolve_style(p: &Props, parent_size: Option<f32>, body_size: f32) -> Style {
    let base = parent_size.unwrap_or(body_size);
    let size = p.font_size.map(|l| l.px(base)).unwrap_or(base);
    let lh = match p.line_height {
        Some(LineHeight::Mult(m)) => m * size,
        Some(LineHeight::Px(px)) => px,
        None => 1.5 * size,
    };
    let l = |o: Option<Len>| o.map(|l| l.px(size)).unwrap_or(0.0);
    Style {
        families: p.font_family.clone().filter(|f| !f.is_empty()).unwrap_or_else(|| vec!["Segoe UI".into()]),
        size,
        weight: p.font_weight.unwrap_or(400),
        italic: p.italic.unwrap_or(false),
        color: p.color.unwrap_or(Color::rgb(0, 0, 0)),
        background: p.background.filter(|c| c.a > 0.0),
        line_height: lh,
        border_top: p.border_top.filter(|b| b.width > 0.0),
        border_right: p.border_right.filter(|b| b.width > 0.0),
        border_bottom: p.border_bottom.filter(|b| b.width > 0.0),
        border_left: p.border_left.filter(|b| b.width > 0.0),
        radius: l(p.radius),
        padding: [l(p.padding[0]), l(p.padding[1]), l(p.padding[2]), l(p.padding[3])],
        margin: [l(p.margin[0]), l(p.margin[1]), l(p.margin[2]), l(p.margin[3])],
        underline: p.underline.unwrap_or(false),
        strike: p.strike.unwrap_or(false),
        uppercase: p.uppercase.unwrap_or(false),
        letter_spacing: l(p.letter_spacing),
    }
}

fn split_ws(v: &str) -> Vec<&str> {
    // Split on whitespace outside parentheses.
    let mut out = Vec::new();
    let mut level = 0;
    let mut start: Option<usize> = None;
    for (i, ch) in v.char_indices() {
        match ch {
            '(' => {
                level += 1;
                if start.is_none() {
                    start = Some(i);
                }
            }
            ')' => level -= 1,
            c if c.is_whitespace() && level == 0 => {
                if let Some(s) = start.take() {
                    out.push(&v[s..i]);
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
    }
    if let Some(s) = start {
        out.push(&v[s..]);
    }
    out
}

pub fn parse_len(v: &str) -> Option<Len> {
    let v = v.trim();
    if v == "0" {
        return Some(Len::Px(0.0));
    }
    let num_end = v.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+')).unwrap_or(v.len());
    let n: f32 = v[..num_end].parse().ok()?;
    match &v[num_end..] {
        "px" | "" => Some(Len::Px(n)),
        "em" | "rem" => Some(Len::Em(n)),
        "%" => Some(Len::Em(n / 100.0)),
        "pt" => Some(Len::Px(n * 96.0 / 72.0)),
        "ch" => Some(Len::Em(n * 0.55)),
        _ => None,
    }
}

fn hex(v: &str) -> Option<Color> {
    let h = v.strip_prefix('#')?;
    let d = |s: &str| u8::from_str_radix(s, 16).ok();
    let dd = |c: char| d(&format!("{c}{c}"));
    let ch: Vec<char> = h.chars().collect();
    match ch.len() {
        3 | 4 => {
            let a = if ch.len() == 4 { dd(ch[3])? } else { 255 };
            Some(Color { a: a as f32 / 255.0, ..Color::rgb(dd(ch[0])?, dd(ch[1])?, dd(ch[2])?) })
        }
        6 | 8 => {
            let a = if ch.len() == 8 { d(&h[6..8])? } else { 255 };
            Some(Color { a: a as f32 / 255.0, ..Color::rgb(d(&h[0..2])?, d(&h[2..4])?, d(&h[4..6])?) })
        }
        _ => None,
    }
}

fn func_args(v: &str, name: &str) -> Option<Vec<String>> {
    let rest = v.strip_prefix(name)?.trim_start().strip_prefix('(')?;
    let inner = rest.strip_suffix(')')?;
    let mut args = Vec::new();
    let mut level = 0;
    let mut cur = String::new();
    for ch in inner.chars() {
        match ch {
            '(' => {
                level += 1;
                cur.push(ch)
            }
            ')' => {
                level -= 1;
                cur.push(ch)
            }
            ',' | '/' if level == 0 => {
                args.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        args.push(cur.trim().to_string());
    }
    Some(args)
}

pub fn parse_color(v: &str) -> Option<Color> {
    let v = v.trim();
    let lower = v.to_ascii_lowercase();
    if lower.starts_with('#') {
        return hex(&lower);
    }
    let comp = |s: &str, scale: f32| -> Option<f32> {
        let s = s.trim();
        if let Some(p) = s.strip_suffix('%') {
            p.trim().parse::<f32>().ok().map(|x| x / 100.0)
        } else {
            s.parse::<f32>().ok().map(|x| x / scale)
        }
    };
    for name in ["rgba", "rgb"] {
        if let Some(args) = func_args(&lower, name) {
            // rgb(1 2 3 / 0.5) or rgb(1, 2, 3, 0.5)
            let parts: Vec<String> = if args.len() == 1 {
                split_ws(&args[0]).iter().map(|s| s.to_string()).collect()
            } else if args.len() == 2 && !args[0].contains(',') {
                let mut p: Vec<String> = split_ws(&args[0]).iter().map(|s| s.to_string()).collect();
                p.push(args[1].clone());
                p
            } else {
                args
            };
            if parts.len() < 3 {
                return None;
            }
            let a = parts.get(3).and_then(|s| comp(s, 1.0)).unwrap_or(1.0);
            return Some(Color { r: comp(&parts[0], 255.0)?, g: comp(&parts[1], 255.0)?, b: comp(&parts[2], 255.0)?, a });
        }
    }
    for name in ["hsla", "hsl"] {
        if let Some(args) = func_args(&lower, name) {
            let parts: Vec<String> = if args.len() == 1 { split_ws(&args[0]).iter().map(|s| s.to_string()).collect() } else { args };
            if parts.len() < 3 {
                return None;
            }
            let h: f32 = parts[0].trim_end_matches("deg").parse().ok()?;
            let s = comp(&parts[1], 100.0)?;
            let l = comp(&parts[2], 100.0)?;
            let a = parts.get(3).and_then(|s| comp(s, 1.0)).unwrap_or(1.0);
            let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
            let hp = (h.rem_euclid(360.0)) / 60.0;
            let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
            let (r, g, b) = match hp as i32 {
                0 => (c, x, 0.0),
                1 => (x, c, 0.0),
                2 => (0.0, c, x),
                3 => (0.0, x, c),
                4 => (x, 0.0, c),
                _ => (c, 0.0, x),
            };
            let m = l - c / 2.0;
            return Some(Color { r: r + m, g: g + m, b: b + m, a });
        }
    }
    if let Some(args) = func_args(&lower, "color-mix") {
        // color-mix(in srgb, A p%, B [q%])
        if args.len() >= 3 {
            let pc = |s: &str| -> (Option<Color>, Option<f32>) {
                let parts = split_ws(s);
                let mut col = None;
                let mut pct = None;
                for p in parts {
                    if let Some(n) = p.strip_suffix('%') {
                        pct = n.parse::<f32>().ok().map(|x| x / 100.0);
                    } else {
                        col = parse_color(p);
                    }
                }
                (col, pct)
            };
            let (a, pa) = pc(&args[1]);
            let (b, pb) = pc(&args[2]);
            let (a, b) = (a?, b?);
            let wa = pa.unwrap_or_else(|| 1.0 - pb.unwrap_or(0.5));
            return Some(b.mix(&a, wa));
        }
        return None;
    }
    let named = match lower.as_str() {
        "transparent" => Color::TRANSPARENT,
        "white" => Color::rgb(255, 255, 255),
        "black" => Color::rgb(0, 0, 0),
        "red" => Color::rgb(255, 0, 0),
        "green" => Color::rgb(0, 128, 0),
        "blue" => Color::rgb(0, 0, 255),
        "gray" | "grey" => Color::rgb(128, 128, 128),
        "silver" => Color::rgb(192, 192, 192),
        "yellow" => Color::rgb(255, 255, 0),
        "orange" => Color::rgb(255, 165, 0),
        "purple" => Color::rgb(128, 0, 128),
        "teal" => Color::rgb(0, 128, 128),
        "navy" => Color::rgb(0, 0, 128),
        "maroon" => Color::rgb(128, 0, 0),
        "olive" => Color::rgb(128, 128, 0),
        "lime" => Color::rgb(0, 255, 0),
        "aqua" | "cyan" => Color::rgb(0, 255, 255),
        "fuchsia" | "magenta" => Color::rgb(255, 0, 255),
        "none" => Color::TRANSPARENT,
        _ => return None,
    };
    Some(named)
}

fn parse_border(v: &str) -> Option<Border> {
    let v = v.trim();
    if v == "none" || v == "0" {
        return Some(Border { width: 0.0, color: Color::TRANSPARENT });
    }
    let mut width = 1.0;
    let mut color = None;
    for part in split_ws(v) {
        if let Some(l) = parse_len(part) {
            width = l.px(16.0);
        } else if let Some(c) = parse_color(part) {
            color = Some(c);
        } else if matches!(part, "solid" | "dashed" | "dotted" | "double") {
        } else if part == "none" {
            width = 0.0;
        } else {
            return None;
        }
    }
    Some(Border { width, color: color.unwrap_or(Color::rgb(128, 128, 128)) })
}

fn box4(v: &str) -> Option<[Option<Len>; 4]> {
    let parts: Vec<Len> =
        split_ws(v).iter().map(|p| if *p == "auto" { Some(Len::Px(0.0)) } else { parse_len(p) }).collect::<Option<Vec<_>>>()?;
    let [t, r, b, l] = match parts.len() {
        1 => [parts[0], parts[0], parts[0], parts[0]],
        2 => [parts[0], parts[1], parts[0], parts[1]],
        3 => [parts[0], parts[1], parts[2], parts[1]],
        4 => [parts[0], parts[1], parts[2], parts[3]],
        _ => return None,
    };
    Some([Some(t), Some(r), Some(b), Some(l)])
}

fn parse_family(v: &str) -> Vec<String> {
    v.split(',')
        .map(|s| s.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
        .filter(|s| !s.is_empty())
        .map(|s| match s.as_str() {
            "sans-serif" | "system-ui" | "ui-sans-serif" | "-apple-system" => "Segoe UI".to_string(),
            "serif" | "ui-serif" => "Georgia".to_string(),
            "monospace" | "ui-monospace" => "Consolas".to_string(),
            _ => s,
        })
        .collect()
}

fn apply_prop(p: &mut Props, prop: &str, v: &str) -> bool {
    let v = v.trim();
    match prop {
        "font-family" => p.font_family = Some(parse_family(v)),
        "font-size" => match parse_len(v) {
            Some(l) => p.font_size = Some(l),
            None => return false,
        },
        "font-weight" => {
            p.font_weight = Some(match v {
                "normal" => 400,
                "bold" => 700,
                "bolder" => 700,
                "lighter" => 300,
                _ => match v.parse::<u16>() {
                    Ok(n) => n.clamp(100, 950),
                    Err(_) => return false,
                },
            })
        }
        "font-style" => p.italic = Some(v == "italic" || v == "oblique"),
        "font" => {
            // font: [style] [weight] size[/line-height] family
            let parts = split_ws(v);
            let mut i = 0;
            while i < parts.len() {
                let part = parts[i];
                if part == "italic" {
                    p.italic = Some(true);
                } else if part == "bold" {
                    p.font_weight = Some(700);
                } else if let Ok(n) = part.parse::<u16>() {
                    p.font_weight = Some(n);
                } else if part.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                    let (sz, lh) = match part.find('/') {
                        Some(s) => (&part[..s], Some(&part[s + 1..])),
                        None => (part, None),
                    };
                    p.font_size = parse_len(sz).or(p.font_size);
                    if let Some(lh) = lh {
                        apply_prop(p, "line-height", lh);
                    }
                    let fam = parts[i + 1..].join(" ");
                    if !fam.is_empty() {
                        p.font_family = Some(parse_family(&fam));
                    }
                    break;
                }
                i += 1;
            }
        }
        "color" => match parse_color(v) {
            Some(c) => p.color = Some(c),
            None => return false,
        },
        "background" | "background-color" => match parse_color(v) {
            Some(c) => p.background = Some(c),
            None => return false,
        },
        "line-height" => {
            if v == "normal" {
                p.line_height = Some(LineHeight::Mult(1.4));
            } else if let Ok(m) = v.parse::<f32>() {
                p.line_height = Some(LineHeight::Mult(m));
            } else {
                match parse_len(v) {
                    Some(Len::Px(px)) => p.line_height = Some(LineHeight::Px(px)),
                    Some(Len::Em(e)) => p.line_height = Some(LineHeight::Mult(e)),
                    None => return false,
                }
            }
        }
        "border" => match parse_border(v) {
            Some(b) => {
                p.border_top = Some(b);
                p.border_right = Some(b);
                p.border_bottom = Some(b);
                p.border_left = Some(b);
            }
            None => return false,
        },
        "border-top" | "border-right" | "border-bottom" | "border-left" => match parse_border(v) {
            Some(b) => match prop {
                "border-top" => p.border_top = Some(b),
                "border-right" => p.border_right = Some(b),
                "border-bottom" => p.border_bottom = Some(b),
                _ => p.border_left = Some(b),
            },
            None => return false,
        },
        "border-color" | "border-left-color" => match parse_color(v) {
            Some(c) => {
                let set = |b: &mut Option<Border>| {
                    let w = b.map(|b| b.width).unwrap_or(1.0);
                    *b = Some(Border { width: w, color: c });
                };
                if prop == "border-color" {
                    set(&mut p.border_top);
                    set(&mut p.border_right);
                    set(&mut p.border_bottom);
                }
                set(&mut p.border_left);
            }
            None => return false,
        },
        "border-radius" => match parse_len(split_ws(v).first().copied().unwrap_or("0")) {
            Some(l) => p.radius = Some(l),
            None => return false,
        },
        "padding" => match box4(v) {
            Some(b) => p.padding = b,
            None => return false,
        },
        "margin" => match box4(v) {
            Some(b) => p.margin = b,
            None => return false,
        },
        "padding-top" | "padding-right" | "padding-bottom" | "padding-left" | "margin-top" | "margin-right" | "margin-bottom"
        | "margin-left" => {
            let Some(l) = parse_len(v) else { return false };
            let i = match prop.rsplit('-').next().unwrap() {
                "top" => 0,
                "right" => 1,
                "bottom" => 2,
                _ => 3,
            };
            if prop.starts_with("padding") {
                p.padding[i] = Some(l);
            } else {
                p.margin[i] = Some(l);
            }
        }
        "max-width" | "width" => match parse_len(v) {
            Some(l) => p.max_width = Some(l),
            None => return false,
        },
        "text-decoration" | "text-decoration-line" => {
            p.underline = Some(v.contains("underline"));
            p.strike = Some(v.contains("line-through"));
        }
        "caret-color" => match parse_color(v) {
            Some(c) => p.caret_color = Some(c),
            None => return false,
        },
        "text-transform" => p.uppercase = Some(v == "uppercase"),
        "letter-spacing" => match parse_len(v) {
            Some(l) => p.letter_spacing = Some(l),
            None => {
                if v == "normal" {
                    p.letter_spacing = Some(Len::Px(0.0));
                } else {
                    return false;
                }
            }
        },
        // Accepted and ignored: harmless CSS a theme author may paste from a web stylesheet.
        "border-style"
        | "list-style"
        | "list-style-type"
        | "border-collapse"
        | "overflow"
        | "overflow-x"
        | "white-space"
        | "overflow-wrap"
        | "word-break"
        | "display"
        | "text-align"
        | "font-feature-settings"
        | "font-variant-ligatures"
        | "font-synthesis"
        | "color-scheme"
        | "-webkit-font-smoothing"
        | "tab-size"
        | "text-underline-offset"
        | "box-shadow"
        | "opacity"
        | "cursor"
        | "transition" => {}
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_resolve_without_warnings() {
        for (name, _) in BUILTIN {
            let t = Theme::builtin(name);
            assert!(t.warnings.is_empty(), "{name}: {:?}", t.warnings);
            assert!(t.style(El::Body).size > 8.0);
        }
        assert!(Theme::builtin("Dark").dark);
        assert!(!Theme::builtin("Light").dark);
    }

    #[test]
    fn colors() {
        assert_eq!(parse_color("#fff").unwrap().key(), 0xffffffff);
        assert_eq!(parse_color("#14161a").unwrap().key(), 0x14161aff);
        assert_eq!(parse_color("rgba(255, 0, 0, 0.5)").unwrap().key() & 0xff, 128);
        assert_eq!(parse_color("rgb(0 255 0 / 50%)").unwrap().key(), 0x00ff0080);
        assert!(parse_color("hsl(0, 100%, 50%)").unwrap().r > 0.99);
        let m = parse_color("color-mix(in srgb, #ff0000 50%, #0000ff)").unwrap();
        assert!((m.r - 0.5).abs() < 0.01 && (m.b - 0.5).abs() < 0.01);
    }

    #[test]
    fn vars_import_and_overrides() {
        let t = Theme::from_css(
            "x",
            "@import \"dark\"; :root { --ink: #ff0000; } h1 { color: var(--ink); font-size: 2em } nonsense { color: red }",
        );
        assert_eq!(t.style(El::H1).color.key(), 0xff0000ff);
        assert!((t.style(El::H1).size - 2.0 * t.style(El::Body).size).abs() < 0.01);
        assert!(t.warnings.iter().any(|w| w.contains("nonsense")));
        assert!(t.dark);
    }

    #[test]
    fn garbage_never_panics() {
        for css in [
            "{",
            "}",
            "a{b",
            "@import",
            "h1 { color: var(--x",
            "h1{font:}",
            "/* unterminated",
            "h1 { border: 2px solid } p{margin:1 2 3 4 5}",
        ] {
            let _ = Theme::from_css("g", css);
        }
    }
}
