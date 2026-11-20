//! Printing: lay the document out at page width in a light theme, break pages between lines,
//! and draw each page through a Direct2D DC render target onto the printer.

use crate::gfx::*;
use crate::markdown;
use crate::theme::{El, Theme};
use crate::view::{Mode, View};
use std::path::Path;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
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
        if let Err(e) = print_to_dc(hdc, text, &name, None) {
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

/// Test hook: print through "Microsoft Print to PDF" into `out` without any dialog.
pub fn print_test(text: &str, out: &Path) -> Result<()> {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        let hdc = CreateDCW(w!("WINSPOOL"), w!("Microsoft Print to PDF"), PCWSTR::null(), None);
        if hdc.is_invalid() {
            return Err(Error::new(Error::from_thread().code(), "CreateDC failed"));
        }
        let r = print_to_dc(hdc, text, "print-test.md", Some(out));
        let _ = DeleteDC(hdc);
        r
    }
}

unsafe fn print_to_dc(hdc: HDC, text: &str, name: &str, output: Option<&Path>) -> Result<()> {
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
    // One off-screen band bitmap for the whole job: drawn with Direct2D, then copied to the
    // printer with StretchDIBits (Direct2D's DC target refuses printer DCs).
    let band_px = (2.0 * dpi_y) as i32;
    let band_w = area_w_px.ceil() as u32;
    let mut gfx = Gfx::new()?;
    let wic = crate::export::wic()?;
    let (bmp, rt) = crate::export::wic_target(&gfx, &wic, band_w, band_px as u32, dpi_x / 96.0)?;
    rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
    gfx.set_target(Some(rt.clone()));

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
    let out_w = output.map(|o| wstr(&o.to_string_lossy()));
    let di = DOCINFOW {
        cbSize: std::mem::size_of::<DOCINFOW>() as i32,
        lpszDocName: PCWSTR(doc_name.as_ptr()),
        lpszOutput: out_w.as_ref().map(|w| PCWSTR(w.as_ptr())).unwrap_or(PCWSTR::null()),
        ..Default::default()
    };
    if StartDocW(hdc, &di) <= 0 {
        return Err(Error::new(Error::from_thread().code(), "StartDoc failed"));
    }
    let footer_style = theme.style(El::Status).clone();
    let ffam = gfx.pick_family(&footer_style.families);
    let total = pages.len();
    for (n, (top, bottom)) in pages.iter().enumerate() {
        if StartPage(hdc) <= 0 {
            break;
        }
        // Draw the page in horizontal bands to keep memory modest at printer resolution.
        let page_px_h = ((bottom - top) * dpi_y / 96.0).ceil() as i32 + footer_px as i32 + 4;
        let mut band_top = 0;
        while band_top < page_px_h {
            let band_h = band_px.min(page_px_h - band_top);
            rt.BeginDraw();
            rt.Clear(Some(&d2color(&crate::theme::Color::rgb(255, 255, 255))));
            let band_dip = band_top as f32 * 96.0 / dpi_y;
            let clip_h = (bottom - top) - band_dip;
            if clip_h > 0.0 {
                rt.PushAxisAlignedClip(&rect(0.0, 0.0, page_w, clip_h), D2D1_ANTIALIAS_MODE_ALIASED);
                view.scroll = top + band_dip;
                view.screen_h = (band_h as f32 * 96.0 / dpi_y).min(clip_h);
                view.paint(&gfx, &theme, &rt, 0.0, (0, 0), None, &[], None);
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
            rt.EndDraw(None, None).map_err(|e| Error::new(e.code(), "EndDraw failed"))?;
            if let Some(dir) = std::env::var_os("BRISK_PRINT_DEBUG") {
                let _ = crate::export::save_png(&wic, &bmp, &std::path::PathBuf::from(dir).join(format!("band-{n}-{band_top}.png")));
            }
            blit(hdc, &bmp, ml as i32, mt as i32 + band_top, band_w, band_h as u32)?;
            band_top += band_h;
        }
        EndPage(hdc);
    }
    EndDoc(hdc);
    Ok(())
}

/// Copy the top `h` rows of the band bitmap to the printer at (x, y).
unsafe fn blit(hdc: HDC, bmp: &windows::Win32::Graphics::Imaging::IWICBitmap, x: i32, y: i32, w: u32, h: u32) -> Result<()> {
    use windows::Win32::Graphics::Imaging::*;
    let lock = bmp.Lock(&WICRect { X: 0, Y: 0, Width: w as i32, Height: h as i32 }, WICBitmapLockRead.0 as u32)?;
    let stride = lock.GetStride()?;
    let mut size = 0u32;
    let mut ptr: *mut u8 = std::ptr::null_mut();
    lock.GetDataPointer(&mut size, &mut ptr)?;
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: (stride / 4) as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let r = StretchDIBits(hdc, x, y, w as i32, h as i32, 0, 0, w as i32, h as i32, Some(ptr as *const _), &bmi, DIB_RGB_COLORS, SRCCOPY);
    drop(lock);
    if r == 0 {
        return Err(Error::new(Error::from_thread().code(), "StretchDIBits failed"));
    }
    Ok(())
}
