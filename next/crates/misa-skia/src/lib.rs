//! The pixel frontend: the semantic tree as a scene, and as a raster.
//!
//! # What is shared and what is not
//!
//! A pixel frontend cannot use the terminal's lines: it has no cells, and it wants
//! real type. What it *can* share is everything that is not a cell —
//! [`misa_render::text`] for measurement, [`misa_render::Theme`] for what a role
//! looks like, and the tree itself.
//!
//! So this frontend maps the same tree to a **scene** of positioned runs and
//! rectangles, and that mapping is the interesting part: it is where a role becomes
//! a colour and a weight, where a rail becomes a bar, and where a code block becomes
//! a raised panel. It is testable with no window and no GPU, which is why the scene
//! and the raster are both built and tested by default.
//!
//! # Known limitation, stated rather than implied
//!
//! The scene's *text flow* is still the linear renderer's: it wraps to a column
//! count and stacks runs. A pixel frontend that wants proportional type, reflow, or
//! a non-linear layout (a rail that spans a message, a code block that scrolls
//! sideways) needs a layout of its own, and that is the next piece of work here. The
//! mapping from role to appearance, and the fact that no session is involved in it,
//! already hold.

use misa_proto::view::Node;
use misa_render::{Line, Style, Theme};

/// One thing to draw.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// A retained local scene, positioned without rebuilding its paint operations.
    Group { x:f32, y:f32, ops:std::sync::Arc<Vec<Op>> },
    Image { x: f32, y: f32, width: f32, height: f32, image: std::sync::Arc<image::RgbaImage> },
    /// A run of text at a baseline position.
    Text { x: f32, y: f32, size: f32, style: Style, text: String },
    /// A filled rectangle, in device pixels.
    Rect { x: f32, y: f32, width: f32, height: f32, style: Style },
}

/// A drawable frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub width: f32,
    pub height: f32,
    pub ops: Vec<Op>,
}

/// The measurements a scene is laid out with.
///
/// Passed in rather than assumed, because a frontend that guesses at its advance
/// width mis-measures every line by a little and the result looks wrong for reasons
/// nobody can name.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub advance: f32,
    pub line_height: f32,
    pub margin: f32,
    pub font_size: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Layout { advance: 8.4, line_height: 21.0, margin: 24.0, font_size: 15.0 }
    }
}

/// Build a scene from a view tree.
///
/// The one decision this makes that the terminal renderer does not: a node with a
/// rail gets a *drawn bar* beside it rather than a glyph, because a pixel frontend
/// can do that and the rail is what makes a message's extent visible.
pub fn scene(view: &Node, theme: &Theme, columns: usize, rows: usize, layout: Layout) -> Scene {
    let lines = misa_render::render(view, theme, columns);
    let mut scene = Scene {
        width: layout.margin * 2.0 + columns as f32 * layout.advance,
        height: layout.margin * 2.0 + rows as f32 * layout.line_height,
        ops: Vec::new(),
    };
    for (index, line) in lines.iter().take(rows).enumerate() {
        let y = layout.margin + index as f32 * layout.line_height;
        rail(line, theme, &mut scene, layout, y);
        let mut x = layout.margin + line.indent as f32 * layout.advance;
        for (style, text) in &line.spans {
            if !text.is_empty() {
                scene.ops.push(Op::Text {
                    x,
                    y,
                    size: layout.font_size,
                    style: *style,
                    text: text.clone(),
                });
            }
            x += misa_render::width(text) as f32 * layout.advance;
        }
    }
    scene
}

/// A rail is drawn, not typed.
fn rail(line: &Line, theme: &Theme, scene: &mut Scene, layout: Layout, y: f32) {
    let Some(node) = &line.node else {
        return;
    };
    // The role is not on the line, so the rail is derived from the node id's prefix
    // when the caller set one. A frontend that wants a rail per role would carry the
    // role on the line; the linear renderer drops it because a terminal draws the
    // glyph instead.
    let role = node.rsplit_once('.').map(|(head, _)| head).unwrap_or(node);
    let Some((_, style)) = theme.rail(role) else {
        return;
    };
    let x = layout.margin + line.indent as f32 * layout.advance - layout.advance;
    scene.ops.push(Op::Rect {
        x,
        y,
        width: 2.0,
        height: layout.line_height - 4.0,
        style,
    });
}

/// The scene as a PNG.

pub mod app;
pub mod connection;
pub mod window;
pub mod workspace;
mod preferences;
pub mod appearance;

pub mod paint {
    use super::{Op, Scene};
    use misa_render::Color;
    use skia_safe::{surfaces, Canvas, Font, FontMgr, FontStyle, Paint as SkPaint, PaintStyle, Rect};

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
        let width = scene.width.ceil().max(1.0) as i32;
        let height = scene.height.ceil().max(1.0) as i32;
        let mut surface = surfaces::raster_n32_premul((width, height)).ok_or("no raster surface")?;
        let canvas: &Canvas = surface.canvas();
        canvas.clear(skia_safe::Color::from(skia_color(background, 0xff14_161a)));

        let fonts = FontMgr::default();
        let typeface = fonts.match_family_style("monospace", FontStyle::default())
            .or_else(|| fonts.family_names().find_map(|family| {
                fonts.match_family_style(family, FontStyle::default())
            }))
            .ok_or("no typeface available; install a font")?;
        let mut fill = SkPaint::default();
        fill.set_anti_alias(true);

        draw_ops(canvas,&scene.ops,&typeface,&mut fill);

        let pixmap = surface.peek_pixels().ok_or("no pixels")?;
        let bytes = pixmap.bytes().ok_or("no pixel bytes")?;
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for pixel in bytes.chunks_exact(4) {
            // N32 premultiplied is BGRA on a little-endian machine.
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
        image::RgbaImage::from_raw(width as u32, height as u32, rgba).ok_or_else(|| "no image".to_string())
    }

    fn draw_ops(canvas:&Canvas,ops:&[Op],typeface:&skia_safe::Typeface,fill:&mut SkPaint) {
        for op in ops {
            match op {
                Op::Group {x,y,ops}=>{canvas.save();canvas.translate((*x,*y));draw_ops(canvas,ops,typeface,fill);canvas.restore();}
                Op::Image { x, y, width, height, image } => {
                    let info = skia_safe::ImageInfo::new((image.width() as i32, image.height() as i32), skia_safe::ColorType::RGBA8888, skia_safe::AlphaType::Unpremul, None);
                    let data = skia_safe::Data::new_copy(image.as_raw());
                    if let Some(bitmap) = skia_safe::images::raster_from_data(&info, data, image.width() as usize * 4) {
                        canvas.draw_image_rect(bitmap, None, Rect::from_xywh(*x, *y, *width, *height), fill);
                    }
                }
                Op::Rect { x, y, width, height, style } => {
                    fill.set_style(PaintStyle::Fill);
                    fill.set_color(skia_safe::Color::from(skia_color(style.fg, 0xff9a_a2ad)));
                    canvas.draw_rect(Rect::from_xywh(*x, *y, *width, *height), fill);
                }
                Op::Text { x, y, size, style, text } => {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::view::{Kind, Span, State};

    fn view() -> Node {
        Node::section("session")
            .id("session")
            .child(
                Node::text("message.user", [Span::plain("a question")])
                    .id("msg.1")
                    .state(State::Done),
            )
            .child(Node::new(
                "tool.result",
                Kind::Code {
                    lang: Some("rust".into()),
                    text: "let x = 1;".into(),
                    captures: vec![misa_proto::view::Capture { start: 0, end: 3, token: "keyword".into() }],
                },
            ))
    }

    fn scene_of(theme: &Theme) -> Scene {
        scene(&view(), theme, 80, 24, Layout::default())
    }

    #[test]
    fn a_scene_positions_every_run_and_leaves_no_run_unplaced() {
        let scene = scene_of(&Theme::dark());
        assert!(!scene.ops.is_empty());
        assert!(scene.width > 0.0 && scene.height > 0.0);
        for op in &scene.ops {
            match op {
                Op::Text { x, y, text, .. } => {
                    assert!(*x >= 0.0 && *y >= 0.0);
                    assert!(!text.is_empty(), "an empty run was emitted");
                }
                Op::Group { .. } => {},
                Op::Image { width, height, .. } | Op::Rect { width, height, .. } => assert!(*width > 0.0 && *height > 0.0),
            }
        }
    }

    #[test]
    fn a_captured_run_carries_the_themes_colour_for_that_capture() {
        let theme = Theme::dark();
        let scene = scene_of(&theme);
        let keyword = theme.token("keyword");
        assert!(
            scene
                .ops
                .iter()
                .any(|op| matches!(op, Op::Text { style, .. } if *style == keyword)),
            "no run carried the keyword colour"
        );
    }

    #[test]
    fn the_scene_is_a_function_of_the_theme_and_changes_when_it_does() {
        let dark = scene_of(&Theme::dark());
        let plain = scene_of(&Theme::plain());
        let colours = |scene: &Scene| {
            scene
                .ops
                .iter()
                .filter_map(|op| match op {
                    Op::Text { style, .. } => Some(format!("{:?}", style.fg)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_ne!(colours(&dark), colours(&plain), "the theme made no difference");
    }

    #[test]
    fn a_tall_view_is_clipped_to_the_rows_it_was_given() {
        let mut root = Node::section("session");
        for index in 0..200 {
            root = root.child(Node::text("message.assistant", [Span::plain(format!("line {index}"))]));
        }
        let scene = scene(&root, &Theme::dark(), 60, 10, Layout::default());
        let lowest = scene
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Text { y, .. } => Some(*y),
                _ => None,
            })
            .fold(0.0f32, f32::max);
        assert!(lowest < scene.height, "the scene drew past its own height");
    }

    #[test]
    fn the_pixel_frontend_decides_colour_and_the_session_did_not() {
        // The same tree, two themes, two appearances: no colour came from the view.
        let bright = Theme::plain().with_role("message.user", misa_render::Style::rgb(255, 0, 0));
        let scene = scene_of(&bright);
        assert!(scene.ops.iter().any(|op| matches!(
            op,
            Op::Text { style, .. } if style.fg == misa_render::Color::Rgb(255, 0, 0)
        )));
    }

    #[test]
    fn a_scene_paints_to_a_png() {
        let scene = scene_of(&Theme::dark());
        let bytes = paint::png(&scene, misa_render::Color::Rgb(20, 22, 26)).expect("a png");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert!(bytes.len() > 1000, "the raster is suspiciously small: {} bytes", bytes.len());
    }
}
