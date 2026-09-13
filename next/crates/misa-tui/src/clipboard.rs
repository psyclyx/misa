//! Desktop clipboard capability, separate from terminal key handling and blob transport.
#[cfg(test)]
use std::borrow::Cow;

pub enum Contents {
    Text(String),
    Image {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
}
pub trait Source {
    fn read(&mut self) -> Result<Contents, String>;
}
pub struct Desktop;
impl Source for Desktop {
    fn read(&mut self) -> Result<Contents, String> {
        let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
        if let Ok(image) = clipboard.get_image() {
            return Ok(Contents::Image {
                width: image.width,
                height: image.height,
                rgba: image.bytes.into_owned(),
            });
        }
        clipboard
            .get_text()
            .map(Contents::Text)
            .map_err(|error| format!("Clipboard has no text or image: {error}"))
    }
}
pub fn png(width: usize, height: usize, rgba: Vec<u8>) -> Result<Vec<u8>, String> {
    let width = u32::try_from(width).map_err(|_| "Clipboard image is too wide")?;
    let height = u32::try_from(height).map_err(|_| "Clipboard image is too tall")?;
    if width == 0 || height == 0 {
        return Err("Clipboard image is empty".into());
    }
    let image = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or("Clipboard image dimensions do not match its pixels")?;
    let mut output = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(output.into_inner())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_preserves_pixels_and_rejects_invalid_dimensions() {
        let bytes = png(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 128]).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap().into_rgba8();
        assert_eq!(decoded.into_raw(), vec![255, 0, 0, 255, 0, 255, 0, 128]);
        assert!(png(2, 2, vec![0; 4]).is_err());
        assert!(png(0, 1, vec![]).is_err());
    }
    #[test]
    #[ignore = "requires a desktop display; run under Xvfb"]
    fn desktop_image_and_text_roundtrip() {
        let mut owner = arboard::Clipboard::new().unwrap();
        owner
            .set_image(arboard::ImageData {
                width: 2,
                height: 1,
                bytes: Cow::Owned(vec![255, 0, 0, 255, 0, 255, 0, 255]),
            })
            .unwrap();
        let Contents::Image {
            width,
            height,
            rgba,
        } = Desktop.read().unwrap()
        else {
            panic!("expected image")
        };
        assert_eq!(
            image::load_from_memory(&png(width, height, rgba).unwrap())
                .unwrap()
                .into_rgba8()
                .into_raw(),
            vec![255, 0, 0, 255, 0, 255, 0, 255]
        );
        if std::env::var_os("MISA_WAYLAND_CLIPBOARD_TEST").is_some() {
            assert!(std::env::var_os("DISPLAY").is_none());
            let png = std::process::Command::new("wl-paste")
                .args(["--type", "image/png"])
                .output()
                .unwrap();
            assert!(
                png.status.success(),
                "{}",
                String::from_utf8_lossy(&png.stderr)
            );
            assert_eq!(
                image::load_from_memory(&png.stdout)
                    .unwrap()
                    .into_rgba8()
                    .into_raw(),
                vec![255, 0, 0, 255, 0, 255, 0, 255]
            );
        }
        owner.set_text("clipboard text").unwrap();
        assert!(matches!(Desktop.read().unwrap(),Contents::Text(text) if text=="clipboard text"));
        if std::env::var_os("MISA_WAYLAND_CLIPBOARD_TEST").is_some() {
            let text = std::process::Command::new("wl-paste")
                .args(["--no-newline", "--type", "text"])
                .output()
                .unwrap();
            assert!(text.status.success());
            assert_eq!(text.stdout, b"clipboard text");
        }
    }
}
