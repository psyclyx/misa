//! OSC 52 clipboard output, kept with physical terminal byte encoding.
use base64::Engine as _;
use std::io::{self, Write};

pub fn write(writer: &mut impl Write, text: &str) -> io::Result<()> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    write!(writer, "\x1b]52;c;{encoded}\x07")?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    #[test]
    fn copies_utf8_as_osc52_and_flushes() {
        struct Writer {
            bytes: Vec<u8>,
            flushed: bool,
        }
        impl std::io::Write for Writer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                self.flushed = true;
                Ok(())
            }
        }
        let mut writer = Writer {
            bytes: Vec::new(),
            flushed: false,
        };
        super::write(&mut writer, "é🙂").unwrap();
        assert_eq!(writer.bytes, b"\x1b]52;c;w6nwn5mC\x07");
        assert!(writer.flushed);
    }
}
