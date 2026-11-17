//! Draws a crosshair into a pixmap. Shared by the overlay and the editor preview,
//! so both always look identical.
//!
//! Geometry works on the pixel grid around a center point `c` (a pixel boundary):
//! - even thickness: lines are centered exactly on `c`,
//! - odd thickness: lines cover the pixel right/below `c`.
//!
//! The overlay places `c` on the screen's center boundary (`left + width / 2`), which
//! matches how games draw their own crosshair. Circles and chevrons are anti-aliased
//! and centered on `c` itself.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use resvg::usvg;
use tiny_skia::{
    BlendMode, Color, FilterQuality, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke,
    Transform,
};

use crate::config::{
    self, ChevronLayer, CircleLayer, CrossLayer, Crosshair, DotLayer, ImageLayer, LIMIT_ANGLE, LIMIT_CHEVRON,
    LIMIT_DOT, LIMIT_GAP, LIMIT_LENGTH, LIMIT_OUTLINE, LIMIT_RADIUS, LIMIT_SCALE, LIMIT_THICKNESS, Layer, Outline,
    Rgba,
};

/// Largest canvas edge in pixels.
const MAX_CANVAS: i32 = 2048;
/// Largest accepted source image edge.
const MAX_IMAGE: u32 = 4096;

const FALLBACK_COLOR: Rgba = Rgba { r: 0, g: 255, b: 102, a: 255 };
const FALLBACK_OUTLINE: Rgba = Rgba { r: 0, g: 0, b: 0, a: 255 };

/// Axis-aligned box in pixels, relative to the center point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Box2 {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Box2 {
    fn grow(self, by: i32) -> Self {
        Self { x: self.x - by, y: self.y - by, w: self.w + 2 * by, h: self.h + 2 * by }
    }

    /// Largest distance of any edge from the center.
    fn extent(self) -> i32 {
        self.x.abs().max((self.x + self.w).abs()).max(self.y.abs()).max((self.y + self.h).abs())
    }
}

fn color(s: &str) -> Rgba {
    Rgba::parse(s).unwrap_or(FALLBACK_COLOR)
}

fn outline(o: &Outline) -> Option<(i32, Rgba)> {
    o.enabled.then(|| {
        (
            o.thickness.clamp(LIMIT_OUTLINE.0, LIMIT_OUTLINE.1) as i32,
            Rgba::parse(&o.color).unwrap_or(FALLBACK_OUTLINE),
        )
    })
}

/// Boxes of a cross: right, left, bottom and (unless T-style) top arm.
/// `h` is the length of the horizontal arms, `v` of the vertical ones.
fn cross_boxes(thickness: i32, h: i32, v: i32, gap: i32, t_style: bool) -> Vec<Box2> {
    let t = thickness;
    let odd = t % 2;
    let cross = -(t / 2);
    let g = gap;
    let mut out = Vec::with_capacity(4);
    if h > 0 {
        out.push(Box2 { x: odd + g, y: cross, w: h, h: t });
        out.push(Box2 { x: -g - h, y: cross, w: h, h: t });
    }
    if v > 0 {
        out.push(Box2 { x: cross, y: odd + g, w: t, h: v });
        if !t_style {
            out.push(Box2 { x: cross, y: -g - v, w: t, h: v });
        }
    }
    out
}

fn dot_box(size: i32) -> Box2 {
    Box2 { x: -(size / 2), y: -(size / 2), w: size, h: size }
}

/// A layer with clamped values and decoded images, ready to draw.
enum Prepared {
    Boxes { boxes: Vec<Box2>, color: Rgba, outline: Option<(i32, Rgba)> },
    Circle { radius: f32, thickness: f32, color: Rgba, outline: Option<(i32, Rgba)> },
    Chevron { size: f32, thickness: f32, half_angle: f32, gap: f32, color: Rgba, outline: Option<(i32, Rgba)> },
    Image { pixmap: Pixmap, opacity: f32 },
}

fn prepare(layer: &Layer) -> Option<Prepared> {
    Some(match layer {
        Layer::Cross(l) => prepare_cross(l),
        Layer::Dot(l) => prepare_dot(l),
        Layer::Circle(l) => prepare_circle(l),
        Layer::Chevron(l) => prepare_chevron(l),
        Layer::Image(l) => prepare_image(l)?,
    })
}

fn prepare_cross(l: &CrossLayer) -> Prepared {
    Prepared::Boxes {
        boxes: cross_boxes(
            l.thickness.clamp(LIMIT_THICKNESS.0, LIMIT_THICKNESS.1) as i32,
            l.length.clamp(LIMIT_LENGTH.0, LIMIT_LENGTH.1) as i32,
            l.vertical().clamp(LIMIT_LENGTH.0, LIMIT_LENGTH.1) as i32,
            l.gap.clamp(LIMIT_GAP.0, LIMIT_GAP.1),
            l.t_style,
        ),
        color: color(&l.color),
        outline: outline(&l.outline),
    }
}

fn prepare_dot(l: &DotLayer) -> Prepared {
    Prepared::Boxes {
        boxes: vec![dot_box(l.size.clamp(LIMIT_DOT.0, LIMIT_DOT.1) as i32)],
        color: color(&l.color),
        outline: outline(&l.outline),
    }
}

fn prepare_circle(l: &CircleLayer) -> Prepared {
    Prepared::Circle {
        radius: l.radius.clamp(LIMIT_RADIUS.0, LIMIT_RADIUS.1) as f32,
        thickness: l.thickness.clamp(LIMIT_THICKNESS.0, LIMIT_THICKNESS.1) as f32,
        color: color(&l.color),
        outline: outline(&l.outline),
    }
}

fn prepare_chevron(l: &ChevronLayer) -> Prepared {
    Prepared::Chevron {
        size: l.size.clamp(LIMIT_CHEVRON.0, LIMIT_CHEVRON.1) as f32,
        thickness: l.thickness.clamp(LIMIT_THICKNESS.0, 16) as f32,
        half_angle: (l.angle.clamp(LIMIT_ANGLE.0, LIMIT_ANGLE.1) as f32 / 2.0).to_radians(),
        gap: l.gap.clamp(LIMIT_GAP.0, LIMIT_GAP.1) as f32,
        color: color(&l.color),
        outline: outline(&l.outline),
    }
}

fn prepare_image(l: &ImageLayer) -> Option<Prepared> {
    if l.file.trim().is_empty() {
        return None;
    }
    let scale = if l.scale.is_finite() { l.scale.clamp(LIMIT_SCALE.0, LIMIT_SCALE.1) } else { 1.0 };
    let opacity = if l.opacity.is_finite() { l.opacity.clamp(0.0, 1.0) } else { 1.0 };
    let source = load_source(&config::image_path(&l.file))?;
    let pixmap = source.scaled(scale)?;
    Some(Prepared::Image { pixmap, opacity })
}

impl Prepared {
    fn extent(&self) -> i32 {
        match self {
            Prepared::Boxes { boxes, outline, .. } => {
                let grow = outline.map_or(0, |(w, _)| w);
                boxes.iter().map(|b| b.grow(grow).extent()).max().unwrap_or(0)
            }
            Prepared::Circle { radius, thickness, outline, .. } => {
                (radius + thickness / 2.0 + outline.map_or(0.0, |(w, _)| w as f32)).ceil() as i32 + 1
            }
            Prepared::Chevron { size, thickness, half_angle, gap, outline, .. } => {
                let reach = gap.abs().max((gap + size * half_angle.cos()).abs()).max(size * half_angle.sin());
                // A full thickness of margin covers the miter at the tip.
                (reach + thickness + outline.map_or(0.0, |(w, _)| w as f32)).ceil() as i32 + 1
            }
            Prepared::Image { pixmap, .. } => (pixmap.width().max(pixmap.height()) as i32 + 1) / 2 + 1,
        }
    }

    fn draw(&self, pixmap: &mut Pixmap, c: i32) {
        match self {
            Prepared::Boxes { boxes, color, outline } => draw_boxes(pixmap, c, boxes, *color, *outline),
            Prepared::Circle { radius, thickness, color, outline } => {
                let (cx, cy) = (c as f32, c as f32);
                if let Some(path) = PathBuilder::from_circle(cx, cy, *radius) {
                    if let Some((w, rgba)) = outline {
                        stroke(pixmap, &path, thickness + 2.0 * *w as f32, *rgba, LineJoin::Round);
                    }
                    stroke(pixmap, &path, *thickness, *color, LineJoin::Round);
                }
            }
            Prepared::Chevron { size, thickness, half_angle, gap, color, outline } => {
                let tip = (c as f32, c as f32 + gap);
                let dir = (half_angle.sin(), half_angle.cos());
                let arms = |len: f32, extend: f32| {
                    let mut pb = PathBuilder::new();
                    pb.move_to(tip.0 - dir.0 * (len + extend), tip.1 + dir.1 * (len + extend));
                    pb.line_to(tip.0, tip.1);
                    pb.line_to(tip.0 + dir.0 * (len + extend), tip.1 + dir.1 * (len + extend));
                    pb.finish()
                };
                if let Some((w, rgba)) = outline
                    && let Some(path) = arms(*size, *w as f32) {
                        stroke(pixmap, &path, thickness + 2.0 * *w as f32, *rgba, LineJoin::Miter);
                    }
                if let Some(path) = arms(*size, 0.0) {
                    stroke(pixmap, &path, *thickness, *color, LineJoin::Miter);
                }
            }
            Prepared::Image { pixmap: image, opacity } => {
                let x = c - image.width() as i32 / 2;
                let y = c - image.height() as i32 / 2;
                let paint = PixmapPaint { opacity: *opacity, ..PixmapPaint::default() };
                pixmap.draw_pixmap(x, y, image.as_ref(), &paint, Transform::identity(), None);
            }
        }
    }
}

fn stroke(pixmap: &mut Pixmap, path: &tiny_skia::Path, width: f32, rgba: Rgba, join: LineJoin) {
    let mut paint = Paint { anti_alias: true, ..Paint::default() };
    paint.set_color_rgba8(rgba.r, rgba.g, rgba.b, rgba.a);
    let stroke = Stroke { width, line_cap: LineCap::Butt, line_join: join, ..Stroke::default() };
    pixmap.stroke_path(path, &paint, &stroke, Transform::identity(), None);
}

/// Renders the crosshair onto a square canvas. The center point is at `(size / 2, size / 2)`.
pub fn render(ch: &Crosshair) -> Pixmap {
    let layers: Vec<Prepared> = ch.layers.iter().filter(|l| l.visible()).filter_map(prepare).collect();
    let extent = layers.iter().map(Prepared::extent).max().unwrap_or(0);
    let size = (2 * (extent + 2)).clamp(8, MAX_CANVAS) as u32;
    let c = (size / 2) as i32;
    let mut canvas = Pixmap::new(size, size).expect("canvas size is non-zero");
    if let [single] = layers.as_slice() {
        single.draw(&mut canvas, c);
        return canvas;
    }
    // Each layer gets its own scratch canvas, so a layer's fill only replaces its own outline.
    let mut scratch = Pixmap::new(size, size).expect("canvas size is non-zero");
    for layer in &layers {
        scratch.fill(Color::TRANSPARENT);
        layer.draw(&mut scratch, c);
        canvas.draw_pixmap(0, 0, scratch.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    }
    canvas
}

fn draw_boxes(pixmap: &mut Pixmap, c: i32, boxes: &[Box2], color: Rgba, outline: Option<(i32, Rgba)>) {
    let mut paint = Paint { anti_alias: false, ..Paint::default() };
    let mut fill = |b: Box2, rgba: Rgba, blend: BlendMode| {
        let Some(rect) = Rect::from_xywh((c + b.x) as f32, (c + b.y) as f32, b.w as f32, b.h as f32) else {
            return;
        };
        paint.set_color_rgba8(rgba.r, rgba.g, rgba.b, rgba.a);
        paint.blend_mode = blend;
        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    };
    if let Some((width, rgba)) = outline {
        for b in boxes {
            fill(b.grow(width), rgba, BlendMode::SourceOver);
        }
    }
    // `Source` replaces the outline underneath, so a translucent fill does not show it through.
    for b in boxes {
        fill(*b, color, BlendMode::Source);
    }
}

// ---------------------------------------------------------------------------------------------
// Images

enum Source {
    Raster(Pixmap),
    Vector(Box<usvg::Tree>),
}

impl Source {
    fn decode(path: &Path) -> Option<Self> {
        let data = std::fs::read(path).ok()?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "svg" {
            let tree = usvg::Tree::from_data(&data, &usvg::Options::default()).ok()?;
            return Some(Source::Vector(Box::new(tree)));
        }
        let pixmap = Pixmap::decode_png(&data).ok()?;
        (pixmap.width() <= MAX_IMAGE && pixmap.height() <= MAX_IMAGE).then_some(Source::Raster(pixmap))
    }

    fn scaled(&self, scale: f32) -> Option<Pixmap> {
        let limit = (MAX_CANVAS - 8) as f32;
        match self {
            Source::Raster(src) => {
                let w = (src.width() as f32 * scale).round().clamp(1.0, limit);
                let h = (src.height() as f32 * scale).round().clamp(1.0, limit);
                let mut out = Pixmap::new(w as u32, h as u32)?;
                let crisp = scale >= 1.0 && scale.fract() == 0.0;
                let paint = PixmapPaint {
                    quality: if crisp { FilterQuality::Nearest } else { FilterQuality::Bicubic },
                    blend_mode: BlendMode::Source,
                    ..PixmapPaint::default()
                };
                let t = Transform::from_scale(w / src.width() as f32, h / src.height() as f32);
                out.draw_pixmap(0, 0, src.as_ref(), &paint, t, None);
                Some(out)
            }
            Source::Vector(tree) => {
                let size = tree.size();
                let s = scale.min(limit / size.width().max(size.height()).max(1.0));
                let w = (size.width() * s).ceil().max(1.0) as u32;
                let h = (size.height() * s).ceil().max(1.0) as u32;
                let mut out = Pixmap::new(w, h)?;
                resvg::render(tree, Transform::from_scale(s, s), &mut out.as_mut());
                Some(out)
            }
        }
    }
}

type CacheEntry = (Option<SystemTime>, Option<Rc<Source>>);

thread_local! {
    static IMAGE_CACHE: RefCell<HashMap<PathBuf, CacheEntry>> = RefCell::new(HashMap::new());
}

/// Decodes an image once per file version.
fn load_source(path: &Path) -> Option<Rc<Source>> {
    let stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    IMAGE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((cached_stamp, source)) = cache.get(path)
            && *cached_stamp == stamp {
                return source.clone();
            }
        let source = stamp.and_then(|_| Source::decode(path)).map(Rc::new);
        cache.insert(path.to_path_buf(), (stamp, source.clone()));
        source
    })
}

/// True if the image file of a layer can be read and decoded.
pub fn image_ok(file: &str) -> bool {
    !file.trim().is_empty() && load_source(&config::image_path(file)).is_some()
}

// ---------------------------------------------------------------------------------------------
// Pixel formats

/// Converts premultiplied RGBA to premultiplied BGRA, the layout `UpdateLayeredWindow` expects.
pub fn to_bgra(pixmap: &Pixmap) -> Vec<u8> {
    let mut out = pixmap.data().to_vec();
    for px in out.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    out
}

/// Straight (non-premultiplied) BGRA, the layout icon bitmaps expect.
pub fn to_bgra_straight(pixmap: &Pixmap) -> Vec<u8> {
    pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.blue(), c.green(), c.red(), c.alpha()]
        })
        .collect()
}

/// Straight RGBA, for window icons.
pub fn to_rgba_straight(pixmap: &Pixmap) -> Vec<u8> {
    pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opaque(p: &Pixmap, x: i32, y: i32) -> bool {
        p.pixel(x as u32, y as u32).is_some_and(|c| c.alpha() > 0)
    }

    /// Columns that contain any visible pixel.
    fn painted_columns(p: &Pixmap) -> Vec<i32> {
        let n = p.width() as i32;
        (0..n).filter(|&x| (0..n).any(|y| opaque(p, x, y))).collect()
    }

    fn painted_rows(p: &Pixmap) -> Vec<i32> {
        let n = p.height() as i32;
        (0..n).filter(|&y| (0..n).any(|x| opaque(p, x, y))).collect()
    }

    fn one(layer: Layer) -> Crosshair {
        Crosshair { layers: vec![layer] }
    }

    fn plain(thickness: u32, length: u32, gap: i32) -> CrossLayer {
        CrossLayer { thickness, length, gap, outline: Outline::default(), ..CrossLayer::default() }
    }

    fn center(p: &Pixmap) -> i32 {
        (p.width() / 2) as i32
    }

    #[test]
    fn even_thickness_is_symmetric_around_center() {
        let p = render(&one(Layer::Cross(plain(2, 5, 3))));
        let c = center(&p);
        let cols = painted_columns(&p);
        let (lo, hi) = (*cols.first().unwrap(), *cols.last().unwrap() + 1);
        assert_eq!(c - lo, hi - c, "arms reach equally far left and right");
        assert_eq!(c - lo, 3 + 5);
        assert!(opaque(&p, c - 1, c - 4) && opaque(&p, c, c - 4));
        assert!(!opaque(&p, c - 2, c - 4) && !opaque(&p, c + 1, c - 4));
    }

    #[test]
    fn odd_thickness_is_symmetric_around_center_pixel() {
        let p = render(&one(Layer::Cross(plain(1, 4, 2))));
        let c = center(&p);
        let cols = painted_columns(&p);
        let (lo, hi) = (*cols.first().unwrap(), *cols.last().unwrap());
        assert_eq!(c - lo, hi - c);
        assert_eq!(c - lo, 2 + 4);
        assert!(!opaque(&p, c, c), "center stays free without dot");
    }

    #[test]
    fn gap_is_respected() {
        let p = render(&one(Layer::Cross(plain(2, 4, 3))));
        let c = center(&p);
        for x in c..c + 3 {
            assert!(!opaque(&p, x, c), "gap pixel {x} must be empty");
        }
        assert!(opaque(&p, c + 3, c));
    }

    #[test]
    fn separate_vertical_length() {
        let p = render(&one(Layer::Cross(CrossLayer { vertical_length: Some(10), ..plain(2, 4, 2) })));
        let c = center(&p);
        let cols = painted_columns(&p);
        let rows = painted_rows(&p);
        assert_eq!((c - cols[0], cols.last().unwrap() + 1 - c), (2 + 4, 2 + 4), "horizontal arms use length");
        assert_eq!((c - rows[0], rows.last().unwrap() + 1 - c), (2 + 10, 2 + 10), "vertical arms use their own length");
        // Vertical arms only.
        let p = render(&one(Layer::Cross(CrossLayer { vertical_length: Some(5), ..plain(2, 0, 1) })));
        assert_eq!(painted_columns(&p).len(), 2);
    }

    #[test]
    fn t_style_drops_top_arm() {
        let p = render(&one(Layer::Cross(CrossLayer { t_style: true, ..plain(2, 6, 2) })));
        let c = center(&p);
        assert!(!opaque(&p, c, c - 5));
        assert!(opaque(&p, c, c + 5));
    }

    #[test]
    fn outline_surrounds_fill() {
        let p = render(&one(Layer::Cross(CrossLayer { outline: Outline { enabled: true, ..Outline::default() }, ..plain(2, 4, 2) })));
        let c = center(&p);
        let fill = p.pixel((c + 3) as u32, c as u32).unwrap().demultiply();
        let edge = p.pixel((c + 3) as u32, (c + 1) as u32).unwrap().demultiply();
        assert_eq!((fill.red(), fill.green(), fill.blue()), (0, 255, 0x66));
        assert_eq!((edge.red(), edge.green(), edge.blue(), edge.alpha()), (0, 0, 0, 255));
    }

    #[test]
    fn translucent_fill_hides_outline_below() {
        let layer = CrossLayer {
            color: "#FFFFFF80".into(),
            outline: Outline { enabled: true, ..Outline::default() },
            ..plain(2, 4, 2)
        };
        let p = render(&one(Layer::Cross(layer)));
        let c = center(&p);
        let fill = p.pixel((c + 3) as u32, c as u32).unwrap().demultiply();
        assert_eq!(fill.alpha(), 0x80);
        assert!(fill.red() > 250);
    }

    #[test]
    fn canvas_contains_everything() {
        let big = Crosshair {
            layers: vec![
                Layer::Cross(CrossLayer {
                    length: 200,
                    gap: 100,
                    outline: Outline { enabled: true, thickness: 8, ..Outline::default() },
                    ..CrossLayer::default()
                }),
                Layer::Circle(CircleLayer { radius: 200, thickness: 32, ..CircleLayer::default() }),
                Layer::Chevron(ChevronLayer { size: 100, angle: 160, gap: 100, ..ChevronLayer::default() }),
                Layer::Chevron(ChevronLayer { size: 100, angle: 20, gap: -50, ..ChevronLayer::default() }),
            ],
        };
        for layers in [big.layers.clone(), vec![big.layers[1].clone()], vec![big.layers[2].clone()], vec![big.layers[3].clone()]] {
            let p = render(&Crosshair { layers });
            let n = p.width() as i32;
            let edge_touched =
                (0..n).any(|i| opaque(&p, 0, i) || opaque(&p, n - 1, i) || opaque(&p, i, 0) || opaque(&p, i, n - 1));
            assert!(!edge_touched);
            assert_eq!(p.width() % 2, 0);
        }
    }

    #[test]
    fn dot_only() {
        let p = render(&one(Layer::Dot(DotLayer { size: 3, outline: Outline::default(), ..DotLayer::default() })));
        let c = center(&p);
        assert!(opaque(&p, c - 1, c - 1) && opaque(&p, c + 1, c + 1));
        assert!(!opaque(&p, c + 2, c));
    }

    #[test]
    fn circle_is_symmetric_and_hollow() {
        let p = render(&one(Layer::Circle(CircleLayer { radius: 10, thickness: 2, ..CircleLayer::default() })));
        let c = center(&p);
        let cols = painted_columns(&p);
        let rows = painted_rows(&p);
        assert_eq!(c - cols[0], cols.last().unwrap() + 1 - c);
        assert_eq!(c - rows[0], rows.last().unwrap() + 1 - c);
        assert!(!opaque(&p, c, c));
        assert!(opaque(&p, c + 10, c));
    }

    #[test]
    fn chevron_points_up_from_center() {
        let p = render(&one(Layer::Chevron(ChevronLayer { size: 10, gap: 0, outline: Outline::default(), ..ChevronLayer::default() })));
        let c = center(&p);
        let rows = painted_rows(&p);
        assert!(rows[0] >= c - 2, "tip sits at the center, nothing far above it");
        assert!(*rows.last().unwrap() > c + 5, "arms go down");
        let cols = painted_columns(&p);
        assert_eq!(c - cols[0], cols.last().unwrap() + 1 - c, "left and right arm mirror each other");
    }

    #[test]
    fn hidden_layers_are_skipped() {
        let p = render(&Crosshair {
            layers: vec![
                Layer::Dot(DotLayer { visible: false, ..DotLayer::default() }),
                Layer::Circle(CircleLayer { visible: false, ..CircleLayer::default() }),
            ],
        });
        assert!(painted_columns(&p).is_empty());
    }

    #[test]
    fn layers_stack_in_order() {
        let p = render(&Crosshair {
            layers: vec![
                Layer::Dot(DotLayer { color: "#FF0000".into(), size: 6, outline: Outline::default(), ..DotLayer::default() }),
                Layer::Dot(DotLayer { color: "#0000FF".into(), size: 2, outline: Outline::default(), ..DotLayer::default() }),
            ],
        });
        let c = center(&p);
        let top = p.pixel(c as u32, c as u32).unwrap();
        let rim = p.pixel((c + 2) as u32, c as u32).unwrap();
        assert_eq!((top.red(), top.blue()), (0, 255));
        assert_eq!((rim.red(), rim.blue()), (255, 0));
    }

    #[test]
    fn png_and_svg_images_render_centered() {
        let dir = std::env::temp_dir().join(format!("reticle-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut src = Pixmap::new(4, 4).unwrap();
        src.fill(Color::from_rgba8(255, 255, 255, 255));
        let png = dir.join("square.png");
        src.save_png(&png).unwrap();
        let svg = dir.join("ring.svg");
        std::fs::write(
            &svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><circle cx="5" cy="5" r="4" fill="none" stroke="red" stroke-width="2"/></svg>"#,
        )
        .unwrap();

        let image = |file: &Path, scale: f32| {
            one(Layer::Image(ImageLayer { file: file.to_string_lossy().into(), scale, ..ImageLayer::default() }))
        };
        let p = render(&image(&png, 2.0));
        let c = center(&p);
        assert_eq!(painted_columns(&p), ((c - 4)..(c + 4)).collect::<Vec<_>>());

        let p = render(&image(&svg, 3.0));
        let cols = painted_columns(&p);
        assert!(cols.len() >= 28 && cols.len() <= 31, "svg scaled to ~30 px, got {}", cols.len());

        assert!(image_ok(&png.to_string_lossy()));
        assert!(!image_ok(&dir.join("missing.png").to_string_lossy()));
        let p = render(&one(Layer::Image(ImageLayer { file: "missing.png".into(), ..ImageLayer::default() })));
        assert!(painted_columns(&p).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pixel_formats_have_full_length() {
        let p = render(&Crosshair::default());
        let n = (p.width() * p.height() * 4) as usize;
        assert_eq!((to_bgra(&p).len(), to_bgra_straight(&p).len(), to_rgba_straight(&p).len()), (n, n, n));
    }
}
