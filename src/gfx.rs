//! Direct2D / DirectWrite resources: factories, the render target, cached brushes, text
//! formats and font metrics.

use crate::theme::Color;
use std::cell::RefCell;
use std::collections::HashMap;
use windows::core::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;

#[derive(Clone, PartialEq, Eq, Hash)]
struct FmtKey {
    family: String,
    size: u32,
    weight: u16,
    italic: bool,
}

pub struct Gfx {
    pub d2d: ID2D1Factory,
    pub dw: IDWriteFactory,
    fonts: IDWriteFontCollection,
    pub rt: Option<ID2D1RenderTarget>,
    brushes: RefCell<HashMap<u32, ID2D1SolidColorBrush>>,
    formats: RefCell<HashMap<FmtKey, IDWriteTextFormat>>,
    families: RefCell<HashMap<String, bool>>,
    metrics: RefCell<HashMap<FmtKey, (f32, f32)>>,
    pub stroke_round: Option<ID2D1StrokeStyle>,
}

pub fn wstr(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn d2color(c: &Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.r, g: c.g, b: c.b, a: c.a }
}

pub fn rect(l: f32, t: f32, r: f32, b: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: l, top: t, right: r, bottom: b }
}

pub fn pt(x: f32, y: f32) -> windows_numerics::Vector2 {
    windows_numerics::Vector2 { X: x, Y: y }
}

impl Gfx {
    pub fn new() -> Result<Gfx> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let mut fonts: Option<IDWriteFontCollection> = None;
            dw.GetSystemFontCollection(&mut fonts, false)?;
            let props = D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 10.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
            };
            let stroke_round = d2d.CreateStrokeStyle(&props, None).ok();
            Ok(Gfx {
                d2d,
                dw,
                fonts: fonts.unwrap(),
                rt: None,
                brushes: RefCell::new(HashMap::new()),
                formats: RefCell::new(HashMap::new()),
                families: RefCell::new(HashMap::new()),
                metrics: RefCell::new(HashMap::new()),
                stroke_round,
            })
        }
    }

    /// Forget every resource tied to the render target (brushes); layouts holding brushes must
    /// be rebuilt by the caller.
    pub fn set_target(&mut self, rt: Option<ID2D1RenderTarget>) {
        self.brushes.borrow_mut().clear();
        self.rt = rt;
    }

    pub fn has_family(&self, name: &str) -> bool {
        if let Some(&ok) = self.families.borrow().get(name) {
            return ok;
        }
        let mut idx = 0u32;
        let mut exists = BOOL(0);
        let w = wstr(name);
        unsafe {
            let _ = self.fonts.FindFamilyName(PCWSTR(w.as_ptr()), &mut idx, &mut exists);
        }
        let ok = exists.as_bool();
        self.families.borrow_mut().insert(name.to_string(), ok);
        ok
    }

    /// The first installed family from a CSS-style list.
    pub fn pick_family(&self, list: &[String]) -> String {
        for f in list {
            if self.has_family(f) {
                return f.clone();
            }
        }
        "Segoe UI".to_string()
    }

    pub fn format(&self, family: &str, size: f32, weight: u16, italic: bool) -> IDWriteTextFormat {
        let key = FmtKey { family: family.to_string(), size: (size * 100.0) as u32, weight, italic };
        if let Some(f) = self.formats.borrow().get(&key) {
            return f.clone();
        }
        let w = wstr(family);
        let f = unsafe {
            self.dw
                .CreateTextFormat(
                    PCWSTR(w.as_ptr()),
                    None,
                    DWRITE_FONT_WEIGHT(weight as i32),
                    if italic { DWRITE_FONT_STYLE_ITALIC } else { DWRITE_FONT_STYLE_NORMAL },
                    DWRITE_FONT_STRETCH_NORMAL,
                    size.max(1.0),
                    w!("en-us"),
                )
                .or_else(|_| {
                    self.dw.CreateTextFormat(
                        w!("Segoe UI"),
                        None,
                        DWRITE_FONT_WEIGHT_NORMAL,
                        DWRITE_FONT_STYLE_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        size.max(1.0),
                        w!("en-us"),
                    )
                })
                .expect("text format")
        };
        self.formats.borrow_mut().insert(key, f.clone());
        f
    }

    /// (ascent, descent) in DIPs for a font at `size`.
    pub fn ascent_descent(&self, family: &str, size: f32, weight: u16, italic: bool) -> (f32, f32) {
        let key = FmtKey { family: family.to_string(), size: (size * 100.0) as u32, weight, italic };
        if let Some(m) = self.metrics.borrow().get(&key) {
            return *m;
        }
        let mut res = (size * 0.93, size * 0.24);
        unsafe {
            let mut idx = 0u32;
            let mut exists = BOOL(0);
            let w = wstr(family);
            if self.fonts.FindFamilyName(PCWSTR(w.as_ptr()), &mut idx, &mut exists).is_ok() && exists.as_bool() {
                if let Ok(fam) = self.fonts.GetFontFamily(idx) {
                    let style = if italic { DWRITE_FONT_STYLE_ITALIC } else { DWRITE_FONT_STYLE_NORMAL };
                    if let Ok(font) = fam.GetFirstMatchingFont(DWRITE_FONT_WEIGHT(weight as i32), DWRITE_FONT_STRETCH_NORMAL, style) {
                        let mut m = DWRITE_FONT_METRICS::default();
                        font.GetMetrics(&mut m);
                        if m.designUnitsPerEm > 0 {
                            let u = m.designUnitsPerEm as f32;
                            res = (m.ascent as f32 / u * size, m.descent as f32 / u * size);
                        }
                    }
                }
            }
        }
        self.metrics.borrow_mut().insert(key, res);
        res
    }

    pub fn brush(&self, c: &Color) -> ID2D1SolidColorBrush {
        let key = c.key();
        if let Some(b) = self.brushes.borrow().get(&key) {
            return b.clone();
        }
        let rt = self.rt.as_ref().expect("render target");
        let b = unsafe { rt.CreateSolidColorBrush(&d2color(c), None).expect("brush") };
        self.brushes.borrow_mut().insert(key, b.clone());
        b
    }

    /// A one-off text layout (status bar, markers, labels).
    pub fn simple_layout(&self, text: &str, family: &str, size: f32, weight: u16, italic: bool, max_w: f32) -> IDWriteTextLayout {
        let fmt = self.format(family, size, weight, italic);
        let t: Vec<u16> = text.encode_utf16().collect();
        unsafe { self.dw.CreateTextLayout(&t, &fmt, max_w.max(1.0), 10000.0).expect("layout") }
    }
}

pub fn layout_metrics(l: &IDWriteTextLayout) -> DWRITE_TEXT_METRICS {
    let mut m = DWRITE_TEXT_METRICS::default();
    unsafe {
        let _ = l.GetMetrics(&mut m);
    }
    m
}

pub fn range(start: u32, len: u32) -> DWRITE_TEXT_RANGE {
    DWRITE_TEXT_RANGE { startPosition: start, length: len }
}
