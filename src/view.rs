//! Layout, painting and hit testing for the document area.
//!
//! Coordinates: "doc" units are DIPs at 100% zoom with y measured from the top of the document.
//! The screen shows doc y in [scroll, scroll + height/zoom]. Blocks are laid out lazily: the
//! visible ones first, the rest in idle time; unlaid blocks carry an estimated height and the
//! viewport is re-anchored whenever an estimate above it is replaced by a real height.

use crate::gfx::*;
use crate::markdown::*;
use crate::theme::*;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use windows::core::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Markdown,
    Mono,
}

pub struct LaidPara {
    pub layout: IDWriteTextLayout,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub bgs: Vec<(D2D_RECT_F, El)>,
}

pub enum Deco {
    Rect { r: D2D_RECT_F, fill: Option<Color>, radius: f32 },
    Frame { r: D2D_RECT_F, width: f32, color: Color, radius: f32 },
    Line { x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: Color },
    Text { layout: IDWriteTextLayout, x: f32, y: f32, color: Color },
    Check { x: f32, y: f32, size: f32, checked: bool, color: Color, fill: Color },
}

pub struct Laid {
    pub paras: Vec<LaidPara>,
    pub h: f32,
    pub under: Vec<Deco>,
    pub over: Vec<Deco>,
    /// Clickable task checkboxes: rect (block-relative) and the `[ ]` source range.
    pub tasks: Vec<(D2D_RECT_F, (u32, u32))>,
}

pub struct Geo {
    pub y: f32,
    pub h: f32,
    pub gap: f32,
    pub x: f32,
    pub w: f32,
    pub key: u64,
    pub laid: Option<Laid>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Loc {
    pub block: usize,
    pub para: usize,
    pub d: u32,
}

pub struct View {
    pub mode: Mode,
    pub blocks: Vec<Block>,
    pub geo: Vec<Geo>,
    pub doc_h: f32,
    pub scroll: f32,
    /// Document area size in DIPs (screen, before zoom).
    pub screen_w: f32,
    pub screen_h: f32,
    pub zoom: f32,
    pub wrap: bool,
    pub col_x: f32,
    pub col_w: f32,
    pad_top: f32,
    pad_bottom: f32,
    containers: HashMap<u32, (usize, usize)>,
    pub remaining: usize,
}

fn mono_style(theme: &Theme) -> &Style {
    theme.style(El::Mono)
}

impl View {
    pub fn new() -> View {
        View {
            mode: Mode::Markdown,
            blocks: Vec::new(),
            geo: Vec::new(),
            doc_h: 0.0,
            scroll: 0.0,
            screen_w: 800.0,
            screen_h: 600.0,
            zoom: 1.0,
            wrap: true,
            col_x: 0.0,
            col_w: 600.0,
            pad_top: 0.0,
            pad_bottom: 0.0,
            containers: HashMap::new(),
            remaining: 0,
        }
    }

    /// Doc-space width available for layout.
    pub fn view_w(&self) -> f32 {
        self.screen_w / self.zoom
    }
    pub fn view_h(&self) -> f32 {
        self.screen_h / self.zoom
    }
    pub fn max_scroll(&self) -> f32 {
        (self.doc_h - self.view_h()).max(0.0)
    }
    pub fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    fn compute_column(&mut self, theme: &Theme) {
        let w = self.view_w();
        match self.mode {
            Mode::Markdown => {
                let body = theme.style(El::Body);
                let (pl, pr) = (body.padding[3], body.padding[1]);
                let avail = (w - pl - pr).max(120.0);
                self.col_w = avail.min(theme.max_width.max(200.0));
                self.col_x = ((w - self.col_w) / 2.0).max(pl.min(w * 0.05)).max(8.0);
                if self.col_x + self.col_w > w {
                    self.col_x = ((w - self.col_w) / 2.0).max(4.0);
                }
                self.pad_top = body.padding[0];
                self.pad_bottom = body.padding[2].max(24.0);
            }
            Mode::Mono => {
                let m = mono_style(theme);
                self.col_x = m.padding[3];
                self.col_w = (w - m.padding[3] - m.padding[1]).max(80.0);
                self.pad_top = m.padding[0];
                self.pad_bottom = m.padding[2].max(24.0);
            }
        }
    }

    /// Replace the block list, reusing layouts of blocks whose appearance did not change.
    pub fn set_blocks(&mut self, blocks: Vec<Block>, theme: &Theme) {
        self.compute_column(theme);
        let mut old: HashMap<u64, Vec<Laid>> = HashMap::new();
        for g in self.geo.drain(..) {
            if let Some(l) = g.laid {
                old.entry(g.key).or_default().push(l);
            }
        }
        self.blocks = blocks;
        self.containers.clear();
        for (i, b) in self.blocks.iter().enumerate() {
            for c in &b.ctx {
                self.containers.entry(c.id).and_modify(|e| e.1 = i).or_insert((i, i));
            }
        }
        let mut geo = Vec::with_capacity(self.blocks.len());
        let mut remaining = 0;
        for i in 0..self.blocks.len() {
            let (x, w) = self.block_x_w(i, theme);
            let key = layout_key(&self.blocks[i], w, self.mode, self.wrap);
            let laid = old.get_mut(&key).and_then(|v| v.pop());
            let h = match &laid {
                Some(l) => l.h,
                None => {
                    remaining += 1;
                    self.estimate(i, w, theme)
                }
            };
            let gap = self.gap_before(i, theme);
            geo.push(Geo { y: 0.0, h, gap, x, w, key, laid });
        }
        self.geo = geo;
        self.remaining = remaining;
        self.positions();
    }

    /// Drop every layout (theme, width or zoom-independent style change).
    pub fn invalidate(&mut self, theme: &Theme) {
        for g in self.geo.iter_mut() {
            g.laid = None;
        }
        let blocks = std::mem::take(&mut self.blocks);
        self.geo.clear();
        self.set_blocks(blocks, theme);
    }

    fn positions(&mut self) {
        let mut y = self.pad_top;
        for g in self.geo.iter_mut() {
            y += g.gap;
            g.y = y;
            y += g.h;
        }
        let tail = match (self.mode, self.blocks.last()) {
            (Mode::Markdown, Some(_)) => self.closing_space(self.blocks.len() - 1, None),
            _ => 0.0,
        };
        self.doc_h = y + tail + self.pad_bottom;
    }

    fn indent_of(&self, ctx: &Ctx, theme: &Theme) -> f32 {
        match &ctx.kind {
            CtxKind::Quote => {
                let s = theme.style(El::Blockquote);
                s.border_left.map(|b| b.width).unwrap_or(0.0) + s.padding[3]
            }
            CtxKind::Alert(a) => {
                let s = theme.style(alert_el(*a));
                s.border_left.map(|b| b.width).unwrap_or(0.0) + s.padding[3]
            }
            CtxKind::Item(_) | CtxKind::Footnote(_) => theme.style(El::Li).padding[3],
        }
    }

    fn block_x_w(&self, i: usize, theme: &Theme) -> (f32, f32) {
        if self.mode == Mode::Mono {
            return (self.col_x, if self.wrap { self.col_w } else { 100000.0 });
        }
        let mut x = self.col_x;
        for c in &self.blocks[i].ctx {
            x += self.indent_of(c, theme);
        }
        let w = (self.col_x + self.col_w - x).max(60.0);
        (x, w)
    }

    fn margins(&self, i: usize, theme: &Theme) -> (f32, f32) {
        let b = &self.blocks[i];
        if self.mode == Mode::Mono {
            return (0.0, 0.0);
        }
        let el = match &b.kind {
            Kind::Paragraph => {
                if b.tight {
                    return (0.0, 0.0);
                }
                if b.ctx.iter().any(|c| matches!(c.kind, CtxKind::Footnote(_))) {
                    El::Footnote
                } else {
                    El::P
                }
            }
            Kind::Heading(n) => heading_el(*n),
            Kind::Code { .. } => El::Pre,
            Kind::Diagram { .. } => El::Diagram,
            Kind::MathBlock => El::MathBlock,
            Kind::Table { .. } => El::Table,
            Kind::Rule => El::Hr,
            Kind::Html => El::Html,
            Kind::FrontMatter => El::FrontMatter,
            Kind::AlertTitle(_) => El::CalloutTitle,
            Kind::Line => return (0.0, 0.0),
        };
        let s = theme.style(el);
        let mut top = s.margin[0];
        let mut bottom = s.margin[2];
        // First and last children of a quote sit flush with its padding.
        if let Some(c) = b.ctx.iter().rev().find(|c| matches!(c.kind, CtxKind::Quote | CtxKind::Alert(_))) {
            if c.first {
                top = 0.0;
            }
            if c.last {
                bottom = 0.0;
            }
        }
        if i == 0 {
            top = 0.0;
        }
        (top, bottom)
    }

    fn ctx_box(&self, c: &Ctx, theme: &Theme) -> (f32, f32, f32, f32) {
        // (margin_top, margin_bottom, padding_top, padding_bottom)
        let s = match &c.kind {
            CtxKind::Quote => theme.style(El::Blockquote),
            CtxKind::Alert(a) => theme.style(alert_el(*a)),
            CtxKind::Item(_) => theme.style(El::Li),
            CtxKind::Footnote(_) => theme.style(El::Footnote),
        };
        let pads = matches!(c.kind, CtxKind::Quote | CtxKind::Alert(_));
        (s.margin[0], s.margin[2], if pads { s.padding[0] } else { 0.0 }, if pads { s.padding[2] } else { 0.0 })
    }

    /// Space after block `i` for the containers it closes (relative to the next block).
    fn closing_space(&self, i: usize, next: Option<&Block>) -> f32 {
        let _ = (i, next);
        0.0
    }

    fn gap_before(&self, i: usize, theme: &Theme) -> f32 {
        if self.mode == Mode::Mono {
            return 0.0;
        }
        let cur = &self.blocks[i];
        let (cur_top, _) = self.margins(i, theme);
        let mut top_m = cur_top;
        let mut pad = 0.0;
        let prev_ids: Vec<u32> = if i > 0 { self.blocks[i - 1].ctx.iter().map(|c| c.id).collect() } else { Vec::new() };
        let cur_ids: Vec<u32> = cur.ctx.iter().map(|c| c.id).collect();
        for c in &cur.ctx {
            if !prev_ids.contains(&c.id) {
                let (mt, _, pt, _) = self.ctx_box(c, theme);
                if i > 0 {
                    top_m = top_m.max(mt);
                }
                pad += pt;
            }
        }
        if i == 0 {
            return pad;
        }
        let (_, prev_bottom) = self.margins(i - 1, theme);
        let mut bot_m = prev_bottom;
        for c in &self.blocks[i - 1].ctx {
            if !cur_ids.contains(&c.id) {
                let (_, mb, _, pb) = self.ctx_box(c, theme);
                bot_m = bot_m.max(mb);
                pad += pb;
            }
        }
        top_m.max(bot_m) + pad
    }

    fn estimate(&self, i: usize, w: f32, theme: &Theme) -> f32 {
        let b = &self.blocks[i];
        let st = self.base_style(b, theme);
        let chars = b.para.text.len() as f32;
        let per_line = (w / (st.size * 0.52)).max(8.0);
        match &b.kind {
            Kind::Table { .. } => b.rows.len() as f32 * (st.line_height + 16.0) + 2.0,
            Kind::Rule => 1.0,
            Kind::Code { .. } | Kind::Diagram { .. } | Kind::FrontMatter | Kind::Html => {
                let lines = b.para.text.iter().filter(|&&c| c == 10).count() as f32 + 1.0;
                lines * st.line_height + st.padding[0] + st.padding[2]
            }
            _ => ((chars / per_line).ceil().max(1.0)) * st.line_height,
        }
    }

    fn base_style<'t>(&self, b: &Block, theme: &'t Theme) -> &'t Style {
        if self.mode == Mode::Mono {
            return mono_style(theme);
        }
        theme.style(match &b.kind {
            Kind::Paragraph => {
                if b.ctx.iter().any(|c| matches!(c.kind, CtxKind::Footnote(_))) {
                    El::Footnote
                } else {
                    El::P
                }
            }
            Kind::Heading(n) => heading_el(*n),
            Kind::Code { .. } => El::Pre,
            Kind::Diagram { .. } => El::Diagram,
            Kind::MathBlock => El::MathBlock,
            Kind::Table { .. } => El::Td,
            Kind::Rule => El::Hr,
            Kind::Html => El::Html,
            Kind::FrontMatter => El::FrontMatter,
            Kind::AlertTitle(_) => El::CalloutTitle,
            Kind::Line => El::Mono,
        })
    }

    // ---------------------------------------------------------------- layout

    pub fn ensure_laid(&mut self, i: usize, gfx: &Gfx, theme: &Theme) -> bool {
        if i >= self.geo.len() || self.geo[i].laid.is_some() {
            return false;
        }
        let w = self.geo[i].w;
        let laid = self.lay_block(i, w, gfx, theme);
        let g = &mut self.geo[i];
        let changed = (laid.h - g.h).abs() > 0.01;
        g.h = laid.h;
        g.laid = Some(laid);
        self.remaining = self.remaining.saturating_sub(1);
        changed
    }

    /// Index of the block at doc y (the block whose gap or body contains y).
    pub fn block_at_y(&self, y: f32) -> usize {
        if self.geo.is_empty() {
            return 0;
        }
        let i = self.geo.partition_point(|g| g.y + g.h < y);
        i.min(self.geo.len() - 1)
    }

    /// Lay out everything visible, keeping the first visible block anchored on screen.
    pub fn prepare(&mut self, gfx: &Gfx, theme: &Theme) {
        for _ in 0..50 {
            let top = self.scroll;
            let bottom = self.scroll + self.view_h();
            let first = self.block_at_y(top);
            let anchor_off = self.geo.get(first).map(|g| g.y - self.scroll).unwrap_or(0.0);
            let mut changed = false;
            let mut i = first;
            while i < self.geo.len() {
                if self.geo[i].y - self.geo[i].gap > bottom + 200.0 {
                    break;
                }
                changed |= self.ensure_laid(i, gfx, theme);
                i += 1;
            }
            // a little above, so scrolling up is smooth
            let mut j = first;
            let mut above = 0.0;
            while j > 0 && above < 400.0 {
                j -= 1;
                above += self.geo[j].h;
                changed |= self.ensure_laid(j, gfx, theme);
            }
            if !changed {
                break;
            }
            self.positions();
            if let Some(g) = self.geo.get(first) {
                self.scroll = g.y - anchor_off;
            }
            self.clamp_scroll();
        }
    }

    /// Lay out blocks that are not visible yet, for up to `budget_ms`. Returns true if more remain.
    pub fn idle(&mut self, gfx: &Gfx, theme: &Theme, budget_ms: f64) -> bool {
        if self.remaining == 0 {
            return false;
        }
        let t0 = std::time::Instant::now();
        let first = self.block_at_y(self.scroll);
        let anchor_off = self.geo.get(first).map(|g| g.y - self.scroll).unwrap_or(0.0);
        let mut any = false;
        for i in 0..self.geo.len() {
            if self.geo[i].laid.is_none() {
                any |= self.ensure_laid(i, gfx, theme);
                if t0.elapsed().as_secs_f64() * 1000.0 > budget_ms {
                    break;
                }
            }
        }
        if any {
            self.positions();
            if let Some(g) = self.geo.get(first) {
                self.scroll = g.y - anchor_off;
            }
            self.clamp_scroll();
        }
        self.remaining = self.geo.iter().filter(|g| g.laid.is_none()).count();
        self.remaining > 0
    }

    /// Doc y positions where a page may break without cutting a line: block tops and the top of
    /// every laid-out line.
    pub fn break_candidates(&self) -> Vec<f32> {
        let mut v = Vec::new();
        for g in &self.geo {
            v.push(g.y - g.gap * 0.5);
            let Some(l) = &g.laid else { continue };
            for p in &l.paras {
                let mut count = 0u32;
                unsafe {
                    let _ = p.layout.GetLineMetrics(None, &mut count);
                }
                if count <= 1 {
                    continue;
                }
                let mut lines = vec![DWRITE_LINE_METRICS::default(); count as usize];
                unsafe {
                    let _ = p.layout.GetLineMetrics(Some(&mut lines), &mut count);
                }
                let mut y = g.y + p.y;
                for lm in lines.iter().take(count as usize) {
                    v.push(y);
                    y += lm.height;
                }
            }
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v.dedup_by(|a, b| (*a - *b).abs() < 0.5);
        v
    }

    pub fn lay_all(&mut self, gfx: &Gfx, theme: &Theme) {
        for i in 0..self.geo.len() {
            self.ensure_laid(i, gfx, theme);
        }
        self.remaining = 0;
        self.positions();
    }

    fn container_color(&self, b: &Block, theme: &Theme) -> Option<Color> {
        for c in b.ctx.iter().rev() {
            match &c.kind {
                CtxKind::Quote => return theme.props(El::Blockquote).color,
                CtxKind::Alert(a) => return theme.props(alert_el(*a)).color.or(theme.props(El::Blockquote).color),
                _ => {}
            }
        }
        None
    }

    fn lay_block(&self, i: usize, w: f32, gfx: &Gfx, theme: &Theme) -> Laid {
        let b = &self.blocks[i];
        let st = self.base_style(b, theme).clone();
        let mut laid = Laid { paras: Vec::new(), h: 0.0, under: Vec::new(), over: Vec::new(), tasks: Vec::new() };
        let family = gfx.pick_family(&st.families);
        match &b.kind {
            Kind::Line => {
                let wrap = if self.wrap { Wrap::Word } else { Wrap::None };
                let lp = lay_para(gfx, theme, &b.para, &st, &family, st.color, w, wrap, Align::Left, &b.links);
                laid.h = lp.h;
                laid.paras.push(lp);
            }
            Kind::Paragraph | Kind::Heading(_) | Kind::AlertTitle(_) | Kind::MathBlock => {
                let mut color = self.container_color(b, theme).unwrap_or(st.color);
                if let Kind::Heading(n) = b.kind {
                    color = theme.props(heading_el(n)).color.unwrap_or(color);
                }
                if let Kind::AlertTitle(a) = b.kind {
                    color = theme.style(alert_el(a)).border_left.map(|bd| bd.color).unwrap_or(color);
                }
                let align = if b.kind == Kind::MathBlock { Align::Center } else { Align::Left };
                let mut lp = lay_para(gfx, theme, &b.para, &st, &family, color, w, Wrap::Word, align, &b.links);
                if let Kind::Heading(_) = b.kind {
                    if let Some(bd) = st.border_bottom {
                        let y = lp.h + st.padding[2];
                        laid.under.push(Deco::Line {
                            x0: 0.0,
                            y0: y + bd.width / 2.0,
                            x1: w,
                            y1: y + bd.width / 2.0,
                            width: bd.width,
                            color: bd.color,
                        });
                        laid.h = y + bd.width;
                    }
                }
                if b.kind == Kind::MathBlock {
                    lp.y += st.padding[0];
                    laid.h = lp.h + st.padding[0] + st.padding[2];
                }
                laid.h = laid.h.max(lp.h);
                laid.paras.push(lp);
                self.add_markers(b, &mut laid, &st, &family, theme, gfx);
            }
            Kind::Code { lang } | Kind::Diagram { lang } => {
                let bl = st.border_left.map(|b| b.width).unwrap_or(0.0);
                let br = st.border_right.map(|b| b.width).unwrap_or(0.0);
                let bt = st.border_top.map(|b| b.width).unwrap_or(0.0);
                let bb = st.border_bottom.map(|b| b.width).unwrap_or(0.0);
                let label = if matches!(b.kind, Kind::Diagram { .. }) {
                    format!("{lang} diagram \u{00B7} shown as source")
                } else {
                    lang.to_ascii_lowercase()
                };
                let lab_style = theme.style(El::CodeLang);
                let mut top = bt + st.padding[0];
                if !label.is_empty() {
                    let lf = gfx.pick_family(&lab_style.families);
                    let ll = gfx.simple_layout(&label, &lf, lab_style.size, lab_style.weight, lab_style.italic, w);
                    laid.over.push(Deco::Text { layout: ll, x: bl + st.padding[3], y: bt + st.padding[0] * 0.55, color: lab_style.color });
                    top += lab_style.size * 1.6;
                }
                let inner = (w - bl - br - st.padding[1] - st.padding[3]).max(20.0);
                let mut lp = lay_para(gfx, theme, &b.para, &st, &family, st.color, inner, Wrap::Char, Align::Left, &b.links);
                lp.x = bl + st.padding[3];
                lp.y = top;
                let h = top + lp.h + st.padding[2] + bb;
                push_box(&mut laid.under, &st, 0.0, 0.0, w, h);
                laid.h = h;
                laid.paras.push(lp);
                self.add_markers(b, &mut laid, &st, &family, theme, gfx);
            }
            Kind::Html | Kind::FrontMatter => {
                let bl = st.border_left.map(|b| b.width).unwrap_or(0.0);
                let inner = (w - bl - st.padding[1] - st.padding[3]).max(20.0);
                let color = self.container_color(b, theme).unwrap_or(st.color);
                let mut lp = lay_para(gfx, theme, &b.para, &st, &family, color, inner, Wrap::Char, Align::Left, &b.links);
                lp.x = bl + st.padding[3];
                lp.y = st.padding[0];
                let h = lp.h + st.padding[0] + st.padding[2];
                push_box(&mut laid.under, &st, 0.0, 0.0, w, h);
                laid.h = h;
                laid.paras.push(lp);
            }
            Kind::Rule => {
                let s = theme.style(El::Hr);
                let bw = s.border_top.map(|b| b.width).unwrap_or(1.0);
                let c = s.border_top.map(|b| b.color).unwrap_or(s.color);
                laid.under.push(Deco::Line { x0: 0.0, y0: bw / 2.0, x1: w, y1: bw / 2.0, width: bw, color: c });
                laid.h = bw;
                let lp = lay_para(gfx, theme, &b.para, &st, &family, st.color, w, Wrap::Word, Align::Left, &b.links);
                laid.paras.push(LaidPara { h: bw, ..lp });
            }
            Kind::Table { aligns, header_rows } => {
                self.lay_table(b, aligns, *header_rows, w, gfx, theme, &mut laid);
                self.add_markers(b, &mut laid, &st, &family, theme, gfx);
            }
        }
        laid
    }

    #[allow(clippy::too_many_arguments)]
    fn lay_table(&self, b: &Block, aligns: &[Align], header_rows: u32, w: f32, gfx: &Gfx, theme: &Theme, laid: &mut Laid) {
        let td = theme.style(El::Td).clone();
        let th = theme.style(El::Th).clone();
        let tf = gfx.pick_family(&td.families);
        let hf = gfx.pick_family(&th.families);
        let ncols = b.rows.iter().map(|r| r.len()).max().unwrap_or(0).max(1);
        let border = td.border_left.map(|b| b.width).unwrap_or(1.0);
        let bcolor = td.border_left.map(|b| b.color).unwrap_or(td.color);
        let padx = td.padding[1] + td.padding[3];
        let pady_t = td.padding[0];
        let pady_b = td.padding[2];
        // One layout per cell: measured unwrapped first, then narrowed to its column.
        let mut cells: Vec<Vec<LaidPara>> = Vec::with_capacity(b.rows.len());
        let mut maxw = vec![0f32; ncols];
        for (ri, row) in b.rows.iter().enumerate() {
            let is_head = (ri as u32) < header_rows;
            let (s, f) = if is_head { (&th, &hf) } else { (&td, &tf) };
            let mut out = Vec::with_capacity(row.len());
            for (ci, cell) in row.iter().enumerate() {
                let al = aligns.get(ci).copied().unwrap_or(Align::None);
                let lp = lay_para(gfx, theme, cell, s, f, s.color, 100000.0, Wrap::None, al, &b.links);
                maxw[ci] = maxw[ci].max(lp.w.ceil() + 1.0);
                out.push(lp);
            }
            cells.push(out);
        }
        let avail = (w - border * (ncols as f32 + 1.0) - padx * ncols as f32).max(ncols as f32 * 24.0);
        let sum_max: f32 = maxw.iter().sum();
        let widths: Vec<f32> = if sum_max <= avail {
            maxw.clone()
        } else {
            // Only now pay for minimum widths (the longest unbreakable word per column).
            let mut minw = vec![0f32; ncols];
            for row in &cells {
                for (ci, lp) in row.iter().enumerate() {
                    let mn = unsafe {
                        lp.layout
                            .SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)
                            .ok()
                            .and_then(|_| lp.layout.DetermineMinWidth().ok())
                            .unwrap_or(0.0)
                    };
                    minw[ci] = minw[ci].max(mn.ceil() + 1.0).min(220.0);
                }
            }
            let sum_min: f32 = minw.iter().sum();
            if sum_min >= avail {
                minw.iter().map(|m| (m / sum_min.max(1.0) * avail).max(16.0)).collect()
            } else {
                let extra = avail - sum_min;
                let flex: f32 = maxw.iter().zip(&minw).map(|(a, b)| (a - b).max(0.0)).sum::<f32>().max(1.0);
                maxw.iter().zip(&minw).map(|(a, m)| m + (a - m).max(0.0) / flex * extra).collect()
            }
        };
        let mut y = border;
        let mut row_tops = Vec::new();
        let table_w = widths.iter().sum::<f32>() + padx * ncols as f32 + border * (ncols as f32 + 1.0);
        for (ri, row) in cells.into_iter().enumerate() {
            let is_head = (ri as u32) < header_rows;
            let mut x = border;
            let mut row_h = 0f32;
            let mut ci = 0;
            for mut lp in row {
                let cw = widths[ci];
                unsafe {
                    let _ = lp.layout.SetWordWrapping(DWRITE_WORD_WRAPPING_EMERGENCY_BREAK);
                    let _ = lp.layout.SetMaxWidth(cw.max(1.0));
                }
                lp.w = cw;
                let m = layout_metrics(&lp.layout);
                if b.rows[ri].get(ci).map(|c| c.len() > 0).unwrap_or(false) {
                    lp.h = m.height;
                }
                // Inline backgrounds (code spans) moved when the text rewrapped.
                lp.bgs = relayout_bgs(&lp, &b.rows[ri][ci]);
                lp.x = x + td.padding[3];
                lp.y = y + pady_t;
                row_h = row_h.max(lp.h);
                laid.paras.push(lp);
                x += cw + padx + border;
                ci += 1;
            }
            let rh = row_h + pady_t + pady_b;
            if is_head {
                if let Some(bg) = th.background {
                    laid.under.push(Deco::Rect { r: rect(0.0, y, table_w, y + rh), fill: Some(bg), radius: 0.0 });
                }
            } else if let Some(bg) = td.background {
                laid.under.push(Deco::Rect { r: rect(0.0, y, table_w, y + rh), fill: Some(bg), radius: 0.0 });
            }
            row_tops.push(y);
            y += rh + border;
        }
        // grid
        let half = border / 2.0;
        let mut ylines = row_tops.clone();
        ylines.push(y);
        for yy in ylines {
            laid.under.push(Deco::Line { x0: 0.0, y0: yy - half, x1: table_w, y1: yy - half, width: border, color: bcolor });
        }
        let mut x = border;
        laid.under.push(Deco::Line { x0: half, y0: 0.0, x1: half, y1: y, width: border, color: bcolor });
        for cw in &widths {
            x += cw + padx;
            laid.under.push(Deco::Line { x0: x + half, y0: 0.0, x1: x + half, y1: y, width: border, color: bcolor });
            x += border;
        }
        laid.h = y;
    }

    /// List bullets, numbers, checkboxes and footnote labels for the first block of an item.
    fn add_markers(&self, b: &Block, laid: &mut Laid, st: &Style, family: &str, theme: &Theme, gfx: &Gfx) {
        if laid.paras.is_empty() {
            return;
        }
        // x of each container's content edge relative to this block's x.
        let mut edges = Vec::new();
        let mut acc = 0.0;
        for c in &b.ctx {
            acc += self.indent_of(c, theme);
            edges.push(acc);
        }
        let total = acc;
        let mk = theme.style(El::Marker);
        let li = theme.style(El::Li);
        let first = &laid.paras[0];
        let line_h = first_line_height(&first.layout).unwrap_or(st.line_height);
        for (k, c) in b.ctx.iter().enumerate() {
            if !c.first {
                continue;
            }
            let content_x = edges[k] - total; // negative: left of this block's x
            let marker_color = mk.color;
            match &c.kind {
                CtxKind::Item(Marker::Bullet(depth)) => {
                    let glyph = match depth % 3 {
                        0 => "\u{2022}",
                        1 => "\u{25E6}",
                        _ => "\u{25AA}",
                    };
                    let l = marker_layout(gfx, glyph, family, st, first, li.padding[3]);
                    unsafe {
                        let _ = l.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
                    }
                    let mw = li.padding[3];
                    laid.over.push(Deco::Text { layout: l, x: first.x + content_x - mw * 0.92, y: first.y, color: marker_color });
                }
                CtxKind::Item(Marker::Number(n)) => {
                    let text = format!("{n}.");
                    let mw = li.padding[3] + 40.0;
                    let l = marker_layout(gfx, &text, family, st, first, mw);
                    unsafe {
                        let _ = l.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING);
                    }
                    laid.over.push(Deco::Text { layout: l, x: first.x + content_x - mw - st.size * 0.4, y: first.y, color: marker_color });
                }
                CtxKind::Item(Marker::Task { checked, src }) => {
                    let size = (st.size * 0.95).round();
                    let x = first.x + content_x - size - st.size * 0.45;
                    let y = first.y + (line_h - size) / 2.0;
                    laid.tasks.push((rect(x - 3.0, y - 3.0, x + size + 3.0, y + size + 3.0), *src));
                    let task = theme.style(El::Task);
                    let accent = theme.props(El::Body).caret_color.unwrap_or(task.color);
                    laid.over.push(Deco::Check {
                        x,
                        y,
                        size,
                        checked: *checked,
                        color: if *checked { accent } else { task.color },
                        fill: accent,
                    });
                }
                CtxKind::Footnote(label) => {
                    let text = format!("{label}.");
                    let mw = li.padding[3] + 40.0;
                    let l = marker_layout(gfx, &text, family, st, first, mw);
                    unsafe {
                        let _ = l.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING);
                    }
                    laid.over.push(Deco::Text { layout: l, x: first.x + content_x - mw - st.size * 0.4, y: first.y, color: marker_color });
                }
                _ => {}
            }
        }
    }

    // ---------------------------------------------------------------- painting

    pub fn paint(
        &self,
        gfx: &Gfx,
        theme: &Theme,
        rt: &ID2D1RenderTarget,
        top: f32,
        sel: (usize, usize),
        caret: Option<(f32, f32, f32)>,
        finds: &[(usize, usize)],
        current_find: Option<usize>,
    ) {
        unsafe {
            let z = self.zoom;
            let m = windows_numerics::Matrix3x2 { M11: z, M12: 0.0, M21: 0.0, M22: z, M31: 0.0, M32: top - self.scroll * z };
            rt.SetTransform(&m);
            let vt = self.scroll;
            let vb = self.scroll + self.view_h();
            let first = self.block_at_y(vt);
            // Quote bars and callout boxes, once per container.
            let mut drawn: Vec<u32> = Vec::new();
            let mut i = first.saturating_sub(1);
            let sel_color = theme.props(El::Selection).background.unwrap_or(Color::rgb(80, 120, 200).with_alpha(0.3));
            let find_color = theme.props(El::Find).background.unwrap_or(Color::rgb(220, 180, 60).with_alpha(0.35));
            let cur_color = theme.props(El::FindCurrent).background.unwrap_or(find_color);
            while i < self.geo.len() {
                let g = &self.geo[i];
                if g.y - g.gap > vb {
                    break;
                }
                if self.mode == Mode::Markdown {
                    for c in &self.blocks[i].ctx {
                        if drawn.contains(&c.id) {
                            continue;
                        }
                        drawn.push(c.id);
                        self.paint_container(gfx, theme, rt, c, i);
                    }
                }
                if g.y + g.h >= vt {
                    if let Some(l) = &g.laid {
                        for d in &l.under {
                            paint_deco(gfx, rt, d, g.x, g.y);
                        }
                        // find and selection highlights
                        for (k, &(a, b)) in finds.iter().enumerate() {
                            if b < self.blocks[i].src.start || a > self.blocks[i].src.end {
                                continue;
                            }
                            let c = if Some(k) == current_find { &cur_color } else { &find_color };
                            self.paint_range(gfx, rt, i, l, a, b, c);
                        }
                        if sel.1 > sel.0 && sel.1 >= self.blocks[i].src.start && sel.0 <= self.blocks[i].src.end {
                            self.paint_range(gfx, rt, i, l, sel.0, sel.1, &sel_color);
                        }
                        for p in &l.paras {
                            for (r, el) in &p.bgs {
                                let s = theme.style(*el);
                                let rr = rect(g.x + p.x + r.left, g.y + p.y + r.top, g.x + p.x + r.right, g.y + p.y + r.bottom);
                                if let Some(bg) = s.background {
                                    let rounded = D2D1_ROUNDED_RECT { rect: rr, radiusX: s.radius, radiusY: s.radius };
                                    rt.FillRoundedRectangle(&rounded, &gfx.brush(&bg));
                                }
                                if let Some(bd) = s.border_top {
                                    let rounded =
                                        D2D1_ROUNDED_RECT { rect: inset(rr, bd.width / 2.0), radiusX: s.radius, radiusY: s.radius };
                                    rt.DrawRoundedRectangle(&rounded, &gfx.brush(&bd.color), bd.width, None);
                                }
                            }
                            rt.DrawTextLayout(
                                pt(g.x + p.x, g.y + p.y),
                                &p.layout,
                                &gfx.brush(&theme.style(El::Body).color),
                                D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                            );
                        }
                        for d in &l.over {
                            paint_deco(gfx, rt, d, g.x, g.y);
                        }
                    }
                }
                i += 1;
            }
            if let Some((x, y, h)) = caret {
                let w = (1.6 / z).max(1.0);
                rt.FillRectangle(&rect(x - w / 2.0, y, x + w / 2.0, y + h), &gfx.brush(&theme.caret));
            }
            rt.SetTransform(&windows_numerics::Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 });
        }
    }

    fn paint_container(&self, gfx: &Gfx, theme: &Theme, rt: &ID2D1RenderTarget, c: &Ctx, at: usize) {
        let (s_el, alert) = match &c.kind {
            CtxKind::Quote => (El::Blockquote, false),
            CtxKind::Alert(a) => (alert_el(*a), true),
            _ => return,
        };
        let Some(&(fi, la)) = self.containers.get(&c.id) else { return };
        let st = theme.style(s_el);
        let (_, _, pt_, pb) = self.ctx_box(c, theme);
        let top = self.geo[fi].y - pt_;
        let bottom = self.geo[la].y + self.geo[la].h + pb;
        // x: this container's left edge = block x minus the indents of this and inner containers.
        let b = &self.blocks[at];
        let mut x = self.col_x;
        for cc in &b.ctx {
            if cc.id == c.id {
                break;
            }
            x += self.indent_of(cc, theme);
        }
        let right = self.col_x + self.col_w;
        unsafe {
            if let Some(bg) = st.background {
                let rr = D2D1_ROUNDED_RECT { rect: rect(x, top, right, bottom), radiusX: st.radius, radiusY: st.radius };
                rt.FillRoundedRectangle(&rr, &gfx.brush(&bg));
            } else if alert {
                if let Some(bd) = st.border_left {
                    let rr = D2D1_ROUNDED_RECT { rect: rect(x, top, right, bottom), radiusX: 4.0, radiusY: 4.0 };
                    rt.FillRoundedRectangle(&rr, &gfx.brush(&bd.color.with_alpha(0.07)));
                }
            }
            if let Some(bd) = st.border_left {
                rt.FillRectangle(&rect(x, top, x + bd.width, bottom), &gfx.brush(&bd.color));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_range(&self, gfx: &Gfx, rt: &ID2D1RenderTarget, i: usize, l: &Laid, a: usize, b: usize, color: &Color) {
        let g = &self.geo[i];
        let block = &self.blocks[i];
        for (k, p) in block.paras().iter().enumerate() {
            let Some(lp) = l.paras.get(k) else { break };
            let Some((ps, pe)) = p.src_span() else { continue };
            if b < ps || a > pe {
                continue;
            }
            let d0 = p.s2d(a.max(ps));
            let d1 = p.s2d(b.min(pe));
            let whole_empty = p.len() == 0 && a <= ps && b > pe;
            if d1 <= d0 && !whole_empty {
                continue;
            }
            for r in hit_rects(&lp.layout, d0, d1.saturating_sub(d0).max(if whole_empty { 0 } else { 0 })) {
                let mut rr = rect(g.x + lp.x + r.left, g.y + lp.y + r.top, g.x + lp.x + r.right, g.y + lp.y + r.bottom);
                if rr.right - rr.left < 3.0 {
                    rr.right = rr.left + 4.0;
                }
                unsafe { rt.FillRectangle(&rr, &gfx.brush(color)) };
            }
        }
    }

    // ---------------------------------------------------------------- hit testing & mapping

    pub fn laid_para(&self, loc: Loc) -> Option<(&Geo, &LaidPara)> {
        let g = self.geo.get(loc.block)?;
        let lp = g.laid.as_ref()?.paras.get(loc.para)?;
        Some((g, lp))
    }

    /// Caret rectangle (x, y, height) in doc units.
    pub fn caret_rect(&self, loc: Loc) -> Option<(f32, f32, f32)> {
        let (g, lp) = self.laid_para(loc)?;
        let mut x = 0f32;
        let mut y = 0f32;
        let mut m = DWRITE_HIT_TEST_METRICS::default();
        unsafe { lp.layout.HitTestTextPosition(loc.d, false, &mut x, &mut y, &mut m).ok()? };
        let h = if m.height > 0.0 { m.height } else { lp.h.max(10.0) };
        Some((g.x + lp.x + x, g.y + lp.y + m.top, h))
    }

    /// The location nearest to doc point (x, y).
    pub fn hit(&mut self, x: f32, y: f32, gfx: &Gfx, theme: &Theme) -> Loc {
        if self.geo.is_empty() {
            return Loc::default();
        }
        let mut i = self.block_at_y(y);
        // In the gap above a block, pick whichever neighbour is nearer.
        if i > 0 && y < self.geo[i].y {
            let prev_bottom = self.geo[i - 1].y + self.geo[i - 1].h;
            if (y - prev_bottom).abs() < (self.geo[i].y - y).abs() {
                i -= 1;
            }
        }
        // Skip blocks with nothing to put a caret in (rules) toward the nearest that has text.
        self.ensure_laid(i, gfx, theme);
        self.hit_in_block(i, x, y)
    }

    pub fn hit_in_block(&self, i: usize, x: f32, y: f32) -> Loc {
        let g = &self.geo[i];
        let Some(l) = &g.laid else { return Loc { block: i, para: 0, d: 0 } };
        // choose para: containing point, else nearest by distance
        let mut best = 0usize;
        let mut best_d = f32::MAX;
        for (k, p) in l.paras.iter().enumerate() {
            let (l0, t0, r0, b0) = (g.x + p.x, g.y + p.y, g.x + p.x + p.w.max(4.0), g.y + p.y + p.h);
            let dx = if x < l0 {
                l0 - x
            } else if x > r0 {
                x - r0
            } else {
                0.0
            };
            let dy = if y < t0 {
                t0 - y
            } else if y > b0 {
                y - b0
            } else {
                0.0
            };
            let d = dx * 0.3 + dy;
            if d < best_d {
                best_d = d;
                best = k;
            }
        }
        let p = &l.paras[best];
        let mut trailing = BOOL(0);
        let mut inside = BOOL(0);
        let mut m = DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            let _ = p.layout.HitTestPoint(x - g.x - p.x, y - g.y - p.y, &mut trailing, &mut inside, &mut m);
        }
        let mut d = m.textPosition + if trailing.as_bool() { m.length } else { 0 };
        let len = self.blocks[i].para_at(best).len();
        d = d.min(len);
        Loc { block: i, para: best, d }
    }

    pub fn loc_to_src(&self, loc: Loc, left: bool) -> usize {
        let Some(b) = self.blocks.get(loc.block) else { return 0 };
        b.para_at(loc.para).d2s(loc.d, left)
    }

    /// The location showing source offset `s` (snapping hidden markup to the nearest visible spot).
    pub fn src_to_loc(&self, s: usize) -> Loc {
        if self.blocks.is_empty() {
            return Loc::default();
        }
        let mut i = self.blocks.partition_point(|b| b.src.end < s);
        if i >= self.blocks.len() {
            i = self.blocks.len() - 1;
        }
        // If s lies in the gap before block i, prefer the previous block when s is on its line.
        if i > 0 && s < self.blocks[i].src.start {
            let prev = &self.blocks[i - 1];
            if s <= prev.src.end + 1 {
                i -= 1;
            }
        }
        // Prefer a virtual paragraph sitting exactly at s.
        if i + 1 < self.blocks.len() && self.blocks[i + 1].virtual_ && self.blocks[i + 1].src.start == s {
            i += 1;
        }
        let b = &self.blocks[i];
        let n = b.para_count();
        let mut para = 0;
        if n > 1 {
            for k in 0..n {
                if let Some((ps, _)) = b.para_at(k).src_span() {
                    if ps <= s {
                        para = k;
                    }
                }
            }
        }
        let p = b.para_at(para);
        Loc { block: i, para, d: p.s2d(s) }
    }

    /// Top and bottom (doc y) of the visual line holding `loc`.
    pub fn line_box(&self, loc: Loc) -> Option<(f32, f32)> {
        let (g, lp) = self.laid_para(loc)?;
        let mut count = 0u32;
        unsafe {
            let _ = lp.layout.GetLineMetrics(None, &mut count);
        }
        let mut lines = vec![DWRITE_LINE_METRICS::default(); count.max(1) as usize];
        unsafe {
            let _ = lp.layout.GetLineMetrics(Some(&mut lines), &mut count);
        }
        let mut start = 0u32;
        let mut y = g.y + lp.y;
        for (k, lm) in lines.iter().take(count as usize).enumerate() {
            let end = start + lm.length;
            if loc.d < end || k + 1 == count as usize {
                return Some((y, y + lm.height));
            }
            start = end;
            y += lm.height;
        }
        Some((g.y + lp.y, g.y + lp.y + lp.h))
    }

    /// First and last display positions on the visual line holding `loc`.
    pub fn line_bounds(&self, loc: Loc) -> (u32, u32) {
        let Some((_, lp)) = self.laid_para(loc) else { return (0, 0) };
        let mut count = 0u32;
        unsafe {
            let _ = lp.layout.GetLineMetrics(None, &mut count);
        }
        let mut lines = vec![DWRITE_LINE_METRICS::default(); count as usize];
        unsafe {
            let _ = lp.layout.GetLineMetrics(Some(&mut lines), &mut count);
        }
        let mut start = 0u32;
        for lm in &lines {
            let end = start + lm.length;
            let vis_end = end - lm.newlineLength;
            if loc.d < end || (loc.d == end && lm.newlineLength == 0 && std::ptr::eq(lm, lines.last().unwrap())) {
                // trailing spaces belong to the line; End goes before the trailing whitespace if wrapping
                let content_end =
                    if std::ptr::eq(lm, lines.last().unwrap()) { vis_end } else { (end - lm.trailingWhitespaceLength).max(start) };
                return (start, content_end);
            }
            start = end;
        }
        (start, start)
    }
}

// ------------------------------------------------------------------ helpers
/// Recompute inline background boxes (code, kbd, mark) after a layout's width changed.
fn relayout_bgs(lp: &LaidPara, p: &Para) -> Vec<(D2D_RECT_F, El)> {
    if lp.bgs.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for sp in &p.spans {
        let el = match sp.kind {
            Inline::Code => El::Code,
            Inline::Kbd => El::Kbd,
            Inline::Mark => El::Mark,
            _ => continue,
        };
        if let Some((orig, _)) = lp.bgs.iter().find(|(_, e)| *e == el) {
            let box_h = orig.bottom - orig.top;
            for r in hit_rects(&lp.layout, sp.start, sp.end - sp.start) {
                let pad = ((r.bottom - r.top) - box_h).max(0.0) / 2.0;
                out.push((rect(r.left, r.top + pad, r.right, r.bottom - pad), el));
            }
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
pub enum Wrap {
    Word,
    Char,
    None,
}

pub fn heading_el(n: u8) -> El {
    match n {
        1 => El::H1,
        2 => El::H2,
        3 => El::H3,
        4 => El::H4,
        5 => El::H5,
        _ => El::H6,
    }
}

pub fn alert_el(a: Alert) -> El {
    match a {
        Alert::Note => El::Note,
        Alert::Tip => El::Tip,
        Alert::Important => El::Important,
        Alert::Warning => El::Warning,
        Alert::Caution => El::Caution,
    }
}

fn inline_el(k: Inline) -> Option<El> {
    use crate::highlight::Tok;
    Some(match k {
        Inline::Strong => El::Strong,
        Inline::Em => El::Em,
        Inline::Strike => El::Del,
        Inline::Code => El::Code,
        Inline::Link => El::A,
        Inline::WikiLink => El::WikiLink,
        Inline::Image => El::Image,
        Inline::Math => El::Math,
        Inline::Kbd => El::Kbd,
        Inline::Mark => El::Mark,
        Inline::Sup | Inline::Sub => El::Sup,
        Inline::FootRef => El::Sup,
        Inline::TaskDone => El::TaskDone,
        Inline::Underline => return None,
        Inline::Tok(t) => match t {
            Tok::Keyword => El::TokKeyword,
            Tok::String => El::TokString,
            Tok::Comment => El::TokComment,
            Tok::Number => El::TokNumber,
            Tok::Type => El::TokType,
            Tok::Function => El::TokFunction,
            Tok::Constant => El::TokConstant,
            Tok::Property => El::TokProperty,
            Tok::Tag => El::TokTag,
            Tok::Meta => El::TokMeta,
            Tok::Inserted => El::TokInserted,
            Tok::Deleted => El::TokDeleted,
            Tok::Variable => El::TokVariable,
        },
    })
}

fn layout_key(b: &Block, w: f32, mode: Mode, wrap: bool) -> u64 {
    let mut h = DefaultHasher::new();
    b.kind.hash(&mut h);
    b.para.text.hash(&mut h);
    b.para.spans.hash(&mut h);
    for r in &b.rows {
        for c in r {
            c.text.hash(&mut h);
            c.spans.hash(&mut h);
        }
    }
    for c in &b.ctx {
        c.kind.hash(&mut h);
        c.first.hash(&mut h);
    }
    b.tight.hash(&mut h);
    ((w * 4.0) as i64).hash(&mut h);
    (mode == Mode::Mono).hash(&mut h);
    wrap.hash(&mut h);
    h.finish()
}

fn baseline_for(gfx: &Gfx, family: &str, st: &Style) -> f32 {
    let (asc, desc) = gfx.ascent_descent(family, st.size, st.weight, st.italic);
    let lh = st.line_height.max(asc + desc);
    (lh - (asc + desc)) / 2.0 + asc
}

fn marker_layout(gfx: &Gfx, text: &str, family: &str, st: &Style, _first: &LaidPara, max_w: f32) -> IDWriteTextLayout {
    let l = gfx.simple_layout(text, family, st.size, 400, false, max_w);
    unsafe {
        let _ = l.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, st.line_height.max(st.size), baseline_for(gfx, family, st));
    }
    l
}

fn first_line_height(l: &IDWriteTextLayout) -> Option<f32> {
    let mut count = 0u32;
    let mut lines = [DWRITE_LINE_METRICS::default(); 1];
    unsafe { l.GetLineMetrics(Some(&mut lines), &mut count).ok() };
    if count == 0 {
        unsafe {
            let _ = l.GetLineMetrics(None, &mut count);
        }
        let mut v = vec![DWRITE_LINE_METRICS::default(); count as usize];
        unsafe { l.GetLineMetrics(Some(&mut v), &mut count).ok()? };
        return v.first().map(|m| m.height);
    }
    Some(lines[0].height)
}

pub fn hit_rects(l: &IDWriteTextLayout, start: u32, len: u32) -> Vec<D2D_RECT_F> {
    let mut count = 0u32;
    unsafe {
        let _ = l.HitTestTextRange(start, len, 0.0, 0.0, None, &mut count);
    }
    if count == 0 {
        return Vec::new();
    }
    let mut v = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
    unsafe {
        if l.HitTestTextRange(start, len, 0.0, 0.0, Some(&mut v), &mut count).is_err() {
            return Vec::new();
        }
    }
    v.truncate(count as usize);
    v.iter().map(|m| rect(m.left, m.top, m.left + m.width, m.top + m.height)).collect()
}

fn inset(r: D2D_RECT_F, d: f32) -> D2D_RECT_F {
    rect(r.left + d, r.top + d, r.right - d, r.bottom - d)
}

fn push_box(out: &mut Vec<Deco>, st: &Style, x: f32, y: f32, w: f32, h: f32) {
    let r = rect(x, y, x + w, y + h);
    if let Some(bg) = st.background {
        out.push(Deco::Rect { r, fill: Some(bg), radius: st.radius });
    }
    let all = [st.border_top, st.border_right, st.border_bottom, st.border_left];
    if all.iter().all(|b| b.is_some())
        && all.iter().all(|b| b.map(|b| (b.width, b.color.key())) == all[0].map(|b| (b.width, b.color.key())))
    {
        let b = all[0].unwrap();
        out.push(Deco::Frame { r: inset(r, b.width / 2.0), width: b.width, color: b.color, radius: st.radius });
    } else {
        if let Some(b) = st.border_left {
            out.push(Deco::Rect { r: rect(x, y, x + b.width, y + h), fill: Some(b.color), radius: 0.0 });
        }
        if let Some(b) = st.border_top {
            out.push(Deco::Rect { r: rect(x, y, x + w, y + b.width), fill: Some(b.color), radius: 0.0 });
        }
        if let Some(b) = st.border_bottom {
            out.push(Deco::Rect { r: rect(x, y + h - b.width, x + w, y + h), fill: Some(b.color), radius: 0.0 });
        }
        if let Some(b) = st.border_right {
            out.push(Deco::Rect { r: rect(x + w - b.width, y, x + w, y + h), fill: Some(b.color), radius: 0.0 });
        }
    }
}

fn paint_deco(gfx: &Gfx, rt: &ID2D1RenderTarget, d: &Deco, ox: f32, oy: f32) {
    unsafe {
        match d {
            Deco::Rect { r, fill, radius } => {
                if let Some(c) = fill {
                    let rr = rect(r.left + ox, r.top + oy, r.right + ox, r.bottom + oy);
                    if *radius > 0.0 {
                        rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: rr, radiusX: *radius, radiusY: *radius }, &gfx.brush(c));
                    } else {
                        rt.FillRectangle(&rr, &gfx.brush(c));
                    }
                }
            }
            Deco::Frame { r, width, color, radius } => {
                let rr = rect(r.left + ox, r.top + oy, r.right + ox, r.bottom + oy);
                rt.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT { rect: rr, radiusX: *radius, radiusY: *radius },
                    &gfx.brush(color),
                    *width,
                    None,
                );
            }
            Deco::Line { x0, y0, x1, y1, width, color } => {
                rt.DrawLine(pt(x0 + ox, y0 + oy), pt(x1 + ox, y1 + oy), &gfx.brush(color), *width, None);
            }
            Deco::Text { layout, x, y, color } => {
                rt.DrawTextLayout(pt(x + ox, y + oy), layout, &gfx.brush(color), D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
            }
            Deco::Check { x, y, size, checked, color, fill } => {
                let r = rect(x + ox, y + oy, x + ox + size, y + oy + size);
                let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: size * 0.22, radiusY: size * 0.22 };
                if *checked {
                    rt.FillRoundedRectangle(&rr, &gfx.brush(fill));
                    let s = *size;
                    let (bx, by) = (x + ox, y + oy);
                    let ink = if fill.luminance() > 0.5 { Color::rgb(20, 22, 26) } else { Color::rgb(255, 255, 255) };
                    let br = gfx.brush(&ink);
                    rt.DrawLine(
                        pt(bx + s * 0.24, by + s * 0.52),
                        pt(bx + s * 0.43, by + s * 0.71),
                        &br,
                        s * 0.12,
                        gfx.stroke_round.as_ref(),
                    );
                    rt.DrawLine(
                        pt(bx + s * 0.43, by + s * 0.71),
                        pt(bx + s * 0.78, by + s * 0.30),
                        &br,
                        s * 0.12,
                        gfx.stroke_round.as_ref(),
                    );
                } else {
                    let rr = D2D1_ROUNDED_RECT { rect: inset(r, 0.75), radiusX: size * 0.22, radiusY: size * 0.22 };
                    rt.DrawRoundedRectangle(&rr, &gfx.brush(color), 1.5, None);
                }
            }
        }
    }
}

/// Lay out one paragraph's display text with its inline styles.
#[allow(clippy::too_many_arguments)]
pub fn lay_para(
    gfx: &Gfx,
    theme: &Theme,
    p: &Para,
    st: &Style,
    family: &str,
    color: Color,
    width: f32,
    wrap: Wrap,
    align: Align,
    links: &[String],
) -> LaidPara {
    let _ = links;
    let fmt = gfx.format(family, st.size, st.weight, st.italic);
    let layout = unsafe { gfx.dw.CreateTextLayout(&p.text, &fmt, width.max(1.0), 100000.0).expect("text layout") };
    unsafe {
        let lh = st.line_height.max(st.size * 1.05);
        let _ = layout.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, lh, baseline_for(gfx, family, st));
        let _ = layout.SetWordWrapping(match wrap {
            Wrap::Word => DWRITE_WORD_WRAPPING_WRAP,
            Wrap::Char => DWRITE_WORD_WRAPPING_EMERGENCY_BREAK,
            Wrap::None => DWRITE_WORD_WRAPPING_NO_WRAP,
        });
        let _ = layout.SetTextAlignment(match align {
            Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
            Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            _ => DWRITE_TEXT_ALIGNMENT_LEADING,
        });
        let full = range(0, p.len());
        let _ = layout.SetDrawingEffect(&gfx.brush(&color), full);
        if st.underline {
            let _ = layout.SetUnderline(true, full);
        }
        if st.strike {
            let _ = layout.SetStrikethrough(true, full);
        }
    }
    // Outer spans first so inner ones win.
    let mut spans: Vec<&Span> = p.spans.iter().collect();
    spans.sort_by_key(|s| (std::cmp::Reverse(s.end - s.start), s.start));
    let mut bg_ranges: Vec<(u32, u32, El)> = Vec::new();
    for sp in spans {
        if sp.end <= sp.start || sp.end > p.len() {
            continue;
        }
        let r = range(sp.start, sp.end - sp.start);
        if sp.kind == Inline::Underline {
            unsafe {
                let _ = layout.SetUnderline(true, r);
            }
            continue;
        }
        let Some(el) = inline_el(sp.kind) else { continue };
        let pr = theme.props(el);
        unsafe {
            if let Some(fams) = &pr.font_family {
                let f = gfx.pick_family(fams);
                let w = wstr(&f);
                let _ = layout.SetFontFamilyName(PCWSTR(w.as_ptr()), r);
            }
            if let Some(sz) = pr.font_size {
                let _ = layout.SetFontSize(sz.px(st.size).max(4.0), r);
            }
            if let Some(wt) = pr.font_weight {
                let _ = layout.SetFontWeight(DWRITE_FONT_WEIGHT(wt as i32), r);
            }
            if let Some(it) = pr.italic {
                let _ = layout.SetFontStyle(if it { DWRITE_FONT_STYLE_ITALIC } else { DWRITE_FONT_STYLE_NORMAL }, r);
            }
            if let Some(c) = pr.color {
                let _ = layout.SetDrawingEffect(&gfx.brush(&c), r);
            }
            if let Some(u) = pr.underline {
                let _ = layout.SetUnderline(u, r);
            }
            if let Some(s) = pr.strike {
                let _ = layout.SetStrikethrough(s, r);
            }
        }
        if pr.background.is_some() && matches!(sp.kind, Inline::Code | Inline::Kbd | Inline::Mark) {
            bg_ranges.push((sp.start, sp.end, el));
        }
    }
    let m = layout_metrics(&layout);
    let mut bgs = Vec::new();
    for (s, e, el) in bg_ranges {
        for r in hit_rects(&layout, s, e - s) {
            // Pull the box in vertically to the text, not the full line box.
            let lh = r.bottom - r.top;
            let pad = (lh - st.size * 1.45).max(0.0) / 2.0;
            bgs.push((rect(r.left, r.top + pad, r.right, r.bottom - pad), el));
        }
    }
    let h = if p.len() == 0 { st.line_height.max(st.size * 1.05) } else { m.height };
    LaidPara { layout, x: 0.0, y: 0.0, w: if wrap == Wrap::None { m.widthIncludingTrailingWhitespace } else { width }, h, bgs }
}
