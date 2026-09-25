//! Native appearance is local to the window/profile, never an owner query.
use misa_style::{Color, Style};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Choice {
    #[default]
    System,
    Dark,
    Light,
}
impl Choice {
    pub fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            _ => None,
        }
    }
    pub fn light(self, system_light: bool) -> bool {
        match self {
            Self::System => system_light,
            Self::Dark => false,
            Self::Light => true,
        }
    }
}
#[derive(Clone, Copy)]
pub struct Palette {
    pub background: Color,
    pub surface: Style,
    pub text: Style,
    pub muted: Style,
    pub selection: Style,
    pub accent: Style,
    pub border: Style,
    pub field: Style,
    pub meter: Style,
    pub button: Style,
    pub selected_button: Style,
}
impl Palette {
    pub fn new(light: bool) -> Self {
        let style = |r, g, b| Style {
            fg: Color::Rgb(r, g, b),
            ..Style::PLAIN
        };
        if light {
            Self {
                background: Color::Rgb(248, 249, 252),
                surface: style(231, 235, 241),
                text: style(28, 32, 40),
                muted: style(83, 91, 105),
                selection: style(192, 218, 247),
                accent: style(28, 99, 162),
                border: style(148, 160, 178),
                field: style(255, 255, 255),
                meter: style(214, 221, 232),
                button: style(238, 242, 248),
                selected_button: style(210, 225, 244),
            }
        } else {
            Self {
                background: Color::Rgb(20, 22, 26),
                surface: style(35, 40, 48),
                text: style(230, 232, 236),
                muted: style(180, 180, 190),
                selection: style(55, 86, 120),
                accent: style(90, 170, 240),
                border: style(66, 74, 86),
                field: style(27, 31, 38),
                meter: style(45, 52, 61),
                button: style(28, 33, 40),
                selected_button: style(45, 53, 63),
            }
        }
    }
}
