//! Shared theme tokens. Classic keeps the Studio light and dark palettes. The
//! internal Edit Suite frame uses the approved charcoal and amber [`edit_suite`]
//! tokens in either appearance mode. Viewport scenes remain a separate choice.
//!
//! The mode is process-wide: [`set_mode`] switches it and restyles the context, and the
//! accessors ([`ink`], [`panel`], ...) read the active palette each call, so a widget
//! painted on the next frame is already in the new mode.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Vec2};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::settings::FrameLayout;

/// Which column of the palette the interface reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Light,
    Dark,
}

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::Light, Mode::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Light => "Light",
            Mode::Dark => "Dark",
        }
    }

    /// Parses a mode name as typed on the command line or in a page address.
    pub fn parse(name: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.label().eq_ignore_ascii_case(name.trim()))
    }

    pub fn palette(self) -> &'static Palette {
        match self {
            Mode::Light => &LIGHT,
            Mode::Dark => &DARK,
        }
    }

    pub fn is_dark(self) -> bool {
        self == Mode::Dark
    }
}

/// Shared semantic colours, read by egui and custom painters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Window chrome: the menu bar, the ribbon tab strip, the status bar and the gaps
    /// between panels.
    pub chrome: Color32,
    /// Panels: the tree, properties, the ribbon body, dialogs and menus.
    pub panel: Color32,
    /// Panel headers, the dock tab bar and plain buttons.
    pub panel_header: Color32,
    /// One-pixel separators and widget borders.
    pub line: Color32,
    /// A stronger border, for a hovered widget.
    pub line_strong: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub accent: Color32,
    /// The accent as a fill: the active ribbon item, the selected tree row.
    pub accent_soft: Color32,
    pub selection_bg: Color32,
    pub selection_ink: Color32,
    pub hover: Color32,
    pub ok: Color32,
    pub bounded: Color32,
    pub warn: Color32,
    pub error: Color32,
    /// Notice fills behind warn and error text.
    pub warn_soft: Color32,
    pub error_soft: Color32,
    /// Text fields and value pills.
    pub field_bg: Color32,
    /// Tool chips and raised buttons: the brightest surface in light, a step above the
    /// panel in dark.
    pub chip: Color32,
    /// Text and icons on an accent-filled button.
    pub on_accent: Color32,
    /// Floating pills and tool bars over the viewport.
    pub hud_bg: Color32,
    /// The command line along the bottom and its prompt glyph.
    pub command_line: Color32,
    pub command_prompt: Color32,
    /// The AI session: a dark console in either mode.
    pub terminal_bg: Color32,
    pub terminal_ink: Color32,
    pub terminal_accent: Color32,
    /// The modelling space, a vertical fade from `viewport_top` to `viewport_bottom`.
    /// The ground shadow lands on the bottom tone.
    pub viewport_top: Color32,
    pub viewport_bottom: Color32,
    /// The drawing canvas around the sheet; the sheet itself stays paper white.
    pub canvas_bg: Color32,
    /// The origin axes.
    pub axis_x: Color32,
    pub axis_y: Color32,
    pub axis_z: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied_const(r, g, b, a)
}

pub const LIGHT: Palette = Palette {
    chrome: rgb(0xe9, 0xec, 0xf0),
    panel: rgb(0xf7, 0xf8, 0xfa),
    panel_header: rgb(0xee, 0xf0, 0xf4),
    line: rgb(0x73, 0x7f, 0x90),
    line_strong: rgb(0x58, 0x67, 0x7b),
    ink: rgb(0x0c, 0x14, 0x20),
    muted: rgb(0x46, 0x52, 0x64),
    accent: rgb(0x1d, 0x4e, 0xd8),
    accent_soft: rgb(0xe3, 0xec, 0xfd),
    selection_bg: rgb(0xd4, 0xe2, 0xfc),
    selection_ink: rgb(0x0e, 0x2a, 0x66),
    hover: rgb(0xe2, 0xe6, 0xed),
    ok: rgb(0x17, 0x6b, 0x39),
    bounded: rgb(0x76, 0x35, 0xa8),
    warn: rgb(0x89, 0x50, 0x06),
    error: rgb(0xb5, 0x2b, 0x32),
    warn_soft: rgb(0xfd, 0xf6, 0xe3),
    error_soft: rgb(0xfd, 0xee, 0xe8),
    field_bg: rgb(0xee, 0xf0, 0xf4),
    chip: Color32::WHITE,
    on_accent: Color32::WHITE,
    hud_bg: rgb(0xf7, 0xf8, 0xfa),
    command_line: rgb(0xf7, 0xf8, 0xfa),
    command_prompt: rgb(0x1d, 0x4e, 0xd8),
    terminal_bg: rgb(0x14, 0x18, 0x1f),
    terminal_ink: rgb(0xe6, 0xe9, 0xee),
    terminal_accent: rgb(0x7a, 0xa2, 0xff),
    viewport_top: rgb(0xf2, 0xf4, 0xf7),
    viewport_bottom: rgb(0xd8, 0xdf, 0xe8),
    canvas_bg: rgb(0xdd, 0xe2, 0xe9),
    axis_x: rgb(0xb5, 0x2b, 0x32),
    axis_y: rgb(0x17, 0x6b, 0x39),
    axis_z: rgb(0x1d, 0x4e, 0xd8),
};

pub const DARK: Palette = Palette {
    chrome: rgb(0x2a, 0x2e, 0x35),
    panel: rgb(0x31, 0x36, 0x3e),
    panel_header: rgb(0x2c, 0x31, 0x38),
    line: rgb(0x3e, 0x44, 0x4d),
    line_strong: rgb(0x52, 0x59, 0x64),
    ink: rgb(0xe4, 0xe7, 0xec),
    muted: rgb(0x98, 0xa1, 0xae),
    accent: rgb(0x6f, 0xa0, 0xff),
    accent_soft: rgba(0x6f, 0xa0, 0xff, 41),
    selection_bg: rgba(0x6f, 0xa0, 0xff, 61),
    selection_ink: rgb(0xcf, 0xe0, 0xff),
    hover: rgb(0x3a, 0x40, 0x49),
    ok: rgb(0x5b, 0xd2, 0x7a),
    bounded: rgb(0xd2, 0xa0, 0xff),
    warn: rgb(0xe3, 0xa0, 0x3c),
    error: rgb(0xff, 0x6b, 0x6b),
    warn_soft: rgba(0xe3, 0xa0, 0x3c, 46),
    error_soft: rgba(0xff, 0x6b, 0x6b, 46),
    field_bg: rgb(0x2c, 0x31, 0x38),
    chip: rgb(0x2e, 0x33, 0x3b),
    on_accent: rgb(0x0f, 0x12, 0x16),
    hud_bg: rgba(0x28, 0x2c, 0x34, 235),
    command_line: rgb(0x23, 0x27, 0x2e),
    command_prompt: rgb(0x6f, 0xa0, 0xff),
    terminal_bg: rgb(0x14, 0x18, 0x1f),
    terminal_ink: rgb(0xe6, 0xe9, 0xee),
    terminal_accent: rgb(0x7a, 0xa2, 0xff),
    viewport_top: rgb(0x4a, 0x50, 0x5a),
    viewport_bottom: rgb(0x22, 0x26, 0x2c),
    canvas_bg: rgb(0x1e, 0x22, 0x27),
    axis_x: rgb(0xff, 0x6b, 0x6b),
    axis_y: rgb(0x5b, 0xd2, 0x7a),
    axis_z: rgb(0x6f, 0xa8, 0xff),
};

/// Values from the approved L00 merged prototype, in logical pixels.
pub mod edit_suite {
    use super::{Palette, rgb, rgba};
    use egui::Color32;

    pub const BAR: Color32 = rgb(0x22, 0x24, 0x27);
    pub const CHIP_LINE: Color32 = rgb(0x4a, 0x4e, 0x55);
    pub const POPUP: Color32 = rgb(0x35, 0x38, 0x3e);
    pub const POPUP_HOVER: Color32 = rgba(255, 255, 255, 26);
    pub const PRIMARY_HOVER: Color32 = rgb(0xff, 0xbc, 0x4d);
    pub const DANGER: Color32 = rgb(0xe0, 0x3a, 0x2e);
    pub const ON_DANGER: Color32 = Color32::WHITE;
    pub const UNDER_DEFINED: Color32 = rgb(0x1d, 0x4f, 0xe0);
    pub const FULLY_DEFINED: Color32 = rgb(0x0b, 0x0d, 0x11);
    pub const SKETCH_SHEET: Color32 = rgb(0xf1, 0xf3, 0xf7);

    pub const PALETTE: Palette = Palette {
        chrome: rgb(0x2a, 0x2c, 0x30),
        panel: rgb(0x30, 0x33, 0x38),
        panel_header: BAR,
        line: rgb(0x19, 0x1a, 0x1d),
        line_strong: rgb(0x47, 0x4b, 0x52),
        ink: rgb(0xe6, 0xe8, 0xec),
        muted: rgb(0x9d, 0xa2, 0xab),
        accent: rgb(0xf5, 0xa5, 0x24),
        accent_soft: rgba(245, 165, 36, 51),
        selection_bg: rgba(245, 165, 36, 51),
        selection_ink: rgb(0xf5, 0xa5, 0x24),
        hover: POPUP_HOVER,
        ok: rgb(0x63, 0xd6, 0x8f),
        warn: rgb(0xf5, 0xa5, 0x24),
        error: rgb(0xff, 0xb4, 0xad),
        warn_soft: rgba(245, 165, 36, 51),
        error_soft: rgba(224, 58, 46, 46),
        field_bg: rgb(0x3a, 0x3d, 0x43),
        chip: rgb(0x3a, 0x3d, 0x43),
        on_accent: rgb(0x23, 0x16, 0x00),
        hud_bg: rgba(42, 44, 48, 242),
        command_line: rgb(0x2a, 0x2c, 0x30),
        command_prompt: rgb(0xf5, 0xa5, 0x24),
        terminal_bg: rgb(0x17, 0x18, 0x1b),
        terminal_ink: rgb(0xe6, 0xe9, 0xee),
        terminal_accent: rgb(0xf5, 0xa5, 0x24),
        // A scene supplies the backdrop in Edit Suite; White is the fallback.
        viewport_top: Color32::WHITE,
        viewport_bottom: Color32::WHITE,
        canvas_bg: rgb(0x56, 0x5a, 0x62),
        axis_x: rgb(0xd2, 0x3b, 0x3b),
        axis_y: rgb(0x2e, 0x9e, 0x5b),
        axis_z: rgb(0x1f, 0x5f, 0xd0),
    };

    pub const TITLE_HEIGHT: f32 = 34.0;
    pub const OPTIONS_HEIGHT: f32 = 46.0;
    pub const PALETTE_WIDTH: f32 = 250.0;
    pub const PANE_WIDTH: f32 = 236.0;
    pub const TIMELINE_HEIGHT: f32 = 58.0;
    pub const COMMAND_LINE_HEIGHT: f32 = 30.0;
    pub const TOOL_HEIGHT: f32 = 23.0;
    pub const ROW_HEIGHT: f32 = 25.0;
    pub const CONTROL_HEIGHT: f32 = 28.0;
    pub const ICON_SIZE: f32 = 16.0;
    pub const COLUMN_GAP: f32 = 2.0;
    pub const TOOL_PADDING: f32 = 5.0;
    pub const GROUP_GAP: f32 = 12.0;
    pub const BODY_SIZE: f32 = 12.5;
    pub const TOOL_LABEL_SIZE: f32 = 11.5;
    pub const SMALL_SIZE: f32 = 10.5;
    pub const GROUP_LABEL_SIZE: f32 = 9.0;
    pub const VALUE_SIZE: f32 = 12.0;
    pub const RADIUS: u8 = 6;
    pub const FIELD_RADIUS: u8 = 4;
    pub const TOOL_RADIUS: u8 = 5;
    pub const FLOATING_RADIUS: u8 = 10;
}

static MODE: AtomicU8 = AtomicU8::new(0);
static LAYOUT: AtomicU8 = AtomicU8::new(0);

fn is_edit_suite() -> bool {
    LAYOUT.load(Ordering::Relaxed) == 1
}

/// Whether the active frame has dark chrome, independent of the saved mode.
pub fn dark_chrome() -> bool {
    is_edit_suite() || mode().is_dark()
}

/// The mode the interface is in.
pub fn mode() -> Mode {
    match MODE.load(Ordering::Relaxed) {
        1 => Mode::Dark,
        _ => Mode::Light,
    }
}

/// The active palette.
pub fn palette() -> &'static Palette {
    if is_edit_suite() { &edit_suite::PALETTE } else { mode().palette() }
}

/// Switches the mode and restyles `ctx`, so the next frame is drawn in it.
pub fn set_mode(ctx: &egui::Context, mode: Mode) {
    MODE.store(mode as u8, Ordering::Relaxed);
    ctx.set_theme(if dark_chrome() { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.all_styles_mut(apply_style);
}

/// Applies the frame's tokens without changing the saved appearance preference.
pub fn set_layout(ctx: &egui::Context, layout: FrameLayout) {
    let next = u8::from(layout == FrameLayout::EditSuite);
    if LAYOUT.swap(next, Ordering::Relaxed) != next {
        set_mode(ctx, mode());
    }
}

pub fn chrome() -> Color32 {
    palette().chrome
}
pub fn panel() -> Color32 {
    palette().panel
}
pub fn panel_header() -> Color32 {
    palette().panel_header
}
pub fn line() -> Color32 {
    palette().line
}
pub fn line_strong() -> Color32 {
    palette().line_strong
}
pub fn ink() -> Color32 {
    palette().ink
}
pub fn muted() -> Color32 {
    palette().muted
}
pub fn accent() -> Color32 {
    palette().accent
}
pub fn accent_soft() -> Color32 {
    palette().accent_soft
}
/// The accent at `alpha` of 255, for washes over the viewport and the canvas.
pub fn accent_alpha(alpha: u8) -> Color32 {
    let a = palette().accent;
    Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), alpha)
}
pub fn selection_bg() -> Color32 {
    palette().selection_bg
}
pub fn selection_ink() -> Color32 {
    palette().selection_ink
}
pub fn hover() -> Color32 {
    palette().hover
}
pub fn ok() -> Color32 {
    palette().ok
}
pub fn bounded() -> Color32 {
    palette().bounded
}
pub fn warn() -> Color32 {
    palette().warn
}
pub fn error() -> Color32 {
    palette().error
}
pub fn warn_soft() -> Color32 {
    palette().warn_soft
}
pub fn error_soft() -> Color32 {
    palette().error_soft
}
pub fn field_bg() -> Color32 {
    palette().field_bg
}
pub fn chip() -> Color32 {
    palette().chip
}
pub fn on_accent() -> Color32 {
    palette().on_accent
}
pub fn hud_bg() -> Color32 {
    palette().hud_bg
}
pub fn command_line() -> Color32 {
    palette().command_line
}
pub fn command_prompt() -> Color32 {
    palette().command_prompt
}
pub fn terminal_bg() -> Color32 {
    palette().terminal_bg
}
pub fn terminal_ink() -> Color32 {
    palette().terminal_ink
}
pub fn terminal_accent() -> Color32 {
    palette().terminal_accent
}
/// The viewport's backdrop, top and bottom of the fade.
pub fn viewport_backdrop() -> [Color32; 2] {
    let p = palette();
    [p.viewport_top, p.viewport_bottom]
}
pub fn canvas_bg() -> Color32 {
    palette().canvas_bg
}
pub fn axis_x() -> Color32 {
    palette().axis_x
}
pub fn axis_y() -> Color32 {
    palette().axis_y
}
pub fn axis_z() -> Color32 {
    palette().axis_z
}
/// The colour of an origin axis, in the viewport and the feature tree.
pub fn axis_color(axis: crate::document::AxisKind) -> Color32 {
    match axis {
        crate::document::AxisKind::X => axis_x(),
        crate::document::AxisKind::Y => axis_y(),
        crate::document::AxisKind::Z => axis_z(),
    }
}
/// The session's quiet text: notes, tool lines, the header.
pub fn terminal_muted() -> Color32 {
    terminal_ink().gamma_multiply(0.6)
}
/// The session's separator lines.
pub fn terminal_line() -> Color32 {
    terminal_ink().gamma_multiply(0.16)
}

/// Corner radius used by buttons, fields and cards.
pub const RADIUS: u8 = 4;
/// Corner radius of windows, menus and popups.
pub const WINDOW_RADIUS: u8 = 6;

pub const MENU_HEIGHT: f32 = 30.0;
pub const RIBBON_TABS_HEIGHT: f32 = 26.0;
pub const RIBBON_BODY_HEIGHT: f32 = 92.0;
pub const COMMAND_LINE_HEIGHT: f32 = 32.0;
/// Keep room for the model and an optional pane in a narrow Edit Suite window.
pub const COMPACT_WIDTH: f32 = 1100.0;
pub const ROW_HEIGHT: f32 = 22.0;

pub const FONT_REGULAR: &str = "Inter";
pub const FONT_MEDIUM: &str = "InterMedium";
pub const FONT_SEMIBOLD: &str = "InterSemiBold";

pub fn medium() -> FontFamily {
    FontFamily::Name(Arc::from(FONT_MEDIUM))
}

pub fn semibold() -> FontFamily {
    FontFamily::Name(Arc::from(FONT_SEMIBOLD))
}

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn font_medium(size: f32) -> FontId {
    FontId::new(size, medium())
}

pub fn font_semibold(size: f32) -> FontId {
    FontId::new(size, semibold())
}

/// The command line's and the session's typeface.
pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// Installs the fonts and the style for `mode` on a fresh context.
pub fn install(ctx: &egui::Context, mode: Mode) {
    install_for_layout(ctx, mode, FrameLayout::Classic);
}

/// Installs the shared fonts and the selected frame's tokens on a fresh context.
pub fn install_for_layout(ctx: &egui::Context, mode: Mode, layout: FrameLayout) {
    ctx.set_fonts(fonts());
    LAYOUT.store(u8::from(layout == FrameLayout::EditSuite), Ordering::Relaxed);
    set_mode(ctx, mode);
}

fn fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        FONT_REGULAR.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/Inter-Regular.ttf"))),
    );
    fonts.font_data.insert(
        FONT_MEDIUM.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/Inter-Medium.ttf"))),
    );
    fonts.font_data.insert(
        FONT_SEMIBOLD.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/Inter-SemiBold.ttf"))),
    );
    // Keep egui's bundled fonts as fallbacks for glyphs Inter lacks (symbols, emoji).
    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let with = |first: &str| {
        let mut v = vec![first.to_owned()];
        v.extend(fallbacks.iter().cloned());
        v
    };
    fonts.families.insert(FontFamily::Proportional, with(FONT_REGULAR));
    fonts.families.insert(medium(), with(FONT_MEDIUM));
    fonts.families.insert(semibold(), with(FONT_SEMIBOLD));
    fonts
}

fn apply_style(style: &mut egui::Style) {
    style.text_styles = [
        (TextStyle::Small, FontId::new(edit_suite::SMALL_SIZE, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(edit_suite::BODY_SIZE, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(12.0, FontFamily::Proportional)),
        (TextStyle::Heading, FontId::new(13.5, semibold())),
        (TextStyle::Monospace, FontId::new(edit_suite::VALUE_SIZE, FontFamily::Monospace)),
    ]
    .into();

    let spacing = &mut style.spacing;
    spacing.item_spacing = Vec2::new(6.0, 4.0);
    spacing.button_padding = Vec2::new(8.0, 3.0);
    spacing.interact_size = Vec2::new(40.0, ROW_HEIGHT);
    spacing.indent = 16.0;
    spacing.icon_width = 14.0;
    spacing.icon_width_inner = 9.0;
    spacing.menu_margin = Margin::same(if is_edit_suite() { 4 } else { 6 });
    spacing.window_margin = Margin::same(10);
    spacing.combo_width = 120.0;
    spacing.text_edit_width = 160.0;
    spacing.scroll.bar_width = 6.0;
    spacing.scroll.floating = true;
    spacing.scroll.handle_min_length = 24.0;
    spacing.tooltip_width = 320.0;

    style.interaction.selectable_labels = false;
    style.interaction.tooltip_delay = 0.35;
    style.animation_time = 0.08;
    style.visuals = visuals();
}

fn visuals() -> egui::Visuals {
    let p = palette();
    let dark = dark_chrome();
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let r = CornerRadius::same(if is_edit_suite() { edit_suite::RADIUS } else { RADIUS });
    let shadow = |blur: u8, offset: i8| egui::Shadow {
        offset: [0, offset],
        blur,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 110 } else { 28 }),
    };
    v.override_text_color = None;
    v.panel_fill = p.panel;
    v.window_fill = if is_edit_suite() { edit_suite::POPUP } else { p.panel };
    v.window_stroke = Stroke::new(1.0, if is_edit_suite() { edit_suite::CHIP_LINE } else { p.line });
    v.window_corner_radius = CornerRadius::same(WINDOW_RADIUS);
    v.window_shadow = shadow(18, 6);
    v.popup_shadow = shadow(12, 4);
    v.menu_corner_radius = CornerRadius::same(WINDOW_RADIUS);
    v.extreme_bg_color = p.field_bg;
    v.faint_bg_color = p.panel_header;
    v.code_bg_color = p.panel_header;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warn;
    v.error_fg_color = p.error;
    v.selection.bg_fill = p.selection_bg;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.text_cursor.stroke = Stroke::new(1.5, p.accent);
    v.striped = false;
    v.button_frame = true;
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = false;
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.resize_corner_size = 10.0;
    v.weak_text_alpha = 0.6;
    if !dark || is_edit_suite() {
        // Hints and secondary labels need opaque ink, not an alpha wash over the field.
        v.weak_text_color = Some(p.muted);
    }

    let w = &mut v.widgets;
    w.noninteractive = egui::style::WidgetVisuals {
        weak_bg_fill: p.panel,
        bg_fill: p.panel,
        bg_stroke: Stroke::new(1.0, p.line),
        fg_stroke: Stroke::new(1.0, p.ink),
        corner_radius: r,
        expansion: 0.0,
    };
    w.inactive = egui::style::WidgetVisuals {
        weak_bg_fill: p.panel_header,
        bg_fill: p.panel_header,
        bg_stroke: Stroke::new(1.0, p.line),
        fg_stroke: Stroke::new(1.0, p.ink),
        corner_radius: r,
        expansion: 0.0,
    };
    w.hovered = egui::style::WidgetVisuals {
        weak_bg_fill: p.hover,
        bg_fill: p.hover,
        bg_stroke: Stroke::new(1.0, p.line_strong),
        fg_stroke: Stroke::new(1.0, p.ink),
        corner_radius: r,
        expansion: 0.0,
    };
    w.active = egui::style::WidgetVisuals {
        weak_bg_fill: p.selection_bg,
        bg_fill: p.selection_bg,
        bg_stroke: Stroke::new(1.0, p.accent),
        fg_stroke: Stroke::new(1.0, p.selection_ink),
        corner_radius: r,
        expansion: 0.0,
    };
    w.open = egui::style::WidgetVisuals {
        weak_bg_fill: p.accent_soft,
        bg_fill: p.accent_soft,
        bg_stroke: Stroke::new(1.0, p.line),
        fg_stroke: Stroke::new(1.0, p.ink),
        corner_radius: r,
        expansion: 0.0,
    };
    v
}

/// egui_dock style matched to the panel chrome: a thin tab strip on the header tone,
/// no body frame, one-pixel separators that light up in the accent when hovered.
pub fn dock_style(ctx: &egui::Context) -> egui_dock::Style {
    let p = palette();
    let mut s = egui_dock::Style::from_egui(&ctx.global_style());
    s.dock_area_padding = None;
    s.main_surface_border_stroke = Stroke::NONE;
    s.main_surface_border_rounding = CornerRadius::ZERO;

    s.separator.width = 1.0;
    s.separator.extra_interact_width = 5.0;
    s.separator.extra = 0.0;
    s.separator.color_idle = p.line;
    s.separator.color_hovered = p.accent;
    s.separator.color_dragged = p.accent;

    s.tab_bar.bg_fill = p.panel_header;
    s.tab_bar.height = 28.0;
    s.tab_bar.hline_color = p.line;
    s.tab_bar.fill_tab_bar = false;
    s.tab_bar.corner_radius = CornerRadius::ZERO;
    s.tab_bar.inner_margin = Margin::symmetric(6, 0);

    let base = egui_dock::TabInteractionStyle {
        outline_color: Color32::TRANSPARENT,
        corner_radius: CornerRadius::ZERO,
        bg_fill: p.panel_header,
        text_color: p.muted,
    };
    s.tab.inactive = base.clone();
    s.tab.hovered = egui_dock::TabInteractionStyle { text_color: p.ink, ..base.clone() };
    s.tab.inactive_with_kb_focus = base.clone();
    let active = egui_dock::TabInteractionStyle { text_color: p.ink, ..base };
    s.tab.active = active.clone();
    s.tab.focused = active.clone();
    s.tab.active_with_kb_focus = active.clone();
    s.tab.focused_with_kb_focus = active;
    s.tab.hline_below_active_tab_name = true;
    s.tab.spacing = 10.0;
    s.tab.minimum_width = None;
    s.tab.tab_body.inner_margin = Margin::ZERO;
    s.tab.tab_body.stroke = Stroke::NONE;
    s.tab.tab_body.corner_radius = CornerRadius::ZERO;
    s.tab.tab_body.bg_fill = p.panel;

    s.overlay.selection_color = accent_alpha(60);
    s.overlay.selection_stroke_width = 1.0;
    s.overlay.button_color = p.accent;
    s.overlay.button_border_stroke = Stroke::new(1.0, p.accent);
    s.overlay.hovered_leaf_highlight.color = accent_alpha(24);
    s.overlay.hovered_leaf_highlight.stroke = Stroke::new(1.0, p.accent);
    s.buttons.close_tab_color = p.muted;
    s.buttons.close_tab_active_color = p.ink;
    s.buttons.close_tab_bg_fill = p.hover;
    s.buttons.add_tab_color = p.muted;
    s.buttons.add_tab_active_color = p.ink;
    s.buttons.add_tab_bg_fill = p.hover;
    s.buttons.add_tab_border_color = p.line;
    s
}

/// A 1-pixel separator line across the full width of `ui`.
pub fn hline(ui: &mut egui::Ui) {
    let rect = ui.available_rect_before_wrap();
    let y = ui.cursor().top();
    ui.painter().hline(rect.x_range(), y, Stroke::new(1.0, line()));
    ui.add_space(1.0);
}

/// Section header inside panels: small muted semibold text.
pub fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).family(semibold()).size(11.0).color(muted()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_parse_and_light_is_the_default() {
        assert_eq!(Mode::default(), Mode::Light);
        for m in Mode::ALL {
            assert_eq!(Mode::parse(&m.label().to_uppercase()), Some(m));
        }
        assert_eq!(Mode::parse("sepia"), None);
        assert_eq!(serde_json::to_string(&Mode::Dark).unwrap(), "\"dark\"");
    }
}
