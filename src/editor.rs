//! The document buffer: source text, selection, undo/redo, file encoding, and the
//! Markdown-aware edits (smart Enter, formatting toggles, headings, list indentation).
//!
//! Everything here works on source byte offsets and is independent of the screen.

use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Ansi,
}

impl Encoding {
    pub fn label(self, bom: bool) -> &'static str {
        match (self, bom) {
            (Encoding::Utf8, false) => "UTF-8",
            (Encoding::Utf8, true) => "UTF-8 BOM",
            (Encoding::Utf16Le, _) => "UTF-16 LE",
            (Encoding::Utf16Be, _) => "UTF-16 BE",
            (Encoding::Ansi, _) => "ANSI",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditKind {
    Typing,
    Delete,
    Other,
}

#[derive(Clone, Debug)]
struct Edit {
    pos: usize,
    removed: String,
    inserted: String,
    sel_before: (usize, usize),
    sel_after: (usize, usize),
    kind: EditKind,
    at: Instant,
    /// Edits sharing a group id undo together.
    group: u64,
}

pub struct Doc {
    pub text: String,
    pub path: Option<PathBuf>,
    pub enc: Encoding,
    pub bom: bool,
    pub crlf: bool,
    pub dirty: bool,
    pub stamp: (u64, u64),
    pub anchor: usize,
    pub head: usize,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    group: u64,
    saved_undo_len: Option<usize>,
}

impl Default for Doc {
    fn default() -> Self {
        Doc::new()
    }
}

impl Doc {
    pub fn new() -> Doc {
        Doc {
            text: String::new(),
            path: None,
            enc: Encoding::Utf8,
            bom: false,
            crlf: true,
            dirty: false,
            stamp: (0, 0),
            anchor: 0,
            head: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            group: 0,
            saved_undo_len: Some(0),
        }
    }

    #[cfg(test)]
    pub fn from_text(text: &str) -> Doc {
        let mut d = Doc::new();
        d.text = text.replace("\r\n", "\n").replace('\r', "\n");
        d
    }

    pub fn sel(&self) -> (usize, usize) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }

    pub fn has_sel(&self) -> bool {
        self.anchor != self.head
    }

    pub fn set_sel(&mut self, anchor: usize, head: usize) {
        self.anchor = clamp_boundary(&self.text, anchor);
        self.head = clamp_boundary(&self.text, head);
    }

    pub fn begin_group(&mut self) {
        self.group += 1;
    }

    /// Replace `range` with `text` and put the caret after the insertion.
    pub fn replace(&mut self, range: (usize, usize), text: &str, kind: EditKind) {
        let (a, b) = (clamp_boundary(&self.text, range.0), clamp_boundary(&self.text, range.1));
        let (a, b) = (a.min(b), a.max(b));
        let removed = self.text[a..b].to_string();
        if removed.is_empty() && text.is_empty() {
            return;
        }
        let sel_before = (self.anchor, self.head);
        self.text.replace_range(a..b, text);
        let caret = a + text.len();
        self.anchor = caret;
        self.head = caret;
        self.redo.clear();
        let now = Instant::now();
        // Coalesce typing into one undo step per word-ish run.
        if kind == EditKind::Typing && removed.is_empty() {
            if let Some(last) = self.undo.last_mut() {
                if last.kind == EditKind::Typing
                    && last.removed.is_empty()
                    && last.pos + last.inserted.len() == a
                    && now.duration_since(last.at).as_millis() < 1500
                    && !(text.chars().all(char::is_whitespace) && !last.inserted.ends_with(char::is_whitespace))
                {
                    last.inserted.push_str(text);
                    last.sel_after = (caret, caret);
                    last.at = now;
                    self.mark_dirty();
                    return;
                }
            }
        }
        if kind == EditKind::Delete && text.is_empty() {
            if let Some(last) = self.undo.last_mut() {
                if last.kind == EditKind::Delete && last.inserted.is_empty() && now.duration_since(last.at).as_millis() < 1500 {
                    if b == last.pos {
                        // backspace run
                        last.removed.insert_str(0, &removed);
                        last.pos = a;
                        last.sel_after = (a, a);
                        last.at = now;
                        self.mark_dirty();
                        return;
                    } else if a == last.pos {
                        // forward delete run
                        last.removed.push_str(&removed);
                        last.sel_after = (a, a);
                        last.at = now;
                        self.mark_dirty();
                        return;
                    }
                }
            }
        }
        self.undo.push(Edit {
            pos: a,
            removed,
            inserted: text.to_string(),
            sel_before,
            sel_after: (caret, caret),
            kind,
            at: now,
            group: self.group,
        });
        if let Some(n) = self.saved_undo_len {
            if n > self.undo.len() {
                self.saved_undo_len = None;
            }
        }
        self.mark_dirty();
    }

    fn mark_dirty(&mut self) {
        self.dirty = self.saved_undo_len != Some(self.undo.len()) || true;
        self.dirty = Some(self.undo.len()) != self.saved_undo_len;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(first) = self.undo.pop() else { return false };
        let group = first.group;
        let mut batch = vec![first];
        while let Some(e) = self.undo.last() {
            if e.group == group && e.kind == EditKind::Other && batch[0].kind == EditKind::Other {
                batch.push(self.undo.pop().unwrap());
            } else {
                break;
            }
        }
        let mut sel = (0, 0);
        for e in &batch {
            let end = e.pos + e.inserted.len();
            self.text.replace_range(e.pos..end, &e.removed);
            sel = e.sel_before;
        }
        self.anchor = clamp_boundary(&self.text, sel.0);
        self.head = clamp_boundary(&self.text, sel.1);
        batch.reverse();
        self.redo.extend(batch.into_iter().rev());
        self.dirty = Some(self.undo.len()) != self.saved_undo_len;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(first) = self.redo.pop() else { return false };
        let group = first.group;
        let mut batch = vec![first];
        while let Some(e) = self.redo.last() {
            if e.group == group && e.kind == EditKind::Other && batch[0].kind == EditKind::Other {
                batch.push(self.redo.pop().unwrap());
            } else {
                break;
            }
        }
        let mut sel = (0, 0);
        for e in &batch {
            let end = e.pos + e.removed.len();
            self.text.replace_range(e.pos..end, &e.inserted);
            sel = e.sel_after;
        }
        self.anchor = clamp_boundary(&self.text, sel.0);
        self.head = clamp_boundary(&self.text, sel.1);
        self.undo.extend(batch);
        self.dirty = Some(self.undo.len()) != self.saved_undo_len;
        true
    }

    pub fn mark_saved(&mut self) {
        self.saved_undo_len = Some(self.undo.len());
        self.dirty = false;
    }

    /// Replace the whole text (reload from disk) as one undoable step, keeping the caret near.
    pub fn reload_text(&mut self, text: String) {
        let caret = self.head.min(text.len());
        self.text = text;
        self.undo.clear();
        self.redo.clear();
        self.saved_undo_len = Some(0);
        self.dirty = false;
        let c = clamp_boundary(&self.text, caret);
        self.anchor = c;
        self.head = c;
    }

    // ------------------------------------------------------------ file I/O

    pub fn load(path: &Path) -> std::io::Result<Doc> {
        let bytes = std::fs::read(path)?;
        let mut d = Doc::new();
        let (text, enc, bom) = decode(&bytes);
        let crlf = text.contains("\r\n");
        let lf_only = text.matches('\n').count() - text.matches("\r\n").count();
        d.crlf = crlf && text.matches("\r\n").count() >= lf_only;
        d.text = text.replace("\r\n", "\n").replace('\r', "\n");
        d.enc = enc;
        d.bom = bom;
        d.path = Some(path.to_path_buf());
        d.stamp = file_stamp(path);
        Ok(d)
    }

    pub fn encoded(&self) -> Vec<u8> {
        let text = if self.crlf { self.text.replace('\n', "\r\n") } else { self.text.clone() };
        encode(&text, self.enc, self.bom)
    }

    pub fn save_to(&mut self, path: &Path) -> std::io::Result<()> {
        let bytes = self.encoded();
        // Write next to the target, then swap it in, so a failed write never truncates the file.
        let tmp = path.with_extension(format!("{}.brisk-tmp", path.extension().and_then(|e| e.to_str()).unwrap_or("md")));
        std::fs::write(&tmp, &bytes)?;
        if let Err(e) = replace_file(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            // Fall back to a direct write (e.g. a share that refuses renames over files).
            std::fs::write(path, &bytes).map_err(|_| e)?;
        }
        self.path = Some(path.to_path_buf());
        self.stamp = file_stamp(path);
        self.mark_saved();
        Ok(())
    }

    pub fn word_count(&self) -> usize {
        self.text.split_whitespace().filter(|w| w.chars().any(|c| c.is_alphanumeric())).count()
    }

    /// 1-based line and column (in characters) of a source offset.
    pub fn line_col(&self, pos: usize) -> (usize, usize) {
        let pos = clamp_boundary(&self.text, pos);
        let before = &self.text[..pos];
        let line = before.matches('\n').count() + 1;
        let ls = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        (line, self.text[ls..pos].chars().count() + 1)
    }

    pub fn offset_of_line(&self, line: usize) -> usize {
        if line <= 1 {
            return 0;
        }
        let mut n = 1;
        for (i, b) in self.text.bytes().enumerate() {
            if b == b'\n' {
                n += 1;
                if n == line {
                    return i + 1;
                }
            }
        }
        self.text.len()
    }
}

fn replace_file(tmp: &Path, path: &Path) -> std::io::Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::*;
    let w = |p: &Path| -> Vec<u16> { p.as_os_str().to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect() };
    let (t, p) = (w(tmp), w(path));
    unsafe {
        if path.exists() {
            // ReplaceFileW keeps the original's attributes, ACLs and creation time.
            if ReplaceFileW(PCWSTR(p.as_ptr()), PCWSTR(t.as_ptr()), PCWSTR::null(), REPLACE_FILE_FLAGS(0), None, None).is_ok() {
                return Ok(());
            }
        }
        MoveFileExW(PCWSTR(t.as_ptr()), PCWSTR(p.as_ptr()), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)
            .map_err(|e| std::io::Error::other(e.message()))
    }
}

pub fn file_stamp(path: &Path) -> (u64, u64) {
    match std::fs::metadata(path) {
        Ok(m) => {
            let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64).unwrap_or(0);
            (t, m.len())
        }
        Err(_) => (0, 0),
    }
}

pub fn decode(bytes: &[u8]) -> (String, Encoding, bool) {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (String::from_utf8_lossy(&bytes[3..]).into_owned(), Encoding::Utf8, true);
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let u: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return (String::from_utf16_lossy(&u), Encoding::Utf16Le, true);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let u: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return (String::from_utf16_lossy(&u), Encoding::Utf16Be, true);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), Encoding::Utf8, false),
        Err(_) => (ansi_to_string(bytes), Encoding::Ansi, false),
    }
}

fn ansi_to_string(bytes: &[u8]) -> String {
    use windows::Win32::Globalization::*;
    unsafe {
        let n = MultiByteToWideChar(CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, None);
        if n <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let mut buf = vec![0u16; n as usize];
        MultiByteToWideChar(CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, Some(&mut buf));
        String::from_utf16_lossy(&buf)
    }
}

pub fn encode(text: &str, enc: Encoding, bom: bool) -> Vec<u8> {
    match enc {
        Encoding::Utf8 => {
            let mut v = Vec::with_capacity(text.len() + 3);
            if bom {
                v.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            }
            v.extend_from_slice(text.as_bytes());
            v
        }
        Encoding::Utf16Le => {
            let mut v = vec![0xFF, 0xFE];
            for u in text.encode_utf16() {
                v.extend_from_slice(&u.to_le_bytes());
            }
            v
        }
        Encoding::Utf16Be => {
            let mut v = vec![0xFE, 0xFF];
            for u in text.encode_utf16() {
                v.extend_from_slice(&u.to_be_bytes());
            }
            v
        }
        Encoding::Ansi => {
            use windows::Win32::Globalization::*;
            let w: Vec<u16> = text.encode_utf16().collect();
            unsafe {
                let n = WideCharToMultiByte(CP_ACP, 0, &w, None, windows::core::PCSTR::null(), None);
                let mut out = vec![0u8; n.max(0) as usize];
                WideCharToMultiByte(CP_ACP, 0, &w, Some(&mut out), windows::core::PCSTR::null(), None);
                out
            }
        }
    }
}

pub fn clamp_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

pub fn line_start(s: &str, pos: usize) -> usize {
    s[..pos.min(s.len())].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

pub fn line_end(s: &str, pos: usize) -> usize {
    let pos = pos.min(s.len());
    s[pos..].find('\n').map(|i| pos + i).unwrap_or(s.len())
}

/// The structural prefix of a Markdown line: indentation, quote markers, list marker, task box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefix {
    /// Bytes of the prefix in the line.
    pub len: usize,
    /// What a new line continuing this one should start with.
    pub continuation: String,
    pub has_list: bool,
    pub has_quote: bool,
    /// Width of the list marker plus its space (for indenting nested items).
    pub marker_width: usize,
}

pub fn parse_prefix(line: &str) -> Prefix {
    let b = line.as_bytes();
    let mut i = 0;
    let mut cont = String::new();
    let mut has_quote = false;
    // leading spaces and quote markers
    loop {
        let start = i;
        while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
            i += 1;
        }
        if i < b.len() && b[i] == b'>' {
            has_quote = true;
            i += 1;
            if i < b.len() && b[i] == b' ' {
                i += 1;
            }
            cont.push_str(&line[start..i]);
            continue;
        }
        cont.push_str(&line[start..i]);
        break;
    }
    let mut has_list = false;
    let mut marker_width = 0;
    let rest = &line[i..];
    let rb = rest.as_bytes();
    if rb.len() >= 2
        && matches!(rb[0], b'-' | b'*' | b'+')
        && (rb[1] == b' ' || rb[1] == b'\t')
        && !(rb[0] == b'*' && rest.starts_with("* * *"))
        && !rest.starts_with("- - -")
    {
        has_list = true;
        let mut k = 1;
        while k < rb.len() && (rb[k] == b' ' || rb[k] == b'\t') {
            k += 1;
        }
        marker_width = k;
        cont.push_str(&rest[..k]);
        i += k;
    } else {
        let digits = rb.iter().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0
            && digits <= 9
            && rb.len() > digits + 1
            && matches!(rb[digits], b'.' | b')')
            && (rb[digits + 1] == b' ' || rb[digits + 1] == b'\t')
        {
            has_list = true;
            let n: u64 = rest[..digits].parse().unwrap_or(1);
            let mut k = digits + 1;
            while k < rb.len() && (rb[k] == b' ' || rb[k] == b'\t') {
                k += 1;
            }
            marker_width = k;
            cont.push_str(&format!("{}{}{}", n + 1, rb[digits] as char, &rest[digits + 1..k]));
            i += k;
        }
    }
    if has_list {
        let r = &line[i..];
        if r.len() >= 3
            && (r.starts_with("[ ]") || r.starts_with("[x]") || r.starts_with("[X]"))
            && (r.len() == 3 || r.as_bytes()[3] == b' ')
        {
            let k = if r.len() > 3 { 4 } else { 3 };
            cont.push_str("[ ] ");
            i += k;
        }
    }
    Prefix { len: i, continuation: cont, has_list, has_quote, marker_width }
}

/// Result of a smart edit: replace `range` with `text`, then put the caret at `caret`.
#[derive(Debug, PartialEq, Eq)]
pub struct Change {
    pub range: (usize, usize),
    pub text: String,
    pub caret: usize,
}

/// Enter. `markdown` is true in Markdown mode, where a plain paragraph line splits into two
/// paragraphs (a blank line); in Mono mode it is a plain newline. Lists and quotes continue in
/// both modes, and Enter on an empty item leaves the list.
pub fn smart_newline(s: &str, sel: (usize, usize), markdown: bool, in_code: bool) -> Change {
    let (a, b) = sel;
    let ls = line_start(s, a);
    let line = &s[ls..line_end(s, a)];
    if in_code {
        let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let text = format!("\n{indent}");
        return Change { range: (a, b), caret: a + text.len(), text };
    }
    let p = parse_prefix(line);
    let after_prefix = &line[p.len.min(line.len())..];
    if (p.has_list || p.has_quote) && after_prefix.trim().is_empty() && a == b && a >= ls + p.len {
        // Empty item or quote line: leave the container, keeping a blank line so the next text
        // starts a fresh paragraph.
        let le = line_end(s, a);
        return Change { range: (ls, le), text: "\n".into(), caret: ls + 1 };
    }
    if p.has_list || p.has_quote {
        if a < ls + p.len {
            // Caret inside the prefix: plain newline before the line.
            return Change { range: (ls, ls), text: "\n".into(), caret: a + 1 };
        }
        let text = format!("\n{}", p.continuation);
        return Change { range: (a, b), caret: a + text.len(), text };
    }
    let is_heading = line.trim_start().starts_with('#');
    if markdown && !line.trim().is_empty() {
        // New paragraph. If the paragraph continues on the next source line, split cleanly.
        let text = "\n\n".to_string();
        let _ = is_heading;
        return Change { range: (a, b), caret: a + text.len(), text };
    }
    Change { range: (a, b), text: "\n".into(), caret: a + 1 }
}

/// Wrap or unwrap the selection with `mark` (e.g. "**"). With no selection, the word at the
/// caret is used; with no word, an empty pair is inserted with the caret between.
pub fn toggle_wrap(s: &str, sel: (usize, usize), mark: &str) -> (Change, (usize, usize)) {
    let (mut a, mut b) = sel;
    if a == b {
        let (wa, wb) = word_at(s, a);
        if wa < wb {
            a = wa;
            b = wb;
        } else {
            let text = format!("{mark}{mark}");
            return (Change { range: (a, a), text, caret: a + mark.len() }, (a + mark.len(), a + mark.len()));
        }
    }
    let m = mark.len();
    // Already wrapped just outside the selection?
    if a >= m && b + m <= s.len() && &s[a - m..a] == mark && &s[b..b + m] == mark {
        let inner = s[a..b].to_string();
        return (Change { range: (a - m, b + m), text: inner, caret: b - m }, (a - m, b - m));
    }
    // Selection itself includes the marks?
    if b - a >= 2 * m && s[a..b].starts_with(mark) && s[a..b].ends_with(mark) {
        let inner = s[a + m..b - m].to_string();
        return (Change { range: (a, b), text: inner, caret: b - 2 * m }, (a, b - 2 * m));
    }
    let text = format!("{mark}{}{mark}", &s[a..b]);
    (Change { range: (a, b), text, caret: b + 2 * m }, (a + m, b + m))
}

pub fn word_at(s: &str, pos: usize) -> (usize, usize) {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '\'';
    let mut a = pos;
    while a > 0 {
        let c = s[..a].chars().next_back().unwrap();
        if !is_word(c) {
            break;
        }
        a -= c.len_utf8();
    }
    let mut b = pos;
    while b < s.len() {
        let c = s[b..].chars().next().unwrap();
        if !is_word(c) {
            break;
        }
        b += c.len_utf8();
    }
    (a, b)
}

/// Set the heading level of the line at `pos` (0 = plain paragraph). Same level again toggles off.
pub fn set_heading(s: &str, pos: usize, level: usize) -> Change {
    let ls = line_start(s, pos);
    let le = line_end(s, pos);
    let line = &s[ls..le];
    let hashes = line.bytes().take_while(|&c| c == b'#').count();
    let has_heading = hashes > 0 && hashes <= 6 && (line.len() == hashes || line.as_bytes()[hashes] == b' ');
    let body_start = if has_heading { (hashes + 1).min(line.len()) } else { 0 };
    let body = &line[body_start..];
    let target = if has_heading && hashes == level { 0 } else { level };
    let new_line = if target == 0 { body.to_string() } else { format!("{} {}", "#".repeat(target), body) };
    let caret_in_body = pos.saturating_sub(ls + body_start);
    let new_body_start = if target == 0 { 0 } else { target + 1 };
    Change { range: (ls, le), caret: ls + new_body_start + caret_in_body.min(body.len()), text: new_line }
}

/// Indent (or outdent) the list items on the selected lines.
pub fn indent_lines(s: &str, sel: (usize, usize), outdent: bool) -> Option<Change> {
    let ls = line_start(s, sel.0);
    let le = line_end(s, sel.1);
    let block = &s[ls..le];
    let mut out = String::with_capacity(block.len() + 16);
    let mut any = false;
    let mut caret_shift: isize = 0;
    let caret_line_start = line_start(s, sel.1);
    let mut offset = ls;
    for (k, line) in block.split('\n').enumerate() {
        if k > 0 {
            out.push('\n');
        }
        let p = parse_prefix(line);
        if p.has_list {
            let width = p.marker_width.clamp(2, 4);
            if outdent {
                let lead = line.bytes().take_while(|&c| c == b' ').count().min(width);
                if lead > 0 {
                    any = true;
                    if offset <= caret_line_start {
                        caret_shift -= lead as isize;
                    }
                }
                out.push_str(&line[lead..]);
            } else {
                any = true;
                out.push_str(&" ".repeat(width));
                out.push_str(line);
                if offset <= caret_line_start {
                    caret_shift += width as isize;
                }
            }
        } else {
            out.push_str(line);
        }
        offset += line.len() + 1;
    }
    if !any {
        return None;
    }
    let caret = (sel.1 as isize + caret_shift).max(ls as isize) as usize;
    Some(Change { range: (ls, le), text: out, caret })
}

/// Toggle a task checkbox `[ ]` / `[x]` at source range `r`.
pub fn toggle_task(s: &str, r: (usize, usize)) -> Option<Change> {
    let t = s.get(r.0..r.1)?;
    let new = if t.contains('x') || t.contains('X') { "[ ]" } else { "[x]" };
    if !t.starts_with('[') {
        return None;
    }
    Some(Change { range: r, text: new.to_string(), caret: r.1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes() {
        let p = parse_prefix("  - [x] done");
        assert!(p.has_list);
        assert_eq!(p.continuation, "  - [ ] ");
        assert_eq!(p.len, 8);
        let p = parse_prefix("> 9. nine");
        assert!(p.has_list && p.has_quote);
        assert_eq!(p.continuation, "> 10. ");
        let p = parse_prefix("plain");
        assert_eq!(p.len, 0);
        assert!(!parse_prefix("---").has_list);
    }

    #[test]
    fn newline_continues_and_exits_lists() {
        let s = "- one";
        let c = smart_newline(s, (5, 5), true, false);
        assert_eq!(c.text, "\n- ");
        let s = "- one\n- ";
        let c = smart_newline(s, (8, 8), true, false);
        assert_eq!(c.range, (6, 8));
        assert_eq!((c.text.as_str(), c.caret), ("\n", 7));
        let c = smart_newline("Hello", (5, 5), true, false);
        assert_eq!(c.text, "\n\n");
        let c = smart_newline("Hello", (5, 5), false, false);
        assert_eq!(c.text, "\n");
        let c = smart_newline("    code", (8, 8), true, true);
        assert_eq!(c.text, "\n    ");
    }

    #[test]
    fn wrap_toggles() {
        let (c, sel) = toggle_wrap("a word here", (2, 6), "**");
        assert_eq!(c.text, "**word**");
        assert_eq!(sel, (4, 8));
        let s = "a **word** here";
        let (c, _) = toggle_wrap(s, (4, 8), "**");
        assert_eq!(c.range, (2, 10));
        assert_eq!(c.text, "word");
        let (c, _) = toggle_wrap("hello", (2, 2), "*");
        assert_eq!(c.text, "*hello*");
    }

    #[test]
    fn headings() {
        let c = set_heading("Title", 2, 2);
        assert_eq!(c.text, "## Title");
        assert_eq!(c.caret, 5);
        let c = set_heading("## Title", 5, 2);
        assert_eq!(c.text, "Title");
        let c = set_heading("### T", 5, 1);
        assert_eq!(c.text, "# T");
    }

    #[test]
    fn undo_redo_and_coalescing() {
        let mut d = Doc::from_text("");
        for ch in ["H", "i", " ", "y", "o"] {
            let p = d.head;
            d.replace((p, p), ch, EditKind::Typing);
        }
        assert_eq!(d.text, "Hi yo");
        assert!(d.dirty);
        d.undo();
        assert_eq!(d.text, "Hi");
        d.undo();
        assert_eq!(d.text, "");
        assert!(!d.dirty);
        d.redo();
        d.redo();
        assert_eq!(d.text, "Hi yo");
    }

    #[test]
    fn encodings_round_trip() {
        for (enc, bom) in [(Encoding::Utf8, false), (Encoding::Utf8, true), (Encoding::Utf16Le, true), (Encoding::Utf16Be, true)] {
            let bytes = encode("héllo ✓", enc, bom);
            let (t, e, b) = decode(&bytes);
            assert_eq!(t, "héllo ✓");
            assert_eq!(e, enc);
            assert_eq!(b, bom || enc != Encoding::Utf8);
        }
    }

    #[test]
    fn crlf_preserved_on_save() {
        let dir = std::env::temp_dir().join("brisk-test-crlf");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("a.md");
        std::fs::write(&p, b"one\r\ntwo\r\n").unwrap();
        let mut d = Doc::load(&p).unwrap();
        assert_eq!(d.text, "one\ntwo\n");
        assert!(d.crlf);
        d.save_to(&p).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"one\r\ntwo\r\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn indent_list_lines() {
        let c = indent_lines("- a\n- b", (6, 6), false).unwrap();
        assert_eq!((c.range, c.text.as_str()), ((4, 7), "  - b"));
        let c = indent_lines("- a\n  - b", (8, 8), true).unwrap();
        assert_eq!(c.text, "- b");
        let c = indent_lines("- a\n- b", (0, 7), false).unwrap();
        assert_eq!(c.text, "  - a\n  - b");
        assert!(indent_lines("plain", (1, 1), false).is_none());
    }
}
