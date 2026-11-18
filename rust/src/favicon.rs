//! Site icons, as the page engine hands them over: the PNG kept per host, in
//! memory only, and turned into a small bitmap the first time it is drawn at a
//! size. A tab that sleeps through a launch shows its letter until it is
//! visited; nothing is read or decoded for it before then.

use std::cell::RefCell;

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};

/// Enough for every tab most people keep open; the oldest goes first.
const KEPT: usize = 64;

/// A decoded icon: premultiplied 32-bit, top-down, `side` pixels square.
struct Bitmap {
    side: i32,
    handle: HBITMAP,
    /// How dark and how light its drawn pixels go (a tenth in from each end),
    /// 0 to 255: a white mark on a white row would vanish.
    tones: (u8, u8),
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.handle.into());
        }
    }
}

struct Icon {
    host: String,
    png: Vec<u8>,
    /// One per size asked for (a row's, a pin's); a DPI change asks for new ones.
    sized: Vec<Bitmap>,
}

thread_local! {
    /// Most recently given or drawn last.
    static ICONS: RefCell<Vec<Icon>> = const { RefCell::new(Vec::new()) };
    static FACTORY: RefCell<Option<IWICImagingFactory>> = const { RefCell::new(None) };
}

/// The icon a page gave for `host`. Any bitmap made from an older one goes with it.
pub fn store(host: &str, png: Vec<u8>) {
    if host.is_empty() || png.is_empty() {
        return;
    }
    ICONS.with(|icons| {
        let mut icons = icons.borrow_mut();
        if let Some(at) = icons.iter().position(|i| i.host == host) {
            if icons[at].png == png {
                return;
            }
            icons.remove(at);
        }
        if icons.len() >= KEPT {
            icons.remove(0);
        }
        icons.push(Icon { host: host.to_string(), png, sized: Vec::new() });
    });
}

pub fn has(host: &str) -> bool {
    ICONS.with(|icons| icons.borrow().iter().any(|i| i.host == host))
}

/// Draws `host`'s icon into `spot` (square), over `behind`. False if there is
/// none to draw, so the caller draws the letter instead. An icon too close in
/// tone to what it sits on (a white mark on a white row) gets a small tile of
/// the other tone under it.
pub fn draw(hdc: HDC, host: &str, spot: RECT, behind: (u8, u8, u8)) -> bool {
    let side = spot.right - spot.left;
    let png = ICONS.with(|icons| {
        let icons = icons.borrow();
        let icon = icons.iter().find(|i| i.host == host)?;
        if icon.sized.iter().any(|b| b.side == side) { Some(None) } else { Some(Some(icon.png.clone())) }
    });
    let Some(png) = png else { return false };
    // Decoded outside the borrow: WIC is a COM call like any other.
    if let Some(png) = png {
        let Some(bitmap) = decode(&png, side) else { return false };
        ICONS.with(|icons| {
            if let Some(icon) = icons.borrow_mut().iter_mut().find(|i| i.host == host) {
                icon.sized.push(bitmap);
            }
        });
    }
    let handle = ICONS.with(|icons| {
        icons.borrow().iter().find(|i| i.host == host).and_then(|i| i.sized.iter().find(|b| b.side == side)).map(|b| (b.handle, b.tones))
    });
    let Some((handle, (darkest, lightest))) = handle else { return false };
    let ground = luma(behind.0, behind.1, behind.2);
    unsafe {
        // Seen if some of it is well darker or well lighter than the ground.
        if ground.saturating_sub(darkest) < 64 && lightest.saturating_sub(ground) < 64 {
            let tile = if ground > 128 { (96, 100, 108) } else { (214, 218, 224) };
            let brush = CreateSolidBrush(COLORREF(tile.0 as u32 | (tile.1 as u32) << 8 | (tile.2 as u32) << 16));
            let pen = CreatePen(PS_SOLID, 1, COLORREF(tile.0 as u32 | (tile.1 as u32) << 8 | (tile.2 as u32) << 16));
            let (old_brush, old_pen) = (SelectObject(hdc, brush.into()), SelectObject(hdc, pen.into()));
            let pad = (side / 8).max(1);
            let _ = RoundRect(hdc, spot.left - pad, spot.top - pad, spot.right + pad, spot.bottom + pad, 2 * pad + 2, 2 * pad + 2);
            SelectObject(hdc, old_brush);
            SelectObject(hdc, old_pen);
            let _ = DeleteObject(brush.into());
            let _ = DeleteObject(pen.into());
        }
        let source = CreateCompatibleDC(Some(hdc));
        let old = SelectObject(source, handle.into());
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
        let _ = GdiAlphaBlend(hdc, spot.left, spot.top, side, side, source, 0, 0, side, side, blend);
        SelectObject(source, old);
        let _ = DeleteDC(source);
    }
    true
}

/// A DPI change: every bitmap is the wrong size now. The PNGs stay.
pub fn forget_sizes() {
    ICONS.with(|icons| icons.borrow_mut().iter_mut().for_each(|i| i.sized.clear()));
}

fn factory() -> Option<IWICImagingFactory> {
    if let Some(factory) = FACTORY.with(|f| f.borrow().clone()) {
        return Some(factory);
    }
    let made: IWICImagingFactory = unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }.ok()?;
    FACTORY.with(|f| *f.borrow_mut() = Some(made.clone()));
    Some(made)
}

/// The PNG, scaled to `side` pixels and premultiplied, as GDI's AlphaBlend wants it.
fn decode(png: &[u8], side: i32) -> Option<Bitmap> {
    if side <= 0 {
        return None;
    }
    let factory = factory()?;
    unsafe {
        let stream = factory.CreateStream().ok()?;
        stream.InitializeFromMemory(png).ok()?;
        let decoder = factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
        let frame = decoder.GetFrame(0).ok()?;
        let scaler = factory.CreateBitmapScaler().ok()?;
        scaler.Initialize(&frame, side as u32, side as u32, WICBitmapInterpolationModeHighQualityCubic).ok()?;
        let converter = factory.CreateFormatConverter().ok()?;
        converter
            .Initialize(&scaler, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
            .ok()?;

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: side,
                biHeight: -side,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let handle = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let mut bitmap = Bitmap { side, handle, tones: (0, 255) };
        let stride = side as u32 * 4;
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, (stride * side as u32) as usize);
        converter.CopyPixels(std::ptr::null(), stride, pixels).ok()?;
        bitmap.tones = tones(pixels);
        Some(bitmap)
    }
}

fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000) as u8
}

/// How dark and how light the pixels that show go, a tenth in from each end
/// (premultiplied BGRA); all of the range when none show.
fn tones(pixels: &[u8]) -> (u8, u8) {
    let mut shown: Vec<u8> = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[3] >= 128)
        .map(|px| {
            let straight = |c: u8| (c as u32 * 255 / px[3] as u32).min(255) as u8;
            luma(straight(px[2]), straight(px[1]), straight(px[0]))
        })
        .collect();
    if shown.is_empty() {
        return (0, 255);
    }
    shown.sort_unstable();
    let tenth = shown.len() / 10;
    (shown[tenth], shown[shown.len() - 1 - tenth])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tones_read_only_what_shows() {
        // Two white opaque pixels and a transparent black one: all white.
        assert_eq!(tones(&[255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 0]), (255, 255));
        assert_eq!(tones(&[0, 0, 0, 0]), (0, 255));
        // Half-transparent white, premultiplied, is still white.
        assert_eq!(tones(&[128, 128, 128, 128]), (255, 255));
        // A black mark on a white square has both.
        let mut square = [255u8; 40];
        square[..4].copy_from_slice(&[0, 0, 0, 255]);
        square[4..8].copy_from_slice(&[0, 0, 0, 255]);
        assert_eq!(tones(&square), (0, 255));
    }
}
