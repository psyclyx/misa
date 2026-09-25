//! ANSI style encoding shared by the production renderer and the fixture runner.
pub fn sgr(style: &misa_style::Style) -> String {
    use misa_style::Color;
    let mut codes: Vec<String> = Vec::new();
    match style.fg {
        Color::Default => {}
        Color::Indexed(index) => codes.push(format!("38;5;{index}")),
        Color::Rgb(r, g, b) => codes.push(format!("38;2;{r};{g};{b}")),
    }
    match style.bg {
        Color::Default => {}
        Color::Indexed(index) => codes.push(format!("48;5;{index}")),
        Color::Rgb(r, g, b) => codes.push(format!("48;2;{r};{g};{b}")),
    }
    if style.bold {
        codes.push("1".into());
    }
    if style.dim {
        codes.push("2".into());
    }
    if style.italic {
        codes.push("3".into());
    }
    if style.underline {
        codes.push("4".into());
    }
    if style.strikethrough {
        codes.push("9".into());
    }
    if codes.is_empty() {
        String::new()
    } else {
        format!("\u{1b}[{}m", codes.join(";"))
    }
}
