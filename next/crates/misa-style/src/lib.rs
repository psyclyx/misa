//! Medium-independent colour and style values shared by renderers and clients.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Color {
    /// Whatever the medium's default is.
    Default,
    /// A terminal palette entry, for a theme that wants to follow the terminal.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style::PLAIN
    }
}

impl Style {
    pub const PLAIN: Style = Style {
        fg: Color::Default,
        bg: Color::Default,
        bold: false,
        dim: false,
        italic: false,
        underline: false,
        strikethrough: false,
    };

    pub fn fg(color: Color) -> Style {
        Style {
            fg: color,
            ..Style::PLAIN
        }
    }

    pub fn rgb(r: u8, g: u8, b: u8) -> Style {
        Style::fg(Color::Rgb(r, g, b))
    }

    pub fn bold(mut self) -> Style {
        self.bold = true;
        self
    }

    pub fn dim(mut self) -> Style {
        self.dim = true;
        self
    }

    pub fn italic(mut self) -> Style {
        self.italic = true;
        self
    }

    pub fn underline(mut self) -> Style {
        self.underline = true;
        self
    }

    pub fn on(mut self, color: Color) -> Style {
        self.bg = color;
        self
    }

    /// Merge: the other style's non-default fields win.
    pub fn over(self, other: Style) -> Style {
        Style {
            fg: if other.fg == Color::Default {
                self.fg
            } else {
                other.fg
            },
            bg: if other.bg == Color::Default {
                self.bg
            } else {
                other.bg
            },
            bold: self.bold || other.bold,
            dim: self.dim || other.dim,
            italic: self.italic || other.italic,
            underline: self.underline || other.underline,
            strikethrough: self.strikethrough || other.strikethrough,
        }
    }
}

/// An attribute patch over a resolved style.
///
/// [`Style::over`] is a merge, so it can add an attribute but not remove one. A
/// theme override has to be able to say `bold: false`, which is what the previous
/// system's `config.styles` did, so this carries `Option`s: a field that is set
/// replaces the base's field whatever its value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StylePatch {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: Option<bool>,
    pub dim: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
}

impl StylePatch {
    pub fn apply(self, base: Style) -> Style {
        Style {
            fg: self.fg.unwrap_or(base.fg),
            bg: self.bg.unwrap_or(base.bg),
            bold: self.bold.unwrap_or(base.bold),
            dim: self.dim.unwrap_or(base.dim),
            italic: self.italic.unwrap_or(base.italic),
            underline: self.underline.unwrap_or(base.underline),
            strikethrough: self.strikethrough.unwrap_or(base.strikethrough),
        }
    }
}
