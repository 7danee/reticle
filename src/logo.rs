//! The "r." logo, drawn from the embedded Poppins SemiBold font.

use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, MetadataProvider};
use tiny_skia::{FillRule, Paint, Path, PathBuilder, Pixmap, Transform};

use crate::config::Rgba;

pub const FONT: &[u8] = include_bytes!("../assets/fonts/poppins-semibold.ttf");

pub const LETTER: Rgba = Rgba { r: 0xF2, g: 0xF3, b: 0xF5, a: 255 };
pub const ACCENT: Rgba = Rgba { r: 0x3D, g: 0xDC, b: 0x84, a: 255 };
pub const MUTED: Rgba = Rgba { r: 0x6B, g: 0x70, b: 0x79, a: 255 };
pub const BACKGROUND: Rgba = Rgba { r: 0x15, g: 0x16, b: 0x1A, a: 255 };

/// Collects glyph outlines into a tiny-skia path, flipping font y-up to pixmap y-down.
struct Pen {
    pb: PathBuilder,
    x: f32,
}

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.pb.move_to(self.x + x, -y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.pb.line_to(self.x + x, -y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.pb.quad_to(self.x + cx0, -cy0, self.x + x, -y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.pb.cubic_to(self.x + cx0, -cy0, self.x + cx1, -cy1, self.x + x, -y);
    }
    fn close(&mut self) {
        self.pb.close();
    }
}

/// Outlines of "r" and "." at 100 px, baseline at y = 0.
fn glyphs() -> Option<(Path, Path)> {
    let font = FontRef::new(FONT).ok()?;
    let outlines = font.outline_glyphs();
    let charmap = font.charmap();
    let metrics = font.glyph_metrics(Size::new(100.0), LocationRef::default());
    let mut x = 0.0;
    let mut draw = |ch: char| -> Option<Path> {
        let id = charmap.map(ch)?;
        let mut pen = Pen { pb: PathBuilder::new(), x };
        outlines.get(id)?.draw(DrawSettings::unhinted(Size::new(100.0), LocationRef::default()), &mut pen).ok()?;
        x += metrics.advance_width(id).unwrap_or(0.0) - 4.0; // a touch tighter than normal
        pen.pb.finish()
    };
    let r = draw('r')?;
    let dot = draw('.')?;
    Some((r, dot))
}

/// Renders the logo on a square canvas, optionally on a rounded dark tile.
pub fn render(size: u32, dot: Rgba, tile: bool) -> Pixmap {
    let mut pixmap = Pixmap::new(size, size).expect("logo size is non-zero");
    let s = size as f32;
    if tile
        && let Some(path) = rounded_square(s, s * 0.22) {
            fill(&mut pixmap, &path, BACKGROUND, Transform::identity());
        }
    let Some((r, period)) = glyphs() else { return pixmap };
    let bounds = r.bounds();
    let right = period.bounds().right();
    let (w, h) = (right - bounds.left(), bounds.height());
    let room = if tile { 0.62 } else { 0.86 } * s;
    let scale = (room / w).min(room / h);
    let tx = (s - w * scale) / 2.0 - bounds.left() * scale;
    let ty = (s - h * scale) / 2.0 - bounds.top() * scale;
    let t = Transform::from_row(scale, 0.0, 0.0, scale, tx, ty);
    fill(&mut pixmap, &r, LETTER, t);
    fill(&mut pixmap, &period, dot, t);
    pixmap
}

fn fill(pixmap: &mut Pixmap, path: &Path, rgba: Rgba, t: Transform) {
    let mut paint = Paint { anti_alias: true, ..Paint::default() };
    paint.set_color_rgba8(rgba.r, rgba.g, rgba.b, rgba.a);
    pixmap.fill_path(path, &paint, FillRule::Winding, t, None);
}

fn rounded_square(s: f32, r: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(r, 0.0);
    pb.line_to(s - r, 0.0);
    pb.quad_to(s, 0.0, s, r);
    pb.line_to(s, s - r);
    pb.quad_to(s, s, s - r, s);
    pb.line_to(r, s);
    pb.quad_to(0.0, s, 0.0, s - r);
    pb.line_to(0.0, r);
    pb.quad_to(0.0, 0.0, r, 0.0);
    pb.close();
    pb.finish()
}

/// Windows .ico with PNG entries.
#[cfg(test)]
fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&s| render(s, ACCENT, true).encode_png().unwrap()).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&s, png) in sizes.iter().zip(&images) {
        let dim = if s >= 256 { 0 } else { s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0, 1, 0, 32, 0]);
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for png in images {
        out.extend_from_slice(&png);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_has_letter_and_dot() {
        for size in [16, 24, 32, 64] {
            let p = render(size, ACCENT, true);
            let px = |x: u32, y: u32| p.pixel(x, y).unwrap().demultiply();
            let painted = |pred: &dyn Fn(u8, u8, u8) -> bool| {
                (0..size).any(|y| (0..size).any(|x| {
                    let c = px(x, y);
                    pred(c.red(), c.green(), c.blue())
                }))
            };
            assert!(painted(&|r, g, b| r > 200 && g > 200 && b > 200), "white r at {size}");
            assert!(painted(&|r, g, b| g > 180 && r < 120 && b < 170), "green dot at {size}");
        }
    }

    /// Regenerates `assets/icon/reticle.ico`: `cargo test write_icon -- --ignored`.
    #[test]
    #[ignore]
    fn write_icon() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icon");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("reticle.ico"), ico(&[16, 20, 24, 32, 40, 48, 64, 256])).unwrap();
        render(256, ACCENT, true).save_png(dir.join("reticle.png")).unwrap();
    }
}
