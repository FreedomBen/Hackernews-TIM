use config_parser2::*;
use cursive::theme::BaseColor;
use serde::{de, Deserialize, Deserializer};

/// The HN default top-bar color (#ff6600). Any `component_style` field whose
/// background currently matches this value is treated as an "HN orange accent"
/// and will be re-pointed at the user's `topcolor` when the override fires.
pub const HN_DEFAULT_TOPCOLOR_HEX: &str = "ff6600";

#[derive(Default, Clone, Copy, Debug, Deserialize, ConfigParse)]
/// Application's theme, consists of two main parts:
/// - a terminal color palette - `palette`
/// - additional component styles - `component_style`
pub struct Theme {
    pub palette: Palette,
    pub component_style: ComponentStyle,
}

impl Theme {
    /// Apply the user's HN `topcolor` (a 6-char hex value like `ff6600`) to
    /// the theme. `title_bar.back` is always overridden; other component
    /// styles are overridden only when their current `back` still matches
    /// the HN default orange, so user-customised colours are preserved.
    /// Returns `true` when at least one field was changed.
    pub fn apply_hn_topcolor(&mut self, hex: &str) -> bool {
        let hex = hex.trim().trim_start_matches('#');
        let color_str = format!("#{hex}");
        let Some(topcolor) = Color::try_parse(&color_str) else {
            return false;
        };
        let default_orange = Color::parse(&format!("#{HN_DEFAULT_TOPCOLOR_HEX}"));

        let cs = &mut self.component_style;
        let mut changed = false;

        // title_bar is the primary "topcolor" surface — always override.
        if cs.title_bar.back != Some(topcolor) {
            cs.title_bar.back = Some(topcolor);
            changed = true;
        }

        // Other orange accents: only override if they still match the HN default.
        for style in [
            &mut cs.link_id,
            &mut cs.matched_highlight,
            &mut cs.single_code_block,
            &mut cs.multiline_code_block,
            &mut cs.header,
            &mut cs.current_story_tag,
            &mut cs.loading_bar,
            &mut cs.ask_hn,
            &mut cs.tell_hn,
            &mut cs.show_hn,
            &mut cs.launch_hn,
        ] {
            if style.back == Some(default_orange) {
                style.back = Some(topcolor);
                changed = true;
            }
        }

        changed
    }
}

#[derive(Clone, Copy, Debug, Deserialize, ConfigParse)]
/// Terminal color palette.
///
/// This struct defines colors for application's background/foreground,
/// selection text's background/foreground, and 16 ANSI colors.
///
/// The struct structure is compatible with the terminal color schemes as
/// listed in https://github.com/mbadolato/iTerm2-Color-Schemes.
pub struct Palette {
    pub background: Color,
    pub foreground: Color,
    pub selection_background: Color,
    pub selection_foreground: Color,

    pub black: Color,
    pub blue: Color,
    pub cyan: Color,
    pub green: Color,
    pub magenta: Color,
    pub red: Color,
    pub white: Color,
    pub yellow: Color,

    pub light_black: Color,
    pub light_white: Color,
    pub light_red: Color,
    pub light_magenta: Color,
    pub light_green: Color,
    pub light_cyan: Color,
    pub light_blue: Color,
    pub light_yellow: Color,
}

#[derive(Clone, Copy, Debug, Deserialize, ConfigParse)]
/// Additional colors/styles for specific components of the application.
pub struct ComponentStyle {
    pub title_bar: Style,
    pub link: Style,
    pub link_id: Style,
    pub matched_highlight: Style,
    pub single_code_block: Style,
    pub multiline_code_block: Style,
    pub header: Style,
    pub quote: Style,
    pub italic: Style,
    pub bold: Style,
    pub metadata: Style,
    pub current_story_tag: Style,
    pub username: Style,
    pub loading_bar: Style,
    pub ask_hn: Style,
    pub tell_hn: Style,
    pub show_hn: Style,
    pub launch_hn: Style,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            background: Color::parse("#f6f6ef"),
            foreground: Color::parse("#242424"),
            selection_background: Color::parse("#d8dad6"),
            selection_foreground: Color::parse("#4a4c4c"),

            black: Color::parse("#000000"),
            blue: Color::parse("#0000aa"),
            cyan: Color::parse("#00aaaa"),
            green: Color::parse("#00aa00"),
            magenta: Color::parse("#aa00aa"),
            red: Color::parse("#aa0000"),
            white: Color::parse("#aaaaaa"),
            yellow: Color::parse("#aaaa00"),

            light_black: Color::parse("#555555"),
            light_white: Color::parse("#ffffff"),
            light_red: Color::parse("#ff5555"),
            light_magenta: Color::parse("#5555ff"),
            light_green: Color::parse("#55ff55"),
            light_cyan: Color::parse("#55ffff"),
            light_blue: Color::parse("#5555ff"),
            light_yellow: Color::parse("#ffff55"),
        }
    }
}

impl Default for ComponentStyle {
    fn default() -> Self {
        Self {
            title_bar: Style::default()
                .back(Color::parse("#ff6600"))
                .effect(Effect::Bold),
            current_story_tag: Style::default().front(Color::parse("light white")),
            link: Style::default().front(Color::parse("#4fbbfd")),
            link_id: Style::default()
                .front(Color::parse("black"))
                .back(Color::parse("#ffff55")),
            matched_highlight: Style::default()
                .front(Color::parse("black"))
                .back(Color::parse("#ffff55")),
            single_code_block: Style::default()
                .front(Color::parse("black"))
                .back(Color::parse("#c8c8c8")),
            multiline_code_block: Style::default()
                .front(Color::parse("light black"))
                .effect(Effect::Bold),
            header: Style::default()
                .front(Color::parse("black"))
                .effect(Effect::Bold),
            quote: Style::default().front(Color::parse("#677280")),
            italic: Style::default().effect(Effect::Italic),
            bold: Style::default().effect(Effect::Bold),
            metadata: Style::default().front(Color::parse("#828282")),
            username: Style::default().effect(Effect::Bold),
            loading_bar: Style::default()
                .front(Color::parse("light yellow"))
                .back(Color::parse("blue")),
            ask_hn: Style::default()
                .front(Color::parse("red"))
                .effect(Effect::Bold),
            tell_hn: Style::default()
                .front(Color::parse("yellow"))
                .effect(Effect::Bold),
            show_hn: Style::default()
                .front(Color::parse("blue"))
                .effect(Effect::Bold),
            launch_hn: Style::default()
                .front(Color::parse("green"))
                .effect(Effect::Bold),
        }
    }
}

#[derive(Default, Clone, Copy, Debug, Deserialize)]
pub struct Style {
    front: Option<Color>,
    back: Option<Color>,
    effect: Option<Effect>,
}

config_parser_impl!(Style);

impl Style {
    pub fn front(self, c: Color) -> Self {
        Self {
            front: Some(c),
            ..self
        }
    }
    pub fn back(self, c: Color) -> Self {
        Self {
            back: Some(c),
            ..self
        }
    }
    pub fn effect(self, e: Effect) -> Self {
        Self {
            effect: Some(e),
            ..self
        }
    }
}

impl From<Style> for cursive::theme::ColorStyle {
    fn from(c: Style) -> Self {
        match (c.front, c.back) {
            (Some(f), Some(b)) => Self::new(f, b),
            (Some(f), None) => Self::front(f),
            (None, Some(b)) => Self::back(b),
            (None, None) => Self::inherit_parent(),
        }
    }
}

impl From<Style> for cursive::theme::Style {
    fn from(c: Style) -> Self {
        let style = Self::from(cursive::theme::ColorStyle::from(c));
        match c.effect {
            None => style,
            Some(e) => style.combine(cursive::theme::Effect::from(e)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(cursive::theme::Color);

config_parser_impl!(Color);

impl Color {
    pub fn new(c: cursive::theme::Color) -> Self {
        Self(c)
    }

    pub fn try_parse(c: &str) -> Option<Self> {
        cursive::theme::Color::parse(c).map(Color)
    }

    pub fn parse(c: &str) -> Self {
        Self::try_parse(c).unwrap_or_else(|| panic!("failed to parse color: {c}"))
    }
}

impl From<u8> for Color {
    fn from(x: u8) -> Self {
        Self(cursive::theme::Color::from_256colors(x))
    }
}

impl From<Color> for cursive::theme::Color {
    fn from(c: Color) -> Self {
        // converts from application's color to `cursive::theme::color` will
        // require to look into the application's pre-defined color palette.
        //
        // Under the hood, the application's palette colors are stored as a wrapper
        // struct of `cursive::theme::color` (`Color`).
        let palette = &get_config_theme().palette;
        match c.0 {
            Self::Dark(c) => match c {
                BaseColor::Black => palette.black.0,
                BaseColor::Red => palette.red.0,
                BaseColor::Green => palette.green.0,
                BaseColor::Yellow => palette.yellow.0,
                BaseColor::Blue => palette.blue.0,
                BaseColor::Magenta => palette.magenta.0,
                BaseColor::Cyan => palette.cyan.0,
                BaseColor::White => palette.white.0,
            },
            Self::Light(c) => match c {
                BaseColor::Black => palette.light_black.0,
                BaseColor::Red => palette.light_red.0,
                BaseColor::Green => palette.light_green.0,
                BaseColor::Yellow => palette.light_yellow.0,
                BaseColor::Blue => palette.light_blue.0,
                BaseColor::Magenta => palette.light_magenta.0,
                BaseColor::Cyan => palette.light_cyan.0,
                BaseColor::White => palette.light_white.0,
            },
            _ => c.0,
        }
    }
}

impl From<Color> for cursive::theme::ColorType {
    fn from(c: Color) -> Self {
        Self::from(cursive::theme::Color::from(c))
    }
}

impl From<Color> for cursive::theme::Style {
    fn from(c: Color) -> Self {
        Self::from(cursive::theme::Color::from(c))
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match Self::try_parse(&s) {
            None => Err(de::Error::custom(format!("failed to parse color: {s}"))),
            Some(color) => Ok(color),
        }
    }
}

// A copy struct of `cursive::theme::Effect` that
// derives serde::Deserialize
#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    Simple,
    Reverse,
    Bold,
    Italic,
    Strikethrough,
    Underline,
    Blink,
}

impl From<Effect> for cursive::theme::Effect {
    fn from(e: Effect) -> Self {
        match e {
            Effect::Simple => Self::Simple,
            Effect::Reverse => Self::Reverse,
            Effect::Bold => Self::Bold,
            Effect::Italic => Self::Italic,
            Effect::Strikethrough => Self::Strikethrough,
            Effect::Underline => Self::Underline,
            Effect::Blink => Self::Blink,
        }
    }
}

pub fn get_config_theme() -> &'static Theme {
    &super::get_config().theme
}
