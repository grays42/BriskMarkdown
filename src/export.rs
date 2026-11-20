//! Off-screen tools used for screenshots and testing:
//!   --png <in.md> <out.png> [--width 900] [--theme Dark] [--mono] [--max-height 16000] [--scale 1]
//!   --check <dir> <report.txt> [--layout]
//! The check walks a folder of Markdown files and verifies the source/display mapping of every
//! block (and, with --layout, lays every block out and times it).

use crate::gfx::*;
use crate::markdown::{self, Block};
use crate::theme::{El, Theme};
use crate::view::{Mode, View};
use std::io::Write;
use std::path::{Path, PathBuf};
use windows::core::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;

fn arg<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(|s| s.as_str())
}

pub fn cli(args: &[String]) -> i32 {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    if let Some(i) = args.iter().position(|a| a == "--png") {
        let (Some(input), Some(out)) = (args.get(i + 1), args.get(i + 2)) else { return 2 };
        let width: f32 = arg(args, "--width").and_then(|s| s.parse().ok()).unwrap_or(900.0);
        let max_h: f32 = arg(args, "--max-height").and_then(|s| s.parse().ok()).unwrap_or(16000.0);
        let scale: f32 = arg(args, "--scale").and_then(|s| s.parse().ok()).unwrap_or(1.0);
        let theme = crate::app::load_theme(arg(args, "--theme").unwrap_or("Dark"));
        let mode = if args.iter().any(|a| a == "--mono") { Mode::Mono } else { Mode::Markdown };
        let text = match std::fs::read(input) {
            Ok(b) => crate::editor::decode(&b).0.replace("\r\n", "\n"),
            Err(_) => return 3,
        };
        return match render_png(&text, Path::new(out), width, &theme, mode, scale, max_h) {
            Ok(_) => 0,
            Err(e) => {
                eprintln!("{e:?}");
                4
            }
        };
    }
    if let Some(i) = args.iter().position(|a| a == "--check") {
        let (Some(dir), Some(out)) = (args.get(i + 1), args.get(i + 2)) else { return 2 };
        let layout = args.iter().any(|a| a == "--layout");
        return check(Path::new(dir), Path::new(out), layout);
    }
    2
}

pub fn wic() -> Result<IWICImagingFactory> {
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
}

pub fn wic_target(gfx: &Gfx, f: &IWICImagingFactory, w: u32, h: u32, scale: f32) -> Result<(IWICBitmap, ID2D1RenderTarget)> {
    unsafe {
        let bmp = f.CreateBitmap(w.max(1), h.max(1), &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0 * scale,
            dpiY: 96.0 * scale,
            ..Default::default()
        };
        let rt = gfx.d2d.CreateWicBitmapRenderTarget(&bmp, &props)?;
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        Ok((bmp, rt))
    }
}

pub fn render_png(text: &str, out: &Path, width: f32, theme: &Theme, mode: Mode, scale: f32, max_h: f32) -> Result<f32> {
    let mut gfx = Gfx::new()?;
    let f = wic()?;
    let build = |m: Mode| -> Vec<Block> {
        match m {
            Mode::Markdown => markdown::build(text, None),
            Mode::Mono => markdown::build_mono(text),
        }
    };
    // Pass 1: measure.
    let (_b, rt) = wic_target(&gfx, &f, 8, 8, scale)?;
    gfx.set_target(Some(rt));
    let mut view = View::new();
    view.mode = mode;
    view.screen_w = width;
    view.screen_h = 1000.0;
    view.set_blocks(build(mode), theme);
    view.lay_all(&gfx, theme);
    let h = view.doc_h.min(max_h).ceil();
    // Pass 2: draw at full size (brushes belong to the new target, so lay out again).
    let (bmp, rt) = wic_target(&gfx, &f, (width * scale).ceil() as u32, (h * scale).ceil() as u32, scale)?;
    gfx.set_target(Some(rt.clone()));
    view.screen_h = h;
    view.invalidate(theme);
    view.lay_all(&gfx, theme);
    let bg = theme.style(if mode == Mode::Mono { El::Mono } else { El::Body }).background.unwrap_or(crate::theme::Color::rgb(20, 22, 26));
    unsafe {
        rt.BeginDraw();
        rt.Clear(Some(&d2color(&bg)));
        view.paint(&gfx, theme, &rt, 0.0, (0, 0), None, &[], None);
        rt.EndDraw(None, None)?;
        save_png(&f, &bmp, out)?;
    }
    Ok(view.doc_h)
}

pub fn save_png(f: &IWICImagingFactory, bmp: &IWICBitmap, out: &Path) -> Result<()> {
    unsafe {
        let stream = f.CreateStream()?;
        let w = wstr(&out.to_string_lossy());
        stream.InitializeFromFilename(PCWSTR(w.as_ptr()), 0x40000000)?; // GENERIC_WRITE
        let enc = f.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        enc.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame: Option<IWICBitmapFrameEncode> = None;
        enc.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.unwrap();
        frame.Initialize(None)?;
        let mut fmt = GUID_WICPixelFormat32bppPBGRA;
        let (mut bw, mut bh) = (0u32, 0u32);
        bmp.GetSize(&mut bw, &mut bh)?;
        frame.SetSize(bw, bh)?;
        frame.SetPixelFormat(&mut fmt)?;
        frame.WriteSource(bmp, std::ptr::null())?;
        frame.Commit()?;
        enc.Commit()?;
    }
    Ok(())
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.eq_ignore_ascii_case("Private") || name == ".git" || name == "node_modules" || name == "target" {
            continue;
        }
        if p.is_dir() {
            collect(&p, out);
        } else if crate::platform::EXTENSIONS.iter().any(|x| name.to_ascii_lowercase().ends_with(x)) {
            out.push(p);
        }
    }
}

fn verify(src: &str, blocks: &[Block]) -> std::result::Result<(), String> {
    for b in blocks {
        if b.src.end > src.len() || b.src.start > b.src.end {
            return Err(format!("block range {:?} out of bounds", b.src));
        }
        for p in b.paras() {
            let mut last = 0u32;
            for seg in &p.segs {
                if seg.d < last {
                    return Err(format!("segments out of order in {:?}", b.kind));
                }
                last = seg.d + seg.dlen;
                let (s, e) = (seg.s as usize, (seg.s + seg.slen) as usize);
                if e > src.len() || !src.is_char_boundary(s) || !src.is_char_boundary(e) {
                    return Err(format!("segment {s}..{e} not on a character boundary"));
                }
                if seg.exact {
                    let disp = String::from_utf16_lossy(&p.text[seg.d as usize..(seg.d + seg.dlen) as usize]);
                    if src[s..e] != disp {
                        return Err(format!(
                            "exact segment differs at {s}: {:?} vs {:?}",
                            &src[s..e.min(s + 40)],
                            disp.chars().take(40).collect::<String>()
                        ));
                    }
                }
            }
            if last as usize > p.text.len() {
                return Err("segment past display end".into());
            }
            // Every position on normal lines; a sample on very long ones (d2s is linear in d).
            let step = if p.len() > 4000 { (p.len() / 2000).max(1) } else { 1 } as usize;
            for d in (0..=p.len()).step_by(step) {
                let s = p.d2s(d, true);
                if s > src.len() || !src.is_char_boundary(s) {
                    return Err(format!("d2s({d}) = {s} invalid"));
                }
                let s2 = p.d2s(d, false);
                if s2 > src.len() || !src.is_char_boundary(s2) {
                    return Err(format!("d2s({d}, right) = {s2} invalid"));
                }
            }
            for sp in &p.spans {
                if sp.end as usize > p.text.len() || sp.start > sp.end {
                    return Err(format!("span {:?} out of range", sp));
                }
            }
        }
    }
    Ok(())
}

fn check(dir: &Path, out: &Path, layout: bool) -> i32 {
    let mut files = Vec::new();
    collect(dir, &mut files);
    files.sort();
    let mut report = std::fs::File::create(out).expect("report");
    let progress_path = out.with_extension("progress");
    let mut failures = 0;
    let mut timings: Vec<(f64, f64, usize, PathBuf)> = Vec::new();
    let theme = Theme::builtin("Dark");
    let gfx_rt = if layout {
        let mut g = Gfx::new().unwrap();
        let f = wic().unwrap();
        let (_b, rt) = wic_target(&g, &f, 8, 8, 1.0).unwrap();
        g.set_target(Some(rt));
        Some(g)
    } else {
        None
    };
    let t_all = std::time::Instant::now();
    let mut total_bytes = 0usize;
    for (n, p) in files.iter().enumerate() {
        let _ = std::fs::write(&progress_path, format!("{n} {}\n", p.display()));
        let Ok(bytes) = std::fs::read(p) else { continue };
        let text = crate::editor::decode(&bytes).0.replace("\r\n", "\n").replace('\r', "\n");
        total_bytes += text.len();
        let t0 = std::time::Instant::now();
        let blocks = markdown::build(&text, None);
        let parse_ms = t0.elapsed().as_secs_f64() * 1000.0;
        if let Err(e) = verify(&text, &blocks) {
            failures += 1;
            let _ = writeln!(report, "FAIL markdown {}: {e}", p.display());
        }
        // also with a caret on every 997th byte, exercising the blank-line placeholder
        let mut k = 0;
        while k < text.len() {
            let c = crate::editor::clamp_boundary(&text, k);
            let b = markdown::build(&text, Some(c));
            if let Err(e) = verify(&text, &b) {
                failures += 1;
                let _ = writeln!(report, "FAIL caret {c} {}: {e}", p.display());
                break;
            }
            k += 7919;
        }
        let mono = markdown::build_mono(&text);
        if let Err(e) = verify(&text, &mono) {
            failures += 1;
            let _ = writeln!(report, "FAIL mono {}: {e}", p.display());
        }
        let mut lay_ms = 0.0;
        if let Some(g) = &gfx_rt {
            let t1 = std::time::Instant::now();
            let mut v = View::new();
            v.screen_w = 1000.0;
            v.screen_h = 800.0;
            v.set_blocks(blocks, &theme);
            v.lay_all(g, &theme);
            lay_ms = t1.elapsed().as_secs_f64() * 1000.0;
        }
        timings.push((parse_ms, lay_ms, text.len(), p.clone()));
    }
    let total = t_all.elapsed().as_secs_f64();
    timings.sort_by(|a, b| (b.0 + b.1).partial_cmp(&(a.0 + a.1)).unwrap());
    let _ = writeln!(report, "\nfiles={} failures={} bytes={} seconds={:.1}", files.len(), failures, total_bytes, total);
    let _ = writeln!(report, "slowest (parse ms, layout ms, bytes, file):");
    for t in timings.iter().take(15) {
        let _ = writeln!(report, "  {:7.2} {:8.2} {:8} {}", t.0, t.1, t.2, t.3.display());
    }
    let mut p: Vec<f64> = timings.iter().map(|t| t.0 + t.1).collect();
    p.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if !p.is_empty() {
        let q = |f: f64| p[((p.len() as f64 - 1.0) * f) as usize];
        let _ = writeln!(report, "parse+layout ms: p50={:.2} p90={:.2} p99={:.2} max={:.2}", q(0.5), q(0.9), q(0.99), q(1.0));
    }
    let _ = std::fs::remove_file(&progress_path);
    if failures > 0 {
        1
    } else {
        0
    }
}
