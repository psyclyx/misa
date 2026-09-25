//! Shared Skia canvas painting for raster exports and Ganesh targets.
use misa_render::Color;
use misa_skia_ui::{Op, Scene};
use skia_safe::{Canvas, Font, FontMgr, FontStyle, Paint as SkPaint, PaintStyle, Rect, surfaces};
use std::{
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

// Cache only successful resolution. A missing font remains retryable if the
// system's installed fonts change; the lock serializes first-use resolution.
static TYPEFACE: OnceLock<skia_safe::Typeface> = OnceLock::new();
static TYPEFACE_INIT: Mutex<()> = Mutex::new(());

fn resolve_typeface(fonts: &FontMgr) -> Result<skia_safe::Typeface, String> {
    fonts
        .match_family_style("monospace", FontStyle::default())
        .or_else(|| {
            fonts
                .family_names()
                .find_map(|family| fonts.match_family_style(family, FontStyle::default()))
        })
        .ok_or_else(|| "no typeface available; install a font".to_string())
}

/// Reuse the same resolved typeface on raster and GPU canvases.
fn cached_typeface() -> Result<&'static skia_safe::Typeface, String> {
    if let Some(typeface) = TYPEFACE.get() {
        return Ok(typeface);
    }
    let _guard = TYPEFACE_INIT.lock().unwrap_or_else(|err| err.into_inner());
    if TYPEFACE.get().is_none() {
        let fonts = FontMgr::default();
        TYPEFACE
            .set(resolve_typeface(&fonts)?)
            .ok()
            .expect("typeface initialized under lock");
    }
    Ok(TYPEFACE.get().expect("typeface initialized under lock"))
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
    // Keep the manager alive through painting in the uncached reference,
    // as in the original per-frame implementation.
    let fonts = if CACHE {
        None
    } else {
        Some(FontMgr::default())
    };
    let typeface = if CACHE {
        cached_typeface()?.clone()
    } else {
        resolve_typeface(fonts.as_ref().expect("uncached font manager"))?
    };
    if PROFILE {
        phases.font_resolver = start.expect("profile clock").elapsed();
    }

    let start = if PROFILE { Some(Instant::now()) } else { None };
    let mut fill = SkPaint::default();
    fill.set_anti_alias(true);
    draw_ops(canvas, &scene.ops, &typeface, &mut fill);
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
pub fn draw_scene(canvas: &Canvas, scene: &Scene, background: Color) -> Result<(), String> {
    canvas.clear(skia_safe::Color::from(skia_color(background, 0xff14_161a)));
    let mut fill = SkPaint::default();
    fill.set_anti_alias(true);
    draw_ops(canvas, &scene.ops, cached_typeface()?, &mut fill);
    Ok(())
}

/// Paint the retained operations onto any Skia canvas (raster or Ganesh).
/// The caller owns the surface, clear, and readback; paint order is identical.
fn draw_ops(canvas: &Canvas, ops: &[Op], typeface: &skia_safe::Typeface, fill: &mut SkPaint) {
    for op in ops {
        match op {
            Op::Group { x, y, ops } => {
                canvas.save();
                canvas.translate((*x, *y));
                draw_ops(canvas, ops, typeface, fill);
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
            Op::Text {
                x,
                y,
                size,
                style,
                text,
            } => {
                let font = Font::from_typeface(typeface.clone(), *size);
                fill.set_style(PaintStyle::Fill);
                fill.set_color(skia_safe::Color::from(skia_color(style.fg, 0xffe9_ebee)));
                // The scene positions a baseline; a line's y is its top.
                canvas.draw_str(text, (*x, *y + size * 0.85), &font, fill);
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
