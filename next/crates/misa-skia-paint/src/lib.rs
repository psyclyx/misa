//! Shared Skia canvas painting for raster exports and Ganesh targets.
use misa_pixel_ui::{LineMetrics, Op, Scene, TextMetrics};
use misa_style::Color;
use skia_safe::{
    Canvas, Font, FontMgr, FontStyle, Paint as SkPaint, PaintStyle, RRect, Rect, Typeface, surfaces,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

/// The font stack: the face the user's font configuration resolves first, plus
/// a per-glyph fallback for whatever that face cannot draw. Measurement and
/// painting share one stack, so a line lays out exactly as it draws — including
/// the glyphs the primary face is missing.
pub struct Fonts {
    primary: Typeface,
    /// Faces chosen for characters the primary face cannot draw. `None` is
    /// cached too: no installed face covers the character, and that answer is
    /// stable for the process.
    fallbacks: Mutex<HashMap<char, Option<Typeface>>>,
    /// Sizing is keyed by the face and the exact size: measuring a run and
    /// painting it must use the same Skia Font, not two configurations.
    sized: Mutex<HashMap<(u32, u32), Font>>,
}

// Cache only successful resolution. A missing font remains retryable if the
// system's installed fonts change; the lock serializes first-use resolution.
static FONTS: OnceLock<Fonts> = OnceLock::new();
static FONTS_INIT: Mutex<()> = Mutex::new(());

fn resolve_typeface(fonts: &FontMgr) -> Result<Typeface, String> {
    // "monospace" is fontconfig's alias for whatever the user configured; only
    // when their configuration offers nothing does any installed face do.
    fonts
        .match_family_style("monospace", FontStyle::default())
        .or_else(|| {
            fonts
                .family_names()
                .find_map(|family| fonts.match_family_style(family, FontStyle::default()))
        })
        .ok_or_else(|| "no typeface available; install a font".to_string())
}

impl Fonts {
    fn new() -> Result<Fonts, String> {
        let mgr = FontMgr::default();
        Ok(Fonts {
            primary: resolve_typeface(&mgr)?,
            fallbacks: Mutex::new(HashMap::new()),
            sized: Mutex::new(HashMap::new()),
        })
    }

    /// The face the user's font configuration resolves first.
    pub fn primary(&self) -> &Typeface {
        &self.primary
    }

    fn font(&self, face: &Typeface, size: f32) -> Font {
        self.sized
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .entry((face.unique_id(), size.to_bits()))
            .or_insert_with(|| Font::from_typeface(face.clone(), size))
            .clone()
    }

    /// The face that draws `ch`, or `None` when the primary face does.
    pub fn fallback(&self, ch: char) -> Option<Typeface> {
        let mut cache = self.fallbacks.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(cached) = cache.get(&ch) {
            return cached.clone();
        }
        // Prefer a face from the user's own configuration; only then any face
        // that can draw the character at all. The manager is created per
        // resolution (cache misses only): Skia's is not shareable across
        // threads, and the resolved faces are.
        let mgr = FontMgr::default();
        let found = mgr
            .match_family_style_character("monospace", FontStyle::default(), &[], ch as i32)
            .filter(|face| face.unichar_to_glyph(ch as i32) != 0)
            .or_else(|| {
                (0..mgr.count_families()).find_map(|index| {
                    mgr.match_family_style(&mgr.family_name(index), FontStyle::default())
                        .filter(|face| face.unichar_to_glyph(ch as i32) != 0)
                })
            });
        cache.insert(ch, found.clone());
        found
    }

    fn font_for(&self, face: Option<&Typeface>, size: f32) -> Font {
        self.font(face.unwrap_or(&self.primary), size)
    }

    /// Split `text` into runs of one face, in order. Only characters the
    /// primary face cannot draw leave it.
    pub fn runs<'a>(&self, text: &'a str, size: f32) -> Vec<(Font, &'a str)> {
        let mut runs: Vec<(Font, &str)> = Vec::new();
        let mut start = 0usize;
        let mut face: Option<Typeface> = None; // None: the primary face
        for (index, ch) in text.char_indices() {
            let next = if self.primary.unichar_to_glyph(ch as i32) != 0 {
                None
            } else {
                self.fallback(ch)
            };
            let same = match (&face, &next) {
                (None, None) => true,
                (Some(have), Some(want)) => have.unique_id() == want.unique_id(),
                _ => false,
            };
            if !same {
                if start < index {
                    runs.push((self.font_for(face.as_ref(), size), &text[start..index]));
                }
                start = index;
                face = next;
            }
        }
        if start < text.len() {
            runs.push((self.font_for(face.as_ref(), size), &text[start..]));
        }
        runs
    }

    /// The advance width of `text`, measured run by run with the same fonts
    /// that paint it.
    pub fn measure(&self, text: &str, size: f32) -> f32 {
        self.runs(text, size)
            .into_iter()
            .map(|(font, run)| font.measure_str(run, None).0)
            .sum()
    }

    /// Per-character advance boundaries: `advances[i]` is the x of the i-th
    /// character boundary, and the last equals [`Self::measure`].
    pub fn advances(&self, text: &str, size: f32) -> Vec<f32> {
        let mut advances = vec![0.0];
        let mut x = 0.0;
        for (font, run) in self.runs(text, size) {
            // draw_str and measure_str both convert UTF-8 to glyph IDs without
            // shaping; get_widths uses the same glyph advance as measure_str.
            let glyphs = font.text_to_glyphs_vec(run);
            let mut widths = vec![0.0; glyphs.len()];
            font.get_widths(&glyphs, &mut widths);
            for width in widths {
                x += width;
                advances.push(x);
            }
        }
        debug_assert_eq!(advances.len(), text.chars().count() + 1);
        advances
    }

    /// The line box of the primary face. A fallback glyph taller than it is
    /// painted in its own metrics but reserves no extra line height, so layout
    /// stays stable across fonts.
    pub fn line_metrics(&self, size: f32) -> LineMetrics {
        let (line_height, metrics) = self.font(&self.primary, size).metrics();
        LineMetrics {
            ascent: metrics.ascent,
            descent: metrics.descent,
            leading: metrics.leading,
            line_height,
        }
    }
}

/// The shared font stack. Cache only successful resolution: a missing font
/// remains retryable if the system's installed fonts change.
fn fonts() -> Result<&'static Fonts, String> {
    if let Some(fonts) = FONTS.get() {
        return Ok(fonts);
    }
    let _guard = FONTS_INIT.lock().unwrap_or_else(|err| err.into_inner());
    if FONTS.get().is_none() {
        FONTS
            .set(Fonts::new()?)
            .ok()
            .expect("fonts initialized under lock");
    }
    Ok(FONTS.get().expect("fonts initialized under lock"))
}

/// Skia measurements backed by the painter's own font stack.
pub struct SkiaTextMetrics {
    fonts: &'static Fonts,
}

/// Resolve the font stack now, rather than deferring a missing-font failure to
/// a layout or draw call. No substitute measurement is used in production.
pub fn text_metrics() -> Result<Arc<dyn TextMetrics>, String> {
    Ok(Arc::new(SkiaTextMetrics { fonts: fonts()? }))
}

impl TextMetrics for SkiaTextMetrics {
    fn measure(&self, text: &str, size: f32) -> f32 {
        self.fonts.measure(text, size)
    }

    fn advances(&self, text: &str, size: f32) -> Vec<f32> {
        self.fonts.advances(text, size)
    }

    fn line_metrics(&self, size: f32) -> LineMetrics {
        self.fonts.line_metrics(size)
    }
}

#[cfg(test)]
mod metrics_tests {
    use super::*;

    #[test]
    fn glyph_advances_match_every_prefix_and_styled_run_width() {
        let metrics = text_metrics().unwrap();
        let long = "WWiiii界🙂e\u{301}".repeat(30);
        for size in [11.0, 15.0, 17.5, 32.0] {
            for text in [
                "",
                "Hello world!",
                "ill MW",
                "界你好λ🙂",
                "e\u{301} cafe\u{301}",
                "👩‍💻🇺🇳",
                "\u{0301}\u{0308}a",
                "a\t b\n",
                "ﬁÆ—─▏•",
                long.as_str(),
            ] {
                let advances = metrics.advances(text, size);
                assert_eq!(advances.len(), text.chars().count() + 1);
                for (index, (start, ch)) in text.char_indices().enumerate() {
                    let end = start + ch.len_utf8();
                    let expected = metrics.measure(&text[..end], size);
                    assert_eq!(
                        advances[index + 1],
                        expected,
                        "{text:?} at {end}, size {size}"
                    );
                }
                assert_eq!(*advances.last().unwrap(), metrics.measure(text, size));
            }
            for runs in [
                &["Hello ", "界e\u{301}", "🙂 world"][..],
                &["e", "\u{301}", "ﬁ", "👩‍💻", " ill MW"][..],
            ] {
                for run in runs {
                    assert_eq!(
                        *metrics.advances(run, size).last().unwrap(),
                        metrics.measure(run, size)
                    );
                }
            }
        }
    }

    #[test]
    fn fallback_faces_cover_what_the_primary_face_cannot_draw() {
        let fonts = fonts().unwrap();
        for ch in ['界', '🙂', '😀', '→', '🟠', '한'] {
            if fonts.primary().unichar_to_glyph(ch as i32) != 0 {
                continue; // this machine's primary face draws it
            }
            let face = fonts
                .fallback(ch)
                .unwrap_or_else(|| panic!("{ch} has no fallback face"));
            assert_ne!(
                face.unichar_to_glyph(ch as i32),
                0,
                "{ch} fallback must be a real glyph"
            );
            // Whatever the fallback adds to a line, runs split at its boundary
            // and measurement stays exactly where painting advances to.
            let text = format!("a{ch}b");
            let runs = fonts.runs(&text, 15.0);
            assert!(runs.len() >= 2, "{ch} must leave the primary face's run");
            let advances = fonts.advances(&text, 15.0);
            assert_eq!(
                advances[2] - advances[1],
                fonts.measure(&ch.to_string(), 15.0),
                "{ch} measures as its own run"
            );
        }
    }
}

/// Non-overlapping wall-clock phases of a fresh raster. PNG encoding is excluded.
#[derive(Clone, Copy, Debug, Default)]
pub struct RasterPhases {
    pub surface_clear: Duration,
    pub font_resolver: Duration,
    pub draw_ops: Duration,
    pub pixel_read_convert: Duration,
}

fn skia_color(color: Color, fallback: u32) -> u32 {
    match color {
        Color::Default => fallback,
        Color::Indexed(index) => 0xff00_0000 | ((index as u32) * 0x0001_0101),
        Color::Rgb(r, g, b) => 0xff00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32,
    }
}

/// Draw a scene to a raster surface and return the pixels.
///
/// Grayscale antialiasing and no GPU: a view is text, and a text renderer that
/// needs a device is a text renderer nobody can test.
pub fn raster(scene: &Scene, background: Color) -> Result<image::RgbaImage, String> {
    raster_impl::<false, true>(scene, background).map(|(pixels, _)| pixels)
}

/// Benchmark-only entry point. Identical paint work to `raster`, with phase clocks.
pub fn raster_profiled(
    scene: &Scene,
    background: Color,
) -> Result<(image::RgbaImage, RasterPhases), String> {
    raster_impl::<true, true>(scene, background)
}

/// Offline benchmark reference: original per-frame font resolution, with
/// otherwise identical painting. Never selected by the production API.
pub fn raster_uncached_profiled(
    scene: &Scene,
    background: Color,
) -> Result<(image::RgbaImage, RasterPhases), String> {
    raster_impl::<true, false>(scene, background)
}

// PROFILE and CACHE are compile-time constants: production has no phase
// clocks or runtime mode switch. Both paths execute identical paint work.
fn raster_impl<const PROFILE: bool, const CACHE: bool>(
    scene: &Scene,
    background: Color,
) -> Result<(image::RgbaImage, RasterPhases), String> {
    let mut phases = RasterPhases::default();
    let start = if PROFILE { Some(Instant::now()) } else { None };
    let width = scene.width.ceil().max(1.0) as i32;
    let height = scene.height.ceil().max(1.0) as i32;
    let mut surface = surfaces::raster_n32_premul((width, height)).ok_or("no raster surface")?;
    let canvas: &Canvas = surface.canvas();
    canvas.clear(skia_safe::Color::from(skia_color(background, 0xff14_161a)));
    if PROFILE {
        phases.surface_clear = start.expect("profile clock").elapsed();
    }

    let start = if PROFILE { Some(Instant::now()) } else { None };
    // The uncached reference rebuilds the whole stack per frame, as the
    // original implementation resolved its typeface per frame.
    let uncached;
    let fonts: &Fonts = if CACHE {
        fonts()?
    } else {
        uncached = Fonts::new()?;
        &uncached
    };
    if PROFILE {
        phases.font_resolver = start.expect("profile clock").elapsed();
    }

    let start = if PROFILE { Some(Instant::now()) } else { None };
    let mut fill = SkPaint::default();
    fill.set_anti_alias(true);
    draw_ops(canvas, &scene.ops, fonts, &mut fill);
    if PROFILE {
        phases.draw_ops = start.expect("profile clock").elapsed();
    }

    let start = if PROFILE { Some(Instant::now()) } else { None };
    let pixmap = surface.peek_pixels().ok_or("no pixels")?;
    let bytes = pixmap.bytes().ok_or("no pixel bytes")?;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for pixel in bytes.chunks_exact(4) {
        // N32 premultiplied is BGRA on a little-endian machine.
        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    let pixels = image::RgbaImage::from_raw(width as u32, height as u32, rgba)
        .ok_or_else(|| "no image".to_string())?;
    if PROFILE {
        phases.pixel_read_convert = start.expect("profile clock").elapsed();
    }
    Ok((pixels, phases))
}

/// Clear and paint a scene on any Skia canvas (raster or Ganesh).
/// The caller owns the surface and any readback.
/// Draw a scene laid out in logical pixels to a canvas of the same pixel size.
pub fn draw_scene(canvas: &Canvas, scene: &Scene, background: Color) -> Result<(), String> {
    draw_scene_scaled(canvas, scene, background, 1.0)
}

/// Draw a scene laid out in logical pixels onto a device canvas, where one
/// logical pixel is `scale` device pixels. Layout and measurement stay in
/// logical units; only presentation converts to device pixels, so text and
/// hit targets keep the same size at every display density.
pub fn draw_scene_scaled(
    canvas: &Canvas,
    scene: &Scene,
    background: Color,
    scale: f32,
) -> Result<(), String> {
    assert!(
        scale.is_finite() && scale > 0.0,
        "device scale must be positive"
    );
    canvas.clear(skia_safe::Color::from(skia_color(background, 0xff14_161a)));
    let mut fill = SkPaint::default();
    fill.set_anti_alias(true);
    canvas.save();
    canvas.scale((scale, scale));
    draw_ops(canvas, &scene.ops, fonts()?, &mut fill);
    canvas.restore();
    Ok(())
}

/// Paint the retained operations onto any Skia canvas (raster or Ganesh).
/// The caller owns the surface, clear, and readback; paint order is identical.
fn draw_ops(canvas: &Canvas, ops: &[Op], fonts: &Fonts, fill: &mut SkPaint) {
    for op in ops {
        match op {
            Op::Group { x, y, ops } => {
                canvas.save();
                canvas.translate((*x, *y));
                draw_ops(canvas, ops, fonts, fill);
                canvas.restore();
            }
            Op::ClipRect {
                x,
                y,
                width,
                height,
                ops,
            } => {
                canvas.save();
                canvas.clip_rect(Rect::from_xywh(*x, *y, *width, *height), None, true);
                draw_ops(canvas, ops, fonts, fill);
                canvas.restore();
            }
            Op::Image {
                x,
                y,
                width,
                height,
                image,
            } => {
                let info = skia_safe::ImageInfo::new(
                    (image.width() as i32, image.height() as i32),
                    skia_safe::ColorType::RGBA8888,
                    skia_safe::AlphaType::Unpremul,
                    None,
                );
                let data = skia_safe::Data::new_copy(image.as_raw());
                if let Some(bitmap) =
                    skia_safe::images::raster_from_data(&info, data, image.width() as usize * 4)
                {
                    canvas.draw_image_rect(
                        bitmap,
                        None,
                        Rect::from_xywh(*x, *y, *width, *height),
                        fill,
                    );
                }
            }
            Op::Rect {
                x,
                y,
                width,
                height,
                style,
            } => {
                fill.set_style(PaintStyle::Fill);
                fill.set_color(skia_safe::Color::from(skia_color(style.fg, 0xff9a_a2ad)));
                canvas.draw_rect(Rect::from_xywh(*x, *y, *width, *height), fill);
            }
            Op::RoundedRect {
                x,
                y,
                width,
                height,
                radius,
                style,
            } => {
                fill.set_style(PaintStyle::Fill);
                fill.set_color(skia_safe::Color::from(skia_color(style.fg, 0xff9a_a2ad)));
                // The radius is the corner radius, clamped to what fits.
                let corner = radius.min(*width / 2.0).min(*height / 2.0).max(0.0);
                let rect =
                    RRect::new_rect_xy(Rect::from_xywh(*x, *y, *width, *height), corner, corner);
                canvas.draw_rrect(rect, fill);
            }
            Op::Text {
                x,
                y,
                size,
                style,
                text,
            } => {
                fill.set_style(PaintStyle::Fill);
                fill.set_color(skia_safe::Color::from(skia_color(style.fg, 0xffe9_ebee)));
                // Dimmed text steps back from the page rather than changing
                // hue: reasoning aloud should read lighter than the answer.
                if style.dim {
                    fill.set_alpha(170);
                } else {
                    fill.set_alpha(255);
                }
                // The scene's y is the line top; every run sits on the primary
                // face's baseline and advances by its own measured width, so
                // painting matches measurement run for run.
                let baseline = *y - fonts.line_metrics(*size).ascent;
                let mut x = *x;
                for (font, run) in fonts.runs(text, *size) {
                    canvas.draw_str(run, (x, baseline), &font, fill);
                    x += font.measure_str(run, None).0;
                }
            }
        }
    }
}

/// A scene as PNG bytes.
pub fn png(scene: &Scene, background: Color) -> Result<Vec<u8>, String> {
    let image = raster(scene, background)?;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Png)
        .map_err(|err| err.to_string())?;
    Ok(bytes.into_inner())
}
