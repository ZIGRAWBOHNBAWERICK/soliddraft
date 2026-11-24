use super::{Mode, Tokens, clear_bench};
use egui::{Color32, Rgba};
use image::{ImageFormat, ImageReader, RgbaImage};
use serde::{Deserialize, Serialize};
use std::{io::Cursor, sync::Arc};

pub const THEME_LIMIT: usize = 64 * 1024;
pub const PICTURE_LIMIT: usize = 16 * 1024 * 1024;
const STORED_PICTURE_LIMIT: usize = 768 * 1024;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltIn {
    #[default]
    ClearBench,
    ClearBenchDark,
    GruvboxLight,
    GruvboxDark,
    HighContrast,
}

impl BuiltIn {
    pub const ALL: [Self; 5] =
        [Self::ClearBench, Self::ClearBenchDark, Self::GruvboxLight, Self::GruvboxDark, Self::HighContrast];

    pub fn for_mode(mode: Mode) -> Self {
        match mode {
            Mode::Light => Self::ClearBench,
            Mode::Dark => Self::ClearBenchDark,
        }
    }

    pub fn tokens(self) -> Tokens {
        use BuiltIn::*;
        let (id, label, dark, colors, background) = match self {
            ClearBench => return clear_bench::TOKENS,
            ClearBenchDark => (
                "clear-bench-dark",
                "Clear Bench Dark",
                true,
                [0x171c25, 0x222b39, 0xeaf0f9, 0x91baff],
                [0x343f53, 0x171d29],
            ),
            GruvboxLight => (
                "gruvbox-light",
                "Gruvbox Light",
                false,
                [0xebdbb2, 0xfbf1c7, 0x3c3836, 0x00665d],
                [0xfff5d4, 0xd5c4a1],
            ),
            GruvboxDark => {
                ("gruvbox-dark", "Gruvbox Dark", true, [0x1d2021, 0x282828, 0xebdbb2, 0x8ec07c], [0x504945, 0x282828])
            }
            HighContrast => {
                ("high-contrast", "High Contrast", true, [0x000000, 0x090909, 0xffffff, 0xffe000], [0x242424, 0x0b0b0b])
            }
        };
        let [chrome, panel, ink, accent] = colors.map(hex);
        let [viewport_top, viewport_bottom] = background.map(hex);
        let mut tokens = Tokens { id, label, is_dark: dark, ..clear_bench::TOKENS };
        restyle(&mut tokens, chrome, panel, ink, accent);
        let palette = &mut tokens.palette;
        palette.viewport_top = viewport_top;
        palette.viewport_bottom = viewport_bottom;
        if self == HighContrast {
            palette.line = hex(0x8c8c8c);
            palette.line_strong = Color32::WHITE;
        }
        tokens
    }
}

fn hex(value: u32) -> Color32 {
    Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

fn restyle(tokens: &mut Tokens, chrome: Color32, panel: Color32, ink: Color32, accent: Color32) {
    let p = &mut tokens.palette;
    let mix = |color, fraction| panel.lerp_to_gamma(color, fraction);
    p.chrome = chrome;
    p.panel = panel;
    p.ink = ink;
    p.accent = accent;
    p.ribbon = mix(chrome, 0.38);
    p.panel_header = mix(ink, 0.06);
    p.line = mix(ink, 0.25);
    p.line_faint = mix(ink, 0.12);
    p.line_strong = mix(ink, 0.47);
    p.muted = mix(ink, 0.76);
    p.accent_hover = accent.lerp_to_gamma(ink, 0.18);
    p.accent_soft = mix(accent, 0.16);
    p.selection_bg = p.accent_soft;
    p.selection_ink = accent;
    p.hover = mix(ink, 0.13);
    p.on_accent = contrasting_ink(accent);
    p.form_bg = mix(accent, 0.09);
    p.field_bg = mix(chrome, 0.35);
    p.chip = mix(ink, 0.09);
    p.hud_bg = panel;
    p.command_line = p.ribbon;
    p.command_prompt = accent;
    p.canvas_bg = chrome;
    if tokens.is_dark {
        p.ok = hex(0x8de6ad);
        p.warn = hex(0xffd078);
        p.error = hex(0xffa0a0);
        p.danger = p.error;
        p.on_danger = Color32::BLACK;
        p.donate_ink = p.error;
        p.donate_bg = mix(p.error, 0.12);
        p.donate_line = mix(p.error, 0.38);
        p.ok_soft = mix(p.ok, 0.14);
        p.warn_soft = mix(p.warn, 0.14);
        p.error_soft = mix(p.error, 0.14);
        p.reference_plane = hex(0xffbc6b);
    }
}

pub(super) fn contrasting_ink(color: Color32) -> Color32 {
    if luminance(color) > 0.179 { Color32::BLACK } else { Color32::WHITE }
}

pub fn luminance(color: Color32) -> f32 {
    let c = Rgba::from(color);
    0.2126 * c.r() + 0.7152 * c.g() + 0.0722 * c.b()
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Rgb(pub [u8; 3]);

impl From<Rgb> for Color32 {
    fn from(Rgb([r, g, b]): Rgb) -> Self {
        Self::from_rgb(r, g, b)
    }
}

impl From<Color32> for Rgb {
    fn from(c: Color32) -> Self {
        Self([c.r(), c.g(), c.b()])
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeFile {
    format: ThemeFileVersion,
    pub name: String,
    #[serde(default)]
    pub base: BuiltIn,
    pub chrome: Option<Rgb>,
    pub panel: Option<Rgb>,
    pub ink: Option<Rgb>,
    pub accent: Option<Rgb>,
    pub viewport_top: Option<Rgb>,
    pub viewport_bottom: Option<Rgb>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
enum ThemeFileVersion {
    #[serde(rename = "soliddraft-theme/1")]
    V1,
}

impl ThemeFile {
    pub fn read(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > THEME_LIMIT {
            return Err("Theme files must be 64 KB or smaller.".into());
        }
        let mut theme: Self = serde_json::from_slice(bytes).map_err(|e| format!("Invalid theme file: {e}"))?;
        theme.name = theme.name.trim().to_owned();
        theme.validate()?;
        Ok(theme)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() || self.name.chars().count() > 52 || self.name.chars().any(char::is_control) {
            return Err("Use a theme name of 1 to 52 characters without control characters.".into());
        }
        let p = self.tokens().palette;
        let contrast = |a, b| {
            let (a, b) = (luminance(a), luminance(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        if [p.panel, p.chrome, p.ribbon].into_iter().any(|surface| contrast(p.ink, surface) < 4.5)
            || contrast(p.accent, p.panel) < 3.0
        {
            return Err("Increase the contrast of ink and accent against the theme backgrounds.".into());
        }
        Ok(())
    }

    pub fn tokens(&self) -> Tokens {
        let mut tokens = self.base.tokens();
        tokens.id = "imported";
        tokens.label = "Imported theme";
        let p = tokens.palette;
        if [self.chrome, self.panel, self.ink, self.accent].iter().any(Option::is_some) {
            restyle(
                &mut tokens,
                self.chrome.map_or(p.chrome, Into::into),
                self.panel.map_or(p.panel, Into::into),
                self.ink.map_or(p.ink, Into::into),
                self.accent.map_or(p.accent, Into::into),
            );
        }
        tokens.palette.viewport_top = self.viewport_top.map_or(p.viewport_top, Into::into);
        tokens.palette.viewport_bottom = self.viewport_bottom.map_or(p.viewport_bottom, Into::into);
        tokens
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Selection {
    BuiltIn(BuiltIn),
    System,
    Imported,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Background {
    #[default]
    Gradient,
    Color,
    Picture,
    Scene,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ViewColors {
    pub grid: Option<Rgb>,
    pub references: Option<Rgb>,
    pub selection: Option<Rgb>,
    pub edges: Option<Rgb>,
    pub logo: Option<Rgb>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
enum Format {
    #[default]
    #[serde(rename = "soliddraft-appearance/1")]
    V1,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Appearance {
    format: Format,
    pub selected: Selection,
    pub imported: Option<ThemeFile>,
    pub background: Background,
    pub background_color: Rgb,
    picture: Option<Arc<Picture>>,
    pub colors: ViewColors,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            format: Format::V1,
            selected: Selection::BuiltIn(BuiltIn::ClearBench),
            imported: None,
            background: Background::Gradient,
            background_color: Rgb([232, 238, 246]),
            picture: None,
            colors: ViewColors::default(),
        }
    }
}

impl Appearance {
    pub fn tokens(&self, system: Option<egui::Theme>) -> Tokens {
        match self.selected {
            Selection::BuiltIn(theme) => theme.tokens(),
            Selection::System if system == Some(egui::Theme::Dark) => BuiltIn::ClearBenchDark.tokens(),
            Selection::Imported => {
                self.imported.as_ref().map_or_else(|| BuiltIn::ClearBench.tokens(), ThemeFile::tokens)
            }
            Selection::System => BuiltIn::ClearBench.tokens(),
        }
    }

    pub fn restore_resources(&mut self) {
        if self.imported.as_ref().is_some_and(|theme| theme.validate().is_err()) {
            self.imported = None;
        }
        if self.selected == Selection::Imported && self.imported.is_none() {
            self.selected = Self::default().selected;
        }
        if let Some(picture) = &mut self.picture {
            let picture = Arc::make_mut(picture);
            picture.image = parse_png_hex(&picture.png).and_then(|bytes| decode_png(&bytes, 1280).ok()).map(Arc::new);
            if let Some(image) = &picture.image {
                picture.dark = dark_picture(image);
            } else {
                self.picture = None;
            }
        }
        if self.background == Background::Picture && self.picture.is_none() {
            self.background = Background::Gradient;
        }
    }

    pub fn set_picture(&mut self, name: String, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > PICTURE_LIMIT {
            return Err("Choose a PNG picture of 16 MB or smaller.".into());
        }
        let mut image = decode_png(bytes, 8192)?;
        let max = image.width().max(image.height());
        if max > 1280 {
            image = resize(&image, 1280, max);
        }
        let png = loop {
            let mut output = Cursor::new(Vec::new());
            image.write_to(&mut output, ImageFormat::Png).map_err(|e| format!("Cannot store the picture: {e}"))?;
            if output.get_ref().len() <= STORED_PICTURE_LIMIT {
                break output.into_inner();
            }
            let max = image.width().max(image.height());
            image = resize(&image, max * 3 / 4, max);
        };
        let dark = dark_picture(&image);
        self.picture = Some(Arc::new(Picture { name, png: png_hex(&png), dark, image: Some(Arc::new(image)) }));
        self.background = Background::Picture;
        Ok(())
    }

    pub fn picture_name(&self) -> Option<&str> {
        self.picture.as_ref().map(|p| p.name.as_str())
    }

    pub fn picture(&self) -> Option<Arc<RgbaImage>> {
        self.picture.as_ref().and_then(|p| p.image.clone())
    }

    pub fn picture_is_dark(&self) -> bool {
        self.picture.as_ref().is_some_and(|p| p.dark)
    }

    pub fn clear_picture(&mut self) {
        self.picture = None;
        if self.background == Background::Picture {
            self.background = Background::Gradient;
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Picture {
    name: String,
    png: String,
    #[serde(skip)]
    dark: bool,
    #[serde(skip)]
    image: Option<Arc<RgbaImage>>,
}

impl PartialEq for Picture {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.png == other.png
    }
}

// A string keeps a picture within the browser settings quota even with pretty JSON.
fn png_hex(png: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(2 * png.len());
    for &byte in png {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn parse_png_hex(encoded: &str) -> Option<Vec<u8>> {
    if encoded.len() > 2 * STORED_PICTURE_LIMIT || !encoded.len().is_multiple_of(2) {
        return None;
    }
    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let hi = char::from(pair[0]).to_digit(16)?;
            let lo = char::from(pair[1]).to_digit(16)?;
            Some(((hi << 4) | lo) as u8)
        })
        .collect()
}

fn decode_png(bytes: &[u8], max_dimension: u32) -> Result<RgbaImage, String> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_dimension);
    limits.max_image_height = Some(max_dimension);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map(|image| image.into_rgba8()).map_err(|e| format!("Cannot open the PNG picture: {e}"))
}

fn resize(image: &RgbaImage, length: u32, longest: u32) -> RgbaImage {
    let width = (image.width() * length / longest).max(1);
    let height = (image.height() * length / longest).max(1);
    image::imageops::resize(image, width, height, image::imageops::FilterType::Triangle)
}

fn dark_picture(image: &RgbaImage) -> bool {
    let (mut light, mut alpha) = (0.0, 0.0);
    for pixel in image.pixels().step_by(31) {
        let [r, g, b, a] = pixel.0;
        let weight = f32::from(a) / 255.0;
        light += luminance(Color32::from_rgb(r, g, b)) * weight;
        alpha += weight;
    }
    alpha > 0.0 && light / alpha < 0.25
}
