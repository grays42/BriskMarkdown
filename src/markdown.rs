//! The document model: Markdown source turned into display blocks.
//!
//! The source text is the only truth. Each block carries its display text (UTF-16, ready for
//! DirectWrite) plus a map of segments back to source byte offsets, so a caret or selection can
//! move between what you see and what is stored without ever re-serializing the document.
//!
//! An `exact` segment is display text that is character-for-character the same as the source
//! (prose, code). Anything else (a list bullet, a decoded entity, converted math, the thin space
//! that stands in for an inline code backtick) is an atomic segment: the caret jumps over it.

use crate::highlight::{highlight, Tok};
use pulldown_cmark::{BlockQuoteKind, CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Inline {
    Strong,
    Em,
    Strike,
    Code,
    Link,
    WikiLink,
    Image,
    Math,
    Kbd,
    Mark,
    Sup,
    Sub,
    Underline,
    FootRef,
    Tok(Tok),
    TaskDone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub kind: Inline,
    /// Index into `Block::links` for Link, WikiLink and Image spans.
    pub link: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Seg {
    pub d: u32,
    pub dlen: u32,
    pub s: u32,
    pub slen: u32,
    pub exact: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Para {
    pub text: Vec<u16>,
    pub segs: Vec<Seg>,
    pub spans: Vec<Span>,
}

impl Para {
    pub fn len(&self) -> u32 {
        self.text.len() as u32
    }

    fn push_atomic(&mut self, display: &str, src: Range<usize>) {
        let d = self.text.len() as u32;
        self.text.extend(display.encode_utf16());
        let dlen = self.text.len() as u32 - d;
        self.segs.push(Seg { d, dlen, s: src.start as u32, slen: (src.end - src.start) as u32, exact: false });
    }

    fn push_exact(&mut self, text: &str, src_start: usize) {
        let d = self.text.len() as u32;
        self.text.extend(text.encode_utf16());
        let dlen = self.text.len() as u32 - d;
        // Merge with the previous exact segment when contiguous.
        if let Some(last) = self.segs.last_mut() {
            if last.exact && last.d + last.dlen == d && (last.s + last.slen) as usize == src_start {
                last.dlen += dlen;
                last.slen += text.len() as u32;
                return;
            }
        }
        self.segs.push(Seg { d, dlen, s: src_start as u32, slen: text.len() as u32, exact: true });
    }

    /// Append `text` (what the parser produced) for source bytes `src`, keeping as much of it
    /// exact as possible: identical characters map 1:1, escapes and entities become atomic.
    fn push_aligned(&mut self, source: &str, src: Range<usize>, text: &str) {
        let s = &source[src.clone()];
        if s == text {
            self.push_exact(text, src.start);
            return;
        }
        let sb = s.as_bytes();
        let mut i = 0usize; // byte index into s
        let mut j = 0usize; // byte index into text
        let mut run_s = 0usize;
        let mut run_t = 0usize;
        let flush = |p: &mut Para, run_s: usize, run_t: usize, i: usize, j: usize| {
            if j > run_t {
                p.push_exact(&text[run_t..j], src.start + run_s);
                debug_assert_eq!(i - run_s, j - run_t);
            }
        };
        while j < text.len() {
            if i >= s.len() {
                flush(self, run_s, run_t, i, j);
                self.push_atomic(&text[j..], src.end..src.end);
                return;
            }
            let tc = text[j..].chars().next().unwrap();
            let sc = s[i..].chars().next().unwrap();
            if tc == sc {
                i += sc.len_utf8();
                j += tc.len_utf8();
                continue;
            }
            flush(self, run_s, run_t, i, j);
            if sb[i] == b'\\' && i + 1 < s.len() && s[i + 1..].starts_with(tc) {
                // backslash escape
                self.push_atomic(&tc.to_string(), src.start + i..src.start + i + 1 + tc.len_utf8());
                i += 1 + tc.len_utf8();
                j += tc.len_utf8();
            } else if sb[i] == b'&' {
                if let Some(semi) = s[i..].find(';').filter(|&k| k <= 32) {
                    self.push_atomic(&tc.to_string(), src.start + i..src.start + i + semi + 1);
                    i += semi + 1;
                    j += tc.len_utf8();
                } else {
                    self.push_atomic(&text[j..], src.start + i..src.end);
                    return;
                }
            } else if (sc == '\n' || sc == '\r') && tc == ' ' {
                self.push_atomic(" ", src.start + i..src.start + i + 1);
                i += 1;
                j += 1;
            } else if sc == '\t' && tc == ' ' {
                // tab expanded to spaces: consume the run of spaces
                let mut k = j;
                while k < text.len() && text.as_bytes()[k] == b' ' {
                    k += 1;
                }
                self.push_atomic(&text[j..k], src.start + i..src.start + i + 1);
                i += 1;
                j = k;
            } else {
                self.push_atomic(&text[j..], src.start + i..src.end);
                return;
            }
            run_s = i;
            run_t = j;
        }
        flush(self, run_s, run_t, i, j);
    }

    /// Source range covered by this paragraph's segments.
    pub fn src_span(&self) -> Option<(usize, usize)> {
        let first = self.segs.first()?;
        let last = self.segs.last()?;
        Some((first.s as usize, (last.s + last.slen) as usize))
    }

    /// Display position → source offset. At a boundary between two segments, `left` picks the
    /// end of the left one (typing continues its formatting); otherwise the start of the right.
    pub fn d2s(&self, d: u32, left: bool) -> usize {
        if self.segs.is_empty() {
            return 0;
        }
        let mut best: Option<usize> = None;
        for (k, seg) in self.segs.iter().enumerate() {
            if d < seg.d {
                break;
            }
            if d <= seg.d + seg.dlen {
                if d == seg.d + seg.dlen && !left {
                    // prefer the next segment if it starts here
                    if let Some(n) = self.segs.get(k + 1) {
                        if n.d == d {
                            continue;
                        }
                    }
                }
                if d == seg.d && left && k > 0 && self.segs[k - 1].d + self.segs[k - 1].dlen == d && best.is_some() {
                    return best.unwrap();
                }
                return self.seg_d2s(seg, d);
            }
            best = Some((seg.s + seg.slen) as usize);
        }
        let last = self.segs.last().unwrap();
        (last.s + last.slen) as usize
    }

    fn seg_d2s(&self, seg: &Seg, d: u32) -> usize {
        if d <= seg.d {
            return seg.s as usize;
        }
        if d >= seg.d + seg.dlen {
            return (seg.s + seg.slen) as usize;
        }
        if !seg.exact {
            // inside an atomic segment: snap to the nearer edge
            return if d - seg.d < (seg.d + seg.dlen) - d { seg.s as usize } else { (seg.s + seg.slen) as usize };
        }
        let slice = &self.text[seg.d as usize..d as usize];
        // A position between the halves of a surrogate pair rounds down to the character start.
        let bytes: usize = char::decode_utf16(slice.iter().copied()).map(|c| c.map(|c| c.len_utf8()).unwrap_or(0)).sum();
        seg.s as usize + bytes
    }

    /// Source offset → display position, snapping hidden source to the nearest visible spot.
    pub fn s2d(&self, s: usize) -> u32 {
        let s = s as u32;
        let mut prev_end: Option<u32> = None;
        for seg in &self.segs {
            if s < seg.s {
                // in a hidden gap before this segment
                return match prev_end {
                    Some(pe) => pe,
                    None => seg.d,
                };
            }
            if s <= seg.s + seg.slen {
                if s == seg.s {
                    return seg.d;
                }
                if !seg.exact {
                    return if s == seg.s + seg.slen { seg.d + seg.dlen } else { seg.d };
                }
                // walk characters
                let mut bytes = 0u32;
                let mut units = 0u32;
                let slice = &self.text[seg.d as usize..(seg.d + seg.dlen) as usize];
                for c in char::decode_utf16(slice.iter().copied()) {
                    if seg.s + bytes >= s {
                        break;
                    }
                    let c = c.unwrap_or('\u{fffd}');
                    bytes += c.len_utf8() as u32;
                    units += c.len_utf16() as u32;
                }
                return seg.d + units;
            }
            prev_end = Some(seg.d + seg.dlen);
        }
        self.len()
    }

}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Alert {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl Alert {
    pub fn title(self) -> &'static str {
        match self {
            Alert::Note => "Note",
            Alert::Tip => "Tip",
            Alert::Important => "Important",
            Alert::Warning => "Warning",
            Alert::Caution => "Caution",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Alert::Note => "\u{24D8}",      // ⓘ
            Alert::Tip => "\u{2726}",       // ✦
            Alert::Important => "\u{2757}", // ❗
            Alert::Warning => "\u{26A0}",   // ⚠
            Alert::Caution => "\u{2716}",   // ✖
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Marker {
    Bullet(u8),
    Number(u64),
    Task { checked: bool, src: (u32, u32) },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CtxKind {
    Quote,
    Alert(Alert),
    Item(Marker),
    Footnote(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ctx {
    pub kind: CtxKind,
    pub id: u32,
    /// This block is the first leaf inside the container (draws the list marker, opens a quote).
    pub first: bool,
    /// This block is the last leaf inside the container.
    pub last: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Paragraph,
    Heading(u8),
    Code {
        lang: String,
    },
    Diagram {
        lang: String,
    },
    MathBlock,
    Table {
        aligns: Vec<Align>,
        header_rows: u32,
    },
    Rule,
    Html,
    FrontMatter,
    AlertTitle(Alert),
    /// One source line in Mono mode.
    Line,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Block {
    pub kind: Kind,
    pub src: Range<usize>,
    pub para: Para,
    /// Table cells, row-major. Empty for non-tables.
    pub rows: Vec<Vec<Para>>,
    pub ctx: Vec<Ctx>,
    pub links: Vec<String>,
    /// Paragraph inside a tight list item (uses the list item spacing instead of paragraph spacing).
    pub tight: bool,
    /// An empty paragraph shown only so the caret has a place to sit on a blank line.
    pub virtual_: bool,
}

impl Block {
    fn new(kind: Kind, src: Range<usize>, ctx: Vec<Ctx>) -> Block {
        Block { kind, src, para: Para::default(), rows: Vec::new(), ctx, links: Vec::new(), tight: false, virtual_: false }
    }

    /// Can text be typed here in Markdown mode?
    pub fn editable(&self) -> bool {
        matches!(self.kind, Kind::Paragraph | Kind::Heading(_) | Kind::Code { .. } | Kind::Diagram { .. } | Kind::Line)
    }

    pub fn is_table(&self) -> bool {
        matches!(self.kind, Kind::Table { .. })
    }

    /// All paragraphs in reading order (one for most blocks, every cell for tables).
    pub fn paras(&self) -> Vec<&Para> {
        if self.is_table() {
            self.rows.iter().flatten().collect()
        } else {
            vec![&self.para]
        }
    }

    pub fn para_count(&self) -> usize {
        if self.is_table() {
            self.rows.iter().map(|r| r.len()).sum()
        } else {
            1
        }
    }

    pub fn para_at(&self, i: usize) -> &Para {
        if self.is_table() {
            let mut k = i;
            for r in &self.rows {
                if k < r.len() {
                    return &r[k];
                }
                k -= r.len();
            }
            &self.para
        } else {
            &self.para
        }
    }

}

pub fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_MATH
        | Options::ENABLE_GFM
        | Options::ENABLE_WIKILINKS
}

struct OpenCtx {
    kind: CtxKind,
    id: u32,
    leaves: u32,
}

struct InlineOpen {
    kind: Inline,
    start: u32,
    link: u32,
}

struct Builder<'a> {
    src: &'a str,
    blocks: Vec<Block>,
    stack: Vec<OpenCtx>,
    next_id: u32,
    cur: Option<Block>,
    /// Which paragraph of `cur` receives inline content (tables: the current cell).
    in_cell: bool,
    inl: Vec<InlineOpen>,
    html_inl: Vec<InlineOpen>,
    list_stack: Vec<(Option<u64>, u8)>, // (next number, bullet depth)
    in_image: Option<(u32, u32)>,       // (display start, link index)
    image_has_alt: bool,
    code_lang: Option<String>,
    code_text: String,
    code_start: Option<usize>,
    html_block: Option<(usize, String)>,
    meta: bool,
    tight_list: Vec<bool>,
}

fn utf16_len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

impl<'a> Builder<'a> {
    fn ctx_snapshot(&mut self) -> Vec<Ctx> {
        let mut v = Vec::with_capacity(self.stack.len());
        for c in self.stack.iter_mut() {
            v.push(Ctx { kind: c.kind.clone(), id: c.id, first: c.leaves == 0, last: false });
            c.leaves += 1;
        }
        v
    }

    fn para_mut(&mut self) -> Option<&mut Para> {
        let in_cell = self.in_cell;
        let b = self.cur.as_mut()?;
        if in_cell {
            b.rows.last_mut().and_then(|r| r.last_mut())
        } else {
            Some(&mut b.para)
        }
    }

    fn open_leaf(&mut self, kind: Kind, range: Range<usize>) {
        self.close_leaf();
        let ctx = self.ctx_snapshot();
        let mut b = Block::new(kind, range, ctx);
        b.tight = matches!(b.kind, Kind::Paragraph) && self.tight_list.last().copied().unwrap_or(false) && self.in_item();
        self.cur = Some(b);
    }

    fn in_item(&self) -> bool {
        matches!(self.stack.last().map(|c| &c.kind), Some(CtxKind::Item(_)))
    }

    /// Inline content arriving with no open leaf (tight list items): open an implicit paragraph.
    fn ensure_leaf(&mut self, range: &Range<usize>) {
        if self.cur.is_none() {
            self.open_leaf(Kind::Paragraph, range.clone());
            if let Some(b) = self.cur.as_mut() {
                b.tight = true;
            }
        }
    }

    fn close_leaf(&mut self) {
        if let Some(mut b) = self.cur.take() {
            // Close any inline spans left open.
            let end = b.para.len();
            for o in self.inl.drain(..).chain(self.html_inl.drain(..)) {
                if end > o.start {
                    b.para.spans.push(Span { start: o.start, end, kind: o.kind, link: o.link });
                }
            }
            // Extend the source range to cover everything the segments touch.
            let spans: Vec<(usize, usize)> = b.paras().iter().filter_map(|p| p.src_span()).collect();
            for (s, e) in spans {
                if s < b.src.start {
                    b.src.start = s;
                }
                if e > b.src.end {
                    b.src.end = e;
                }
            }
            // A block's range ends at its last character, not the newline after it.
            let bytes = self.src.as_bytes();
            while b.src.end > b.src.start && matches!(bytes.get(b.src.end - 1), Some(&b'\n') | Some(&b'\r')) {
                b.src.end -= 1;
            }
            // A paragraph that is nothing but display math becomes a math block.
            if b.kind == Kind::Paragraph && b.para.spans.len() == 1 && b.para.spans[0].kind == Inline::Math {
                let sp = b.para.spans[0];
                if sp.start == 0 && sp.end == b.para.len() && self.src[b.src.clone()].trim_start().starts_with("$$") {
                    b.kind = Kind::MathBlock;
                    b.para.spans.clear();
                }
            }
            self.blocks.push(b);
        }
    }

    fn inline_text(&mut self, text: &str, range: Range<usize>) {
        self.ensure_leaf(&range);
        let src = self.src;
        if let Some(p) = self.para_mut() {
            p.push_aligned(src, range, text);
        }
        if self.in_image.is_some() {
            self.image_has_alt = true;
        }
    }

    fn push_span(&mut self, kind: Inline, start: u32, link: u32) {
        let in_cell = self.in_cell;
        if let Some(b) = self.cur.as_mut() {
            let p = if in_cell { b.rows.last_mut().and_then(|r| r.last_mut()) } else { Some(&mut b.para) };
            if let Some(p) = p {
                let end = p.len();
                if end > start {
                    p.spans.push(Span { start, end, kind, link });
                }
            }
        }
    }

    fn display_len(&mut self) -> u32 {
        self.para_mut().map(|p| p.len()).unwrap_or(0)
    }

    fn add_link(&mut self, url: &str) -> u32 {
        match self.cur.as_mut() {
            Some(b) => {
                b.links.push(url.to_string());
                (b.links.len() - 1) as u32
            }
            None => u32::MAX,
        }
    }

    fn inline_html(&mut self, html: &str, range: Range<usize>) {
        let t = html.trim();
        if t.starts_with("<!--") {
            return;
        }
        let closing = t.starts_with("</");
        let name: String = t
            .trim_start_matches('<')
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        let kind = match name.as_str() {
            "br" => {
                self.ensure_leaf(&range);
                if let Some(p) = self.para_mut() {
                    p.push_atomic("\u{2028}", range);
                }
                return;
            }
            "b" | "strong" => Some(Inline::Strong),
            "i" | "em" | "cite" => Some(Inline::Em),
            "s" | "del" | "strike" => Some(Inline::Strike),
            "u" | "ins" => Some(Inline::Underline),
            "code" | "tt" | "samp" => Some(Inline::Code),
            "kbd" => Some(Inline::Kbd),
            "mark" => Some(Inline::Mark),
            "sup" => Some(Inline::Sup),
            "sub" => Some(Inline::Sub),
            "img" => {
                self.ensure_leaf(&range);
                let alt = attr(t, "alt").unwrap_or_else(|| "image".into());
                let url = attr(t, "src").unwrap_or_default();
                let link = self.add_link(&url);
                let start = self.display_len();
                if let Some(p) = self.para_mut() {
                    p.push_atomic(&format!("\u{1F5BC}\u{FE0E}\u{2009}{alt}"), range);
                }
                self.push_span(Inline::Image, start, link);
                return;
            }
            "a" => {
                if closing {
                    Some(Inline::Link)
                } else if let Some(href) = attr(t, "href") {
                    self.ensure_leaf(&range);
                    let link = self.add_link(&href);
                    let start = self.display_len();
                    self.html_inl.push(InlineOpen { kind: Inline::Link, start, link });
                    return;
                } else {
                    None
                }
            }
            _ => None,
        };
        let Some(kind) = kind else { return };
        if closing {
            if let Some(pos) = self.html_inl.iter().rposition(|o| o.kind == kind) {
                let o = self.html_inl.remove(pos);
                self.push_span(o.kind, o.start, o.link);
            }
        } else {
            self.ensure_leaf(&range);
            let start = self.display_len();
            self.html_inl.push(InlineOpen { kind, start, link: u32::MAX });
        }
    }
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        let before_ok = at == 0 || !lower.as_bytes()[at - 1].is_ascii_alphanumeric();
        let rest = lower[at + name.len()..].trim_start();
        if before_ok && rest.starts_with('=') {
            let off = tag.len() - rest.len() + 1;
            let v = tag[off..].trim_start();
            let (q, body) = match v.chars().next() {
                Some(c @ ('"' | '\'')) => (Some(c), &v[1..]),
                _ => (None, v),
            };
            let end = match q {
                Some(c) => body.find(c).unwrap_or(body.len()),
                None => body.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(body.len()),
            };
            return Some(decode_entities(&body[..end]));
        }
        from = at + name.len();
    }
    None
}

pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        if let Some(semi) = after.find(';').filter(|&k| k <= 12) {
            let ent = &after[1..semi];
            let rep = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                "mdash" => Some('—'),
                "ndash" => Some('–'),
                "hellip" => Some('…'),
                "copy" => Some('©'),
                "reg" => Some('®'),
                "trade" => Some('™'),
                "rarr" => Some('→'),
                "larr" => Some('←'),
                "times" => Some('×'),
                "middot" => Some('·'),
                "bull" => Some('•'),
                _ => {
                    if let Some(h) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                        u32::from_str_radix(h, 16).ok().and_then(char::from_u32)
                    } else if let Some(d) = ent.strip_prefix('#') {
                        d.parse::<u32>().ok().and_then(char::from_u32)
                    } else {
                        None
                    }
                }
            };
            if let Some(c) = rep {
                out.push(c);
                rest = &after[semi + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &after[1..];
    }
    out.push_str(rest);
    out
}

/// Readable text from an HTML block: tags removed, block-level tags become line breaks.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    let mut in_style = false;
    while !rest.is_empty() {
        if rest.starts_with("<!--") {
            match rest.find("-->") {
                Some(e) => rest = &rest[e + 3..],
                None => break,
            }
            continue;
        }
        if rest.starts_with('<') {
            let end = rest.find('>').map(|e| e + 1).unwrap_or(rest.len());
            let tag = &rest[..end];
            let name: String = tag
                .trim_start_matches('<')
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            let closing = tag.starts_with("</");
            if name == "style" || name == "script" {
                in_style = !closing;
            }
            match name.as_str() {
                "br" | "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "details" | "summary" | "table"
                | "section" | "header" | "footer" | "ul" | "ol" | "blockquote" | "pre" | "hr" => {
                    if !out.ends_with('\n') && !out.is_empty() {
                        out.push('\n');
                    }
                    if name == "summary" && !closing {
                        out.push_str("\u{25B8} ");
                    }
                }
                "td" | "th" => {
                    if !closing && !out.ends_with('\n') && !out.is_empty() {
                        out.push_str("  \u{2502}  ");
                    }
                }
                "img" => {
                    if let Some(alt) = attr(tag, "alt") {
                        out.push_str(&format!("\u{1F5BC}\u{FE0E} {alt}"));
                    }
                }
                _ => {}
            }
            rest = &rest[end..];
            continue;
        }
        let next = rest.find('<').unwrap_or(rest.len());
        if !in_style {
            let chunk = decode_entities(&rest[..next]);
            for (k, line) in chunk.split('\n').enumerate() {
                if k > 0 && !out.ends_with('\n') {
                    out.push(' ');
                }
                let t = line.trim();
                if !t.is_empty() {
                    if !out.is_empty() && !out.ends_with('\n') && !out.ends_with(' ') && k == 0 && line.starts_with(' ') {
                        out.push(' ');
                    }
                    out.push_str(t);
                }
            }
        }
        rest = &rest[next..];
    }
    // Tidy: trim lines, drop empty runs.
    let mut lines: Vec<&str> = out.lines().map(|l| l.trim()).collect();
    lines.retain(|l| !l.is_empty());
    lines.join("\n")
}

/// Build Markdown-mode blocks. `caret` adds an empty "virtual" paragraph when the caret sits on
/// a blank line between blocks, so it has somewhere to be drawn and typed into.
pub fn build(src: &str, caret: Option<usize>) -> Vec<Block> {
    let mut b = Builder {
        src,
        blocks: Vec::new(),
        stack: Vec::new(),
        next_id: 1,
        cur: None,
        in_cell: false,
        inl: Vec::new(),
        html_inl: Vec::new(),
        list_stack: Vec::new(),
        in_image: None,
        image_has_alt: false,
        code_lang: None,
        code_text: String::new(),
        code_start: None,
        html_block: None,
        meta: false,
        tight_list: Vec::new(),
    };
    let parser = Parser::new_ext(src, options()).into_offset_iter();
    for (ev, range) in parser {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    b.open_leaf(Kind::Paragraph, range);
                }
                Tag::Heading { level, .. } => {
                    b.open_leaf(Kind::Heading(level as u8), range);
                }
                Tag::BlockQuote(kind) => {
                    b.close_leaf();
                    let alert = kind.map(|k| match k {
                        BlockQuoteKind::Note => Alert::Note,
                        BlockQuoteKind::Tip => Alert::Tip,
                        BlockQuoteKind::Important => Alert::Important,
                        BlockQuoteKind::Warning => Alert::Warning,
                        BlockQuoteKind::Caution => Alert::Caution,
                    });
                    let id = b.next_id;
                    b.next_id += 1;
                    b.stack.push(OpenCtx { kind: alert.map(CtxKind::Alert).unwrap_or(CtxKind::Quote), id, leaves: 0 });
                    if let Some(a) = alert {
                        // A title line standing in for the `[!NOTE]` marker.
                        let s = &src[range.clone()];
                        let (ms, me) = match s.find("[!") {
                            Some(i) => {
                                let e = s[i..].find(']').map(|j| i + j + 1).unwrap_or(i + 2);
                                (range.start + i, range.start + e)
                            }
                            None => (range.start, range.start),
                        };
                        b.open_leaf(Kind::AlertTitle(a), ms..me);
                        if let Some(p) = b.para_mut() {
                            p.push_atomic(&format!("{}\u{2002}{}", a.icon(), a.title()), ms..me);
                        }
                        b.close_leaf();
                    }
                }
                Tag::CodeBlock(kind) => {
                    b.close_leaf();
                    let lang = match kind {
                        CodeBlockKind::Fenced(info) => {
                            info.split(|c: char| c.is_whitespace() || c == ',' || c == '{').next().unwrap_or("").to_string()
                        }
                        CodeBlockKind::Indented => String::new(),
                    };
                    let k = if lang.eq_ignore_ascii_case("mermaid")
                        || lang.eq_ignore_ascii_case("plantuml")
                        || lang.eq_ignore_ascii_case("dot")
                    {
                        Kind::Diagram { lang: lang.clone() }
                    } else {
                        Kind::Code { lang: lang.clone() }
                    };
                    b.open_leaf(k, range.clone());
                    b.code_lang = Some(lang);
                    b.code_text.clear();
                    b.code_start = None;
                }
                Tag::HtmlBlock => {
                    b.close_leaf();
                    b.html_block = Some((range.start, String::new()));
                    b.open_leaf(Kind::Html, range);
                }
                Tag::List(start) => {
                    b.close_leaf();
                    let depth = b.list_stack.iter().filter(|(n, _)| n.is_none()).count() as u8;
                    b.list_stack.push((start, depth));
                    // Tight if no blank line between items: pulldown tells us via the absence of
                    // Paragraph tags; we detect per item when inline text arrives without one.
                    b.tight_list.push(true);
                }
                Tag::Item => {
                    b.close_leaf();
                    let marker = match b.list_stack.last_mut() {
                        Some((Some(n), _)) => {
                            let m = Marker::Number(*n);
                            *n += 1;
                            m
                        }
                        Some((None, depth)) => Marker::Bullet(*depth),
                        None => Marker::Bullet(0),
                    };
                    let id = b.next_id;
                    b.next_id += 1;
                    b.stack.push(OpenCtx { kind: CtxKind::Item(marker), id, leaves: 0 });
                }
                Tag::FootnoteDefinition(label) => {
                    b.close_leaf();
                    let id = b.next_id;
                    b.next_id += 1;
                    b.stack.push(OpenCtx { kind: CtxKind::Footnote(label.to_string()), id, leaves: 0 });
                }
                Tag::Table(aligns) => {
                    let aligns = aligns
                        .iter()
                        .map(|a| match a {
                            pulldown_cmark::Alignment::None => Align::None,
                            pulldown_cmark::Alignment::Left => Align::Left,
                            pulldown_cmark::Alignment::Center => Align::Center,
                            pulldown_cmark::Alignment::Right => Align::Right,
                        })
                        .collect();
                    b.open_leaf(Kind::Table { aligns, header_rows: 0 }, range);
                }
                Tag::TableHead | Tag::TableRow => {
                    if let Some(cur) = b.cur.as_mut() {
                        cur.rows.push(Vec::new());
                        if matches!(tag, Tag::TableHead) {
                            if let Kind::Table { header_rows, .. } = &mut cur.kind {
                                *header_rows += 1;
                            }
                        }
                    }
                }
                Tag::TableCell => {
                    if let Some(cur) = b.cur.as_mut() {
                        if cur.rows.is_empty() {
                            cur.rows.push(Vec::new());
                        }
                        let mut p = Para::default();
                        // An empty cell still needs a home for the caret.
                        p.segs.push(Seg { d: 0, dlen: 0, s: range.start as u32, slen: 0, exact: true });
                        cur.rows.last_mut().unwrap().push(p);
                        b.in_cell = true;
                    }
                }
                Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Superscript | Tag::Subscript => {
                    b.ensure_leaf(&range);
                    let kind = match tag {
                        Tag::Emphasis => Inline::Em,
                        Tag::Strong => Inline::Strong,
                        Tag::Strikethrough => Inline::Strike,
                        Tag::Superscript => Inline::Sup,
                        _ => Inline::Sub,
                    };
                    let start = b.display_len();
                    b.inl.push(InlineOpen { kind, start, link: u32::MAX });
                }
                Tag::Link { link_type, dest_url, .. } => {
                    b.ensure_leaf(&range);
                    let link = b.add_link(&dest_url);
                    let start = b.display_len();
                    let kind = if matches!(link_type, LinkType::WikiLink { .. }) { Inline::WikiLink } else { Inline::Link };
                    b.inl.push(InlineOpen { kind, start, link });
                    if matches!(link_type, LinkType::Autolink | LinkType::Email) {
                        // `<https://x>`: the angle brackets are hidden, the text event follows.
                    }
                }
                Tag::Image { dest_url, .. } => {
                    b.ensure_leaf(&range);
                    let link = b.add_link(&dest_url);
                    let start = b.display_len();
                    if let Some(p) = b.para_mut() {
                        p.push_atomic("\u{1F5BC}\u{FE0E}\u{2009}", range.start..range.start + 2.min(range.len()));
                    }
                    b.in_image = Some((start, link));
                    b.image_has_alt = false;
                }
                Tag::MetadataBlock(_) => {
                    b.open_leaf(Kind::FrontMatter, range);
                    b.meta = true;
                }
                Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => {
                    b.open_leaf(Kind::Paragraph, range);
                }
            },
            Event::End(end) => match end {
                TagEnd::Paragraph | TagEnd::Heading(_) => b.close_leaf(),
                TagEnd::BlockQuote(_) => {
                    b.close_leaf();
                    b.stack.pop();
                }
                TagEnd::CodeBlock => {
                    let lang = b.code_lang.take().unwrap_or_default();
                    let text = std::mem::take(&mut b.code_text);
                    if let Some(cur) = b.cur.as_mut() {
                        // Drop the final newline from the display (the fence line follows it).
                        let mut p = std::mem::take(&mut cur.para);
                        if p.text.last() == Some(&(b'\n' as u16)) {
                            p.text.pop();
                            if let Some(last) = p.segs.last_mut() {
                                if last.dlen > 0 {
                                    last.dlen -= 1;
                                    last.slen -= 1;
                                }
                            }
                        }
                        if p.segs.is_empty() {
                            // Empty code block: give the caret the start of the first content line.
                            let s = src[range.clone()].find('\n').map(|i| range.start + i + 1).unwrap_or(range.start);
                            p.segs.push(Seg { d: 0, dlen: 0, s: s as u32, slen: 0, exact: true });
                        }
                        if !lang.is_empty() && !matches!(cur.kind, Kind::Diagram { .. }) {
                            let code: String = String::from_utf16_lossy(&p.text);
                            let _ = &text;
                            let toks = highlight(&lang, &code);
                            if !toks.is_empty() {
                                // byte offsets in `code` → UTF-16 offsets
                                let mut map = Vec::with_capacity(code.len() + 1);
                                let mut u = 0u32;
                                for ch in code.chars() {
                                    for _ in 0..ch.len_utf8() {
                                        map.push(u);
                                    }
                                    u += ch.len_utf16() as u32;
                                }
                                map.push(u);
                                for (r, t) in toks {
                                    if r.end <= code.len() && r.start < r.end {
                                        p.spans.push(Span { start: map[r.start], end: map[r.end], kind: Inline::Tok(t), link: u32::MAX });
                                    }
                                }
                            }
                        }
                        cur.para = p;
                    }
                    b.close_leaf();
                }
                TagEnd::HtmlBlock => {
                    if let Some((start, html)) = b.html_block.take() {
                        let text = html_to_text(&html);
                        if text.trim().is_empty() {
                            b.cur = None; // comments or empty markup: nothing to show
                        } else if let Some(cur) = b.cur.as_mut() {
                            let end = start + html.len();
                            cur.para = Para::default();
                            cur.para.push_atomic(&text, start..end.min(src.len()));
                            b.close_leaf();
                        }
                    }
                }
                TagEnd::List(_) => {
                    b.close_leaf();
                    b.list_stack.pop();
                    b.tight_list.pop();
                }
                TagEnd::Item => {
                    // An empty item still gets a paragraph so the caret can sit after its marker.
                    let empty = b.stack.last().map(|c| c.leaves == 0).unwrap_or(false);
                    if empty && b.cur.is_none() {
                        let s = &src[range.clone()];
                        let after_marker = marker_len(s);
                        let pos = (range.start + after_marker).min(range.end);
                        b.open_leaf(Kind::Paragraph, pos..pos);
                        if let Some(cur) = b.cur.as_mut() {
                            cur.tight = true;
                            cur.para.segs.push(Seg { d: 0, dlen: 0, s: pos as u32, slen: 0, exact: true });
                        }
                    }
                    b.close_leaf();
                    b.stack.pop();
                }
                TagEnd::FootnoteDefinition => {
                    b.close_leaf();
                    b.stack.pop();
                }
                TagEnd::Table => {
                    b.in_cell = false;
                    b.close_leaf();
                }
                TagEnd::TableHead | TagEnd::TableRow => {}
                TagEnd::TableCell => {
                    // Close inline spans in the cell.
                    let end = b.display_len();
                    let opens: Vec<InlineOpen> = b.inl.drain(..).chain(b.html_inl.drain(..)).collect();
                    for o in opens {
                        if end > o.start {
                            b.push_span(o.kind, o.start, o.link);
                        }
                    }
                    b.in_cell = false;
                    // Stay "in table" for the next cell: TableCell start sets it again.
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Superscript | TagEnd::Subscript => {
                    let kind = match end {
                        TagEnd::Emphasis => Inline::Em,
                        TagEnd::Strong => Inline::Strong,
                        TagEnd::Strikethrough => Inline::Strike,
                        TagEnd::Superscript => Inline::Sup,
                        _ => Inline::Sub,
                    };
                    if let Some(pos) = b.inl.iter().rposition(|o| o.kind == kind) {
                        let o = b.inl.remove(pos);
                        b.push_span(o.kind, o.start, o.link);
                    }
                }
                TagEnd::Link => {
                    if let Some(pos) = b.inl.iter().rposition(|o| o.kind == Inline::Link || o.kind == Inline::WikiLink) {
                        let o = b.inl.remove(pos);
                        b.push_span(o.kind, o.start, o.link);
                    }
                }
                TagEnd::Image => {
                    if let Some((start, link)) = b.in_image.take() {
                        if !b.image_has_alt {
                            if let Some(p) = b.para_mut() {
                                p.push_atomic("image", range.end..range.end);
                            }
                        }
                        b.push_span(Inline::Image, start, link);
                    }
                }
                TagEnd::MetadataBlock(_) => {
                    b.meta = false;
                    b.close_leaf();
                }
                TagEnd::DefinitionList | TagEnd::DefinitionListTitle | TagEnd::DefinitionListDefinition => b.close_leaf(),
            },
            Event::Text(text) => {
                if b.code_lang.is_some() {
                    b.code_text.push_str(&text);
                    if b.code_start.is_none() {
                        b.code_start = Some(range.start);
                    }
                    if let Some(p) = b.para_mut() {
                        p.push_aligned(src, range, &text);
                    }
                } else if b.meta {
                    if let Some(p) = b.para_mut() {
                        let t = text.trim_end_matches('\n');
                        p.push_aligned(src, range.start..(range.start + t.len()).min(range.end), t);
                    }
                } else {
                    b.inline_text(&text, range);
                }
            }
            Event::Code(text) => {
                b.ensure_leaf(&range);
                let s = &src[range.clone()];
                let ticks = s.bytes().take_while(|&c| c == b'`').count();
                let inner_start = range.start + ticks;
                let inner_end = range.end.saturating_sub(ticks).max(inner_start);
                let mut cs = inner_start;
                let mut ce = inner_end;
                let inner = &src[cs..ce];
                if inner.len() >= 2 && inner.starts_with(' ') && inner.ends_with(' ') && !inner.trim().is_empty() {
                    cs += 1;
                    ce -= 1;
                }
                let start = b.display_len();
                if let Some(p) = b.para_mut() {
                    p.push_atomic("\u{2009}", range.start..cs);
                    p.push_aligned(src, cs..ce, &text);
                    p.push_atomic("\u{2009}", ce..range.end);
                }
                b.push_span(Inline::Code, start, u32::MAX);
            }
            Event::InlineMath(text) | Event::DisplayMath(text) => {
                b.ensure_leaf(&range);
                let display = src[range.clone()].starts_with("$$");
                let out = crate::latex::latex_to_unicode(&text, display);
                let start = b.display_len();
                if let Some(p) = b.para_mut() {
                    let out = if out.is_empty() { "\u{2009}".to_string() } else { out };
                    p.push_atomic(&out, range);
                }
                b.push_span(Inline::Math, start, u32::MAX);
            }
            Event::Html(html) => {
                if let Some((_, buf)) = b.html_block.as_mut() {
                    buf.push_str(&html);
                } else {
                    b.inline_html(&html, range);
                }
            }
            Event::InlineHtml(html) => b.inline_html(&html, range),
            Event::FootnoteReference(label) => {
                b.ensure_leaf(&range);
                let start = b.display_len();
                if let Some(p) = b.para_mut() {
                    p.push_atomic(&format!("[{label}]"), range);
                }
                b.push_span(Inline::FootRef, start, u32::MAX);
            }
            Event::SoftBreak => {
                if let Some(p) = b.para_mut() {
                    // After a hard line break (e.g. <br>) the newline adds no space, as in HTML.
                    let after_break = p.text.last() == Some(&0x2028);
                    p.push_atomic(if after_break { "" } else { " " }, range);
                }
            }
            Event::HardBreak => {
                if let Some(p) = b.para_mut() {
                    p.push_atomic("\u{2028}", range);
                }
            }
            Event::Rule => {
                b.open_leaf(Kind::Rule, range.clone());
                if let Some(p) = b.para_mut() {
                    p.segs.push(Seg { d: 0, dlen: 0, s: range.start as u32, slen: 0, exact: false });
                }
                b.close_leaf();
            }
            Event::TaskListMarker(checked) => {
                // Swap the innermost item's bullet for a checkbox; the `[ ]` text stays hidden.
                let task = Marker::Task { checked, src: (range.start as u32, range.end as u32) };
                if let Some(top) = b.stack.iter_mut().rev().find(|c| matches!(c.kind, CtxKind::Item(_))) {
                    top.kind = CtxKind::Item(task.clone());
                    let id = top.id;
                    if let Some(cur) = b.cur.as_mut() {
                        for c in cur.ctx.iter_mut() {
                            if c.id == id {
                                c.kind = CtxKind::Item(task.clone());
                            }
                        }
                    }
                }
                if checked {
                    // Style the whole item text as done.
                    let start = b.display_len();
                    b.inl.push(InlineOpen { kind: Inline::TaskDone, start, link: u32::MAX });
                }
            }
        }
    }
    b.close_leaf();
    let mut blocks = b.blocks;

    // Mark `last` flags for containers.
    let n = blocks.len();
    for i in 0..n {
        let next_ids: Vec<u32> = if i + 1 < n { blocks[i + 1].ctx.iter().map(|c| c.id).collect() } else { Vec::new() };
        for c in blocks[i].ctx.iter_mut() {
            c.last = !next_ids.contains(&c.id);
        }
    }

    // Loose lists: an item paragraph that is not tight keeps paragraph spacing. Tight detection
    // above marks implicit paragraphs; explicit Paragraph tags inside items are loose.

    if blocks.is_empty() {
        let mut p = Block::new(Kind::Paragraph, 0..0, Vec::new());
        p.para.segs.push(Seg { d: 0, dlen: 0, s: 0, slen: 0, exact: true });
        p.virtual_ = true;
        blocks.push(p);
    }

    if let Some(c) = caret {
        insert_virtual(&mut blocks, src, c);
    }
    blocks
}

/// Length of a list marker plus the following whitespace at the start of `s` (after indentation).
fn marker_len(s: &str) -> usize {
    let ind = s.len() - s.trim_start_matches([' ', '\t']).len();
    let rest = &s[ind..];
    let mut k = 0;
    let bytes = rest.as_bytes();
    if !bytes.is_empty() && matches!(bytes[0], b'-' | b'*' | b'+') {
        k = 1;
    } else {
        while k < bytes.len() && bytes[k].is_ascii_digit() {
            k += 1;
        }
        if k < bytes.len() && matches!(bytes[k], b'.' | b')') {
            k += 1;
        }
    }
    while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') {
        k += 1;
    }
    (ind + k).min(s.len())
}

/// If `caret` is on a blank line that no block covers, add an empty paragraph there.
fn insert_virtual(blocks: &mut Vec<Block>, src: &str, caret: usize) {
    if caret > src.len() || !src.is_char_boundary(caret) {
        return;
    }
    // Is the caret's line blank?
    let line_start = src[..caret].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = src[caret..].find('\n').map(|i| caret + i).unwrap_or(src.len());
    if !src[line_start..line_end].trim().is_empty() {
        return;
    }
    // Covered by a block (e.g. inside a code block)?
    let idx = blocks.partition_point(|b| b.src.end < caret);
    if let Some(b) = blocks.get(idx) {
        if b.src.start <= caret && caret < b.src.end {
            return;
        }
        // The caret at the very end of a block's range belongs to it unless it's on a new line.
        if b.src.start <= caret && caret == b.src.end && line_start <= b.src.start {
            return;
        }
    }
    if idx > 0 {
        let prev = &blocks[idx - 1];
        if prev.src.end >= caret && !src[prev.src.start..caret].contains('\n') {
            return;
        }
    }
    let mut v = Block::new(Kind::Paragraph, caret..caret, Vec::new());
    v.para.segs.push(Seg { d: 0, dlen: 0, s: caret as u32, slen: 0, exact: true });
    v.virtual_ = true;
    blocks.insert(idx, v);
}

/// Mono mode: one block per source line, display text identical to the source.
pub fn build_mono(src: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut start = 0usize;
    let mut fence: Option<String> = None;
    loop {
        let end = src[start..].find('\n').map(|i| start + i).unwrap_or(src.len());
        let line = &src[start..end];
        let mut b = Block::new(Kind::Line, start..end, Vec::new());
        b.para.push_exact(line, start);
        if b.para.segs.is_empty() {
            b.para.segs.push(Seg { d: 0, dlen: 0, s: start as u32, slen: 0, exact: true });
        }
        // Light syntax colouring for orientation: headings, fences, quotes, list markers.
        let t = line.trim_start();
        let lead = (line.len() - t.len()) as u32;
        let ulen = utf16_len(line);
        let fence_marker = if t.starts_with("```") {
            Some("```")
        } else if t.starts_with("~~~") {
            Some("~~~")
        } else {
            None
        };
        if let Some(fm) = fence_marker {
            match &fence {
                Some(open) if open == fm => fence = None,
                None => fence = Some(fm.to_string()),
                _ => {}
            }
            b.para.spans.push(Span { start: 0, end: ulen, kind: Inline::Tok(Tok::Meta), link: u32::MAX });
        } else if fence.is_some() {
            b.para.spans.push(Span { start: 0, end: ulen, kind: Inline::Tok(Tok::String), link: u32::MAX });
        } else if t.starts_with('#') && t.trim_start_matches('#').starts_with([' ', '\t']) || t.chars().all(|c| c == '#') && !t.is_empty() {
            b.para.spans.push(Span { start: 0, end: ulen, kind: Inline::Tok(Tok::Keyword), link: u32::MAX });
        } else if t.starts_with('>') {
            b.para.spans.push(Span { start: lead, end: lead + 1, kind: Inline::Tok(Tok::Comment), link: u32::MAX });
        } else if let Some(m) = list_marker_width(t) {
            b.para.spans.push(Span { start: lead, end: lead + m as u32, kind: Inline::Tok(Tok::Function), link: u32::MAX });
        } else if t.starts_with("---") && t.chars().all(|c| c == '-') {
            b.para.spans.push(Span { start: 0, end: ulen, kind: Inline::Tok(Tok::Meta), link: u32::MAX });
        }
        blocks.push(b);
        if end >= src.len() {
            break;
        }
        start = end + 1;
    }
    blocks
}

fn list_marker_width(t: &str) -> Option<usize> {
    let b = t.as_bytes();
    if b.len() >= 2 && matches!(b[0], b'-' | b'*' | b'+') && b[1] == b' ' {
        return Some(1);
    }
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && digits < 10 && b.len() > digits + 1 && matches!(b[digits], b'.' | b')') && b[digits + 1] == b' ' {
        return Some(digits + 1);
    }
    None
}


#[cfg(test)]
mod tests {
    use super::*;

    fn check_invariants(src: &str, blocks: &[Block]) {
        for b in blocks {
            for p in b.paras() {
                let mut last_d = 0;
                for seg in &p.segs {
                    assert!(seg.d >= last_d, "segments out of order in {:?}", b.kind);
                    assert!((seg.s + seg.slen) as usize <= src.len());
                    last_d = seg.d + seg.dlen;
                    if seg.exact {
                        let disp = String::from_utf16_lossy(&p.text[seg.d as usize..(seg.d + seg.dlen) as usize]);
                        assert_eq!(&src[seg.s as usize..(seg.s + seg.slen) as usize], disp, "exact segment mismatch");
                    }
                }
                assert!(last_d as usize <= p.text.len());
                // round trip: every display position maps into the source and back near itself
                for d in 0..=p.len() {
                    let s = p.d2s(d, true);
                    assert!(s <= src.len());
                    assert!(src.is_char_boundary(s));
                }
            }
        }
    }

    #[test]
    fn paragraph_markers_hidden() {
        let src = "Hello **bold** and *it* `code` [link](http://x).";
        let b = build(src, None);
        assert_eq!(b.len(), 1);
        let text = String::from_utf16_lossy(&b[0].para.text);
        assert_eq!(text, "Hello bold and it \u{2009}code\u{2009} link.");
        check_invariants(src, &b);
        // caret after "bold" maps inside the strong markers
        let d = text.find("bold").unwrap() as u32 + 4;
        assert_eq!(b[0].para.d2s(d, true), src.find("bold").unwrap() + 4);
    }

    #[test]
    fn lists_quotes_tables_code() {
        let src = "# Title\n\n- one\n- two\n  - nested\n\n> quote\n> more\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn main() {}\n```\n\n---\n\n1. first\n2. second\n";
        let b = build(src, None);
        check_invariants(src, &b);
        let kinds: Vec<String> = b.iter().map(|b| format!("{:?}", b.kind).split(['{', '(']).next().unwrap().trim().to_string()).collect();
        assert_eq!(
            kinds,
            vec!["Heading", "Paragraph", "Paragraph", "Paragraph", "Paragraph", "Table", "Code", "Rule", "Paragraph", "Paragraph"]
        );
        assert!(b[1].tight);
        assert!(matches!(b[1].ctx[0].kind, CtxKind::Item(Marker::Bullet(0))));
        assert!(matches!(b[3].ctx.last().unwrap().kind, CtxKind::Item(Marker::Bullet(1))));
        assert!(matches!(b[4].ctx[0].kind, CtxKind::Quote));
        assert_eq!(b[5].rows.len(), 2);
        assert!(matches!(b[9].ctx[0].kind, CtxKind::Item(Marker::Number(2))));
        let code = String::from_utf16_lossy(&b[6].para.text);
        assert_eq!(code, "fn main() {}");
        assert!(!b[6].para.spans.is_empty());
    }

    #[test]
    fn tasks_alerts_math_frontmatter() {
        let src = "---\ntitle: x\n---\n\n- [ ] todo\n- [x] done\n\n> [!WARNING]\n> careful\n\nInline $x^2$ math.\n\n$$\n\\frac{1}{2}\n$$\n";
        let b = build(src, None);
        check_invariants(src, &b);
        assert_eq!(b[0].kind, Kind::FrontMatter);
        assert!(matches!(b[1].ctx[0].kind, CtxKind::Item(Marker::Task { checked: false, .. })));
        assert!(matches!(b[2].ctx[0].kind, CtxKind::Item(Marker::Task { checked: true, .. })));
        assert_eq!(String::from_utf16_lossy(&b[1].para.text), "todo");
        assert_eq!(b[3].kind, Kind::AlertTitle(Alert::Warning));
        assert!(matches!(b[4].ctx[0].kind, CtxKind::Alert(Alert::Warning)));
        assert!(b[5].para.spans.iter().any(|s| s.kind == Inline::Math));
        assert_eq!(b[6].kind, Kind::MathBlock);
    }

    #[test]
    fn escapes_and_entities_align() {
        let src = "a \\*not\\* em &amp; b &copy; c";
        let b = build(src, None);
        check_invariants(src, &b);
        assert_eq!(String::from_utf16_lossy(&b[0].para.text), "a *not* em & b © c");
        // "not" stays exact
        let p = &b[0].para;
        let d = 3; // after "a *"
        assert_eq!(p.d2s(d, false), src.find("not").unwrap());
    }

    #[test]
    fn virtual_paragraph_on_blank_line() {
        let src = "Hello\n\n\n\nWorld";
        let b = build(src, Some(6));
        assert_eq!(b.len(), 3);
        assert!(b[1].virtual_);
        let b = build(src, Some(5));
        assert_eq!(b.len(), 2);
        let b = build("", Some(0));
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn mono_lines() {
        let src = "# a\n\nb\n";
        let b = build_mono(src);
        assert_eq!(b.len(), 4);
        assert_eq!(b[3].src, 7..7);
        check_invariants(src, &b);
    }

    #[test]
    fn s2d_snaps_hidden_markers() {
        let src = "x **bold** y";
        let b = build(src, None);
        let p = &b[0].para;
        // offset inside the opening ** snaps to start of "bold"
        assert_eq!(p.s2d(3), 2);
        assert_eq!(p.s2d(src.len()), p.len());
    }

    #[test]
    fn html_text() {
        assert_eq!(html_to_text("<div style=\"x\"><b>Title</b><br>Body &amp; more</div>"), "Title\nBody & more");
        assert_eq!(html_to_text("<details><summary>More</summary>\n\nHidden</details>"), "\u{25B8} More\nHidden");
    }
}
