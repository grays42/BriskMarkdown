//! Printing: lay the document out at page width in a light theme, break pages between lines,
//! and draw each page through a Direct2D DC render target onto the printer.

use crate::gfx::*;
use crate::markdown;
use crate::theme::{El, Theme};
use crate::view::{Mode, View};
use std::path::Path;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::Xps::*;

use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub fn print(hwnd: HWND, text: &str, path: Option<&Path>) {
    unsafe {
        let mut pd = PRINTDLGEXW {
            lStructSize: std::mem::size_of::<PRINTDLGEXW>() as u32,
            hwndOwner: hwnd,
            Flags: PD_RETURNDC | PD_NOPAGENUMS | PD_NOSELECTION | PD_NOCURRENTPAGE | PD_USEDEVMODECOPIESANDCOLLATE,
            nCopies: 1,
            nStartPage: 0xFFFF_FFFF,
            ..Default::default()
        };
        if PrintDlgExW(&mut pd).is_err() || pd.dwResultAction != 1 || pd.hDC.is_invalid() {
            free(&pd);
            return;
        }
        let hdc = pd.hDC;
        let name = path.and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Untitled".into());
        if let Err(e) = print_to_dc(hdc, text, &name) {
            let msg = wstr(&format!("Printing failed: {}", e.message()));
            MessageBoxW(Some(hwnd), PCWSTR(msg.as_ptr()), w!("BriskMarkdown"), MB_OK | MB_ICONWARNING);
        }
        let _ = DeleteDC(hdc);
        free(&pd);
    }
}

unsafe fn free(pd: &PRINTDLGEXW) {
    if !pd.hDevMode.is_invalid() {
        let _ = GlobalFree(Some(pd.hDevMode));
    }
    if !pd.hDevNames.is_invalid() {
        let _ = GlobalFree(Some(pd.hDevNames));
    }
}

unsafe fn print_to_dc(hdc: HDC, text: &str, name: &str) -> Result<()> {
    let dpi_x = GetDeviceCaps(Some(hdc), LOGPIXELSX) as f32;
    let dpi_y = GetDeviceCaps(Some(hdc), LOGPIXELSY) as f32;
    let horz = GetDeviceCaps(Some(hdc), HORZRES) as f32;
    let vert = GetDeviceCaps(Some(hdc), VERTRES) as f32;
    let off_x = GetDeviceCaps(Some(hdc), PHYSICALOFFSETX) as f32;
    let off_y = GetDeviceCaps(Some(hdc), PHYSICALOFFSETY) as f32;
    // 0.75 inch margins, measured from the paper edge.
    let margin = 0.75;
    let ml = (margin * dpi_x - off_x).max(0.0);
    let mt = (margin * dpi_y - off_y).max(0.0);
    let area_w_px = (horz - 2.0 * ml).max(dpi_x);
    let footer_px = 0.35 * dpi_y;
    let area_h_px = (vert - 2.0 * mt - footer_px).max(dpi_y);
    let to_dip = 96.0 / dpi_x;
    let page_w = area_w_px * to_dip;
    let page_h = area_h_px * 96.0 / dpi_y;

    // Print on white: use a light theme whatever the screen theme is.
    let theme = Theme::builtin("Light");
    let mut gfx = Gfx::new()?;
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
        dpiX: dpi_x,
        dpiY: dpi_y,
        ..Default::default()
    };
    let rt = gfx.d2d.CreateDCRenderTarget(&props)?;
    rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
    gfx.set_target(Some(rt.cast()?));

    let mut view = View::new();
    view.mode = Mode::Markdown;
    view.screen_w = page_w;
    view.screen_h = page_h;
    view.set_blocks(markdown::build(text, None), &theme);
    view.lay_all(&gfx, &theme);

    // Page breaks: the lowest line top that fits, so no line is cut in half.
    let cands = view.break_candidates();
    let mut pages: Vec<(f32, f32)> = Vec::new();
    let mut y0 = 0.0f32;
    while y0 < view.doc_h - 1.0 && pages.len() < 5000 {
        let limit = y0 + page_h;
        let mut cut = limit;
        if limit < view.doc_h {
            if let Some(&c) = cands.iter().rev().find(|&&c| c <= limit && c > y0 + page_h * 0.4) {
                cut = c;
            }
        } else {
            cut = view.doc_h;
        }
        pages.push((y0, cut));
        y0 = cut;
    }
    if pages.is_empty() {
        pages.push((0.0, page_h));
    }

    let doc_name = wstr(name);
    let di = DOCINFOW { cbSize: std::mem::size_of::<DOCINFOW>() as i32, lpszDocName: PCWSTR(doc_name.as_ptr()), ..Default::default() };
    if StartDocW(hdc, &di) <= 0 {
        return Err(Error::from_thread());
    }
    let footer_style = theme.style(El::Status).clone();
    let ffam = gfx.pick_family(&footer_style.families);
    let total = pages.len();
    for (n, (top, bottom)) in pages.iter().enumerate() {
        if StartPage(hdc) <= 0 {
            break;
        }
        // Draw the page in horizontal bands to keep memory modest at printer resolution.
        let band_px = (2.0 * dpi_y) as i32;
        let page_px_h = ((bottom - top) * dpi_y / 96.0).ceil() as i32 + footer_px as i32 + 4;
        let mut band_top = 0;
        while band_top < page_px_h {
            let band_h = band_px.min(page_px_h - band_top);
            let r =
                RECT { left: ml as i32, top: mt as i32 + band_top, right: (ml + area_w_px) as i32, bottom: mt as i32 + band_top + band_h };
            rt.BindDC(hdc, &r)?;
            rt.BeginDraw();
            rt.Clear(Some(&d2color(&crate::theme::Color::rgb(255, 255, 255))));
            let band_dip = band_top as f32 * 96.0 / dpi_y;
            let clip_h = (bottom - top) - band_dip;
            if clip_h > 0.0 {
                rt.PushAxisAlignedClip(&rect(0.0, 0.0, page_w, clip_h), D2D1_ANTIALIAS_MODE_ALIASED);
                view.scroll = top + band_dip;
                view.screen_h = (band_h as f32 * 96.0 / dpi_y).min(clip_h);
                let rt2: ID2D1RenderTarget = rt.cast()?;
                view.paint(&gfx, &theme, &rt2, 0.0, (0, 0), None, &[], None);
                rt.PopAxisAlignedClip();
            }
            // Footer
            let footer_y = (bottom - top) + 0.12 * 96.0 - band_dip;
            if footer_y < band_h as f32 * 96.0 / dpi_y && footer_y > -20.0 {
                let label = format!("{name}  \u{00B7}  {} of {}", n + 1, total);
                let lay = gfx.simple_layout(&label, &ffam, 8.5, 400, false, page_w);
                let _ = lay.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
                rt.DrawTextLayout(
                    pt(0.0, footer_y),
                    &lay,
                    &gfx.brush(&crate::theme::Color::rgb(110, 118, 129)),
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
            rt.EndDraw(None, None)?;
            band_top += band_h;
        }
        EndPage(hdc);
    }
    EndDoc(hdc);
    Ok(())
}
