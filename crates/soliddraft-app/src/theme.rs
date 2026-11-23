//! Shared D1 Clear Bench theme tokens: colours, spacing, radii and typography in one
//! structured token set so a later themes lane can register additional themes cleanly.
//! D1 Clear Bench daylight is the default. Viewport scenes remain a separate choice.
//!
//! The mode is process-wide: [`set_mode`] switches it and restyles the context, and the
//! accessors ([`ink`], [`panel`], ...) read the active token set each call, so a widget
//! painted on the next frame is already in the new mode.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Vec2};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// The saved appearance preference.
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

    pub fn is_dark(self) -> bool {
        self == Mode::Dark
    }
}

/// Shared semantic colours, read by egui and custom painters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Window chrome: top bar, left toggle rail, frame gaps.
    pub chrome: Color32,
    /// Ribbon / top tool strip background.
    pub ribbon: Color32,
    /// Panels: side column panes, active document tab, dialogs and menus.
    pub panel: Color32,
    /// Panel headers, splitters and secondary buttons.
    pub panel_header: Color32,
    /// One-pixel separators and widget borders.
    pub line: Color32,
    /// Faint internal dividers.
    pub line_faint: Color32,
    /// A stronger border, for a hovered widget or floating card.
    pub line_strong: Color32,
    pub ink: Color32,
    pub muted: Color32,
    pub accent: Color32,
    /// Accent hover fill for primary CTA buttons.
    pub accent_hover: Color32,
    /// The accent as a soft fill: active ribbon item, selected tree row.
    pub accent_soft: Color32,
    pub selection_bg: Color32,
    pub selection_ink: Color32,
    pub hover: Color32,
    pub ok: Color32,
    pub ok_soft: Color32,
    pub warn: Color32,
    pub error: Color32,
    /// Notice fills behind warn and error text.
    pub warn_soft: Color32,
    pub error_soft: Color32,
    /// Destructive / Close sketch CTA.
    pub danger: Color32,
    pub on_danger: Color32,
    /// Coral Donate pill in the top bar.
    pub donate_bg: Color32,
    pub donate_ink: Color32,
    pub donate_line: Color32,
    /// Contextual inline feature form strip fill.
    pub form_bg: Color32,
    /// Text fields and value pills.
    pub field_bg: Color32,
    /// Tool chips and raised buttons.
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
    pub viewport_top: Color32,
    pub viewport_bottom: Color32,
    /// The drawing canvas around the sheet; the sheet itself stays paper white.
    pub canvas_bg: Color32,
    /// The origin axes.
    pub axis_x: Color32,
    pub axis_y: Color32,
    pub axis_z: Color32,
    /// The origin planes and reference planes. A hue of its own: in the accent's blue a
    /// plane hid the Z axis that crosses it.
    pub reference_plane: Color32,
}

/// Unified spacing and shell dimensions in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    pub top_bar_height: f32,
    pub workspace_strip_height: f32,
    pub tool_strip_height: f32,
    pub options_height: f32,
    pub rail_width: f32,
    pub side_panel_default_width: f32,
    pub side_panel_min_width: f32,
    pub pane_header_height: f32,
    pub splitter_height: f32,
    pub bottom_bar_height: f32,
    pub row_height: f32,
    pub control_height: f32,
    pub icon_size: f32,
    pub ribbon_icon_size: f32,
    pub tool_padding: f32,
    pub group_gap: f32,
}

/// Unified typography sizes in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Typography {
    pub heading: f32,
    pub body: f32,
    pub button: f32,
    pub tool_label: f32,
    pub small: f32,
    pub rail_label: f32,
    pub group_label: f32,
    pub value: f32,
}

/// Unified corner radii in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Radii {
    pub control: u8,
    pub field: u8,
    pub tool: u8,
    pub window: u8,
    pub floating: u8,
    pub pill: u8,
}

/// One complete UI theme token set. The later themes lane can supply additional
/// `Tokens` definitions without changing widget code.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub id: &'static str,
    pub label: &'static str,
    pub is_dark: bool,
    pub palette: Palette,
    pub spacing: Spacing,
    pub typography: Typography,
    pub radii: Radii,
    pub nav_shadow: egui::Shadow,
    pub popup_shadow: egui::Shadow,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied_const(r, g, b, a)
}

/// D1 Clear Bench daylight token set (approved design direction).
pub mod clear_bench {
    use super::{Palette, Radii, Spacing, Tokens, Typography, rgb, rgba};
    use egui::Color32;

    pub const PALETTE: Palette = Palette {
        chrome: rgb(0xe9, 0xed, 0xf3),
        ribbon: rgb(0xf5, 0xf7, 0xfa),
        panel: Color32::WHITE,
        panel_header: rgb(0xf1, 0xf4, 0xf8),
        line: rgb(0xd3, 0xda, 0xe4),
        line_faint: rgb(0xe7, 0xec, 0xf3),
        line_strong: rgb(0xb5, 0xc0, 0xd0),
        ink: rgb(0x17, 0x20, 0x30),
        muted: rgb(0x4c, 0x58, 0x6a),
        accent: rgb(0x1d, 0x52, 0xd6),
        accent_hover: rgb(0x17, 0x43, 0xb5),
        accent_soft: rgb(0xe4, 0xee, 0xff),
        selection_bg: rgb(0xe4, 0xee, 0xff),
        selection_ink: rgb(0x1d, 0x52, 0xd6),
        hover: rgb(0xe5, 0xea, 0xf2),
        ok: rgb(0x12, 0x6e, 0x3e),
        ok_soft: rgb(0xe2, 0xf7, 0xec),
        warn: rgb(0x9a, 0x44, 0x00),
        error: rgb(0xc0, 0x20, 0x20),
        warn_soft: rgb(0xfe, 0xf3, 0xc7),
        error_soft: rgb(0xfe, 0xe2, 0xe2),
        danger: rgb(0xc9, 0x26, 0x26),
        on_danger: Color32::WHITE,
        donate_bg: rgb(0xfe, 0xe9, 0xe7),
        donate_ink: rgb(0xb4, 0x23, 0x18),
        donate_line: rgb(0xf7, 0xb8, 0xb2),
        form_bg: rgb(0xf0, 0xf5, 0xff),
        field_bg: Color32::WHITE,
        chip: rgb(0xee, 0xf2, 0xf7),
        on_accent: Color32::WHITE,
        hud_bg: rgba(255, 255, 255, 242),
        command_line: rgb(0xf5, 0xf7, 0xfa),
        command_prompt: rgb(0x1d, 0x52, 0xd6),
        terminal_bg: rgb(0x17, 0x18, 0x1b),
        terminal_ink: rgb(0xe6, 0xe9, 0xee),
        terminal_accent: rgb(0x60, 0xa5, 0xfa),
        viewport_top: Color32::WHITE,
        viewport_bottom: Color32::WHITE,
        canvas_bg: rgb(0xdf, 0xe4, 0xec),
        axis_x: rgb(0xd2, 0x3b, 0x3b),
        axis_y: rgb(0x2e, 0x9e, 0x5b),
        axis_z: rgb(0x1f, 0x5f, 0xd0),
        reference_plane: rgb(0xf5, 0xa5, 0x24),
    };

    pub const SPACING: Spacing = Spacing {
        top_bar_height: 36.0,
        workspace_strip_height: 28.0,
        tool_strip_height: 58.0,
        options_height: 44.0,
        rail_width: 46.0,
        side_panel_default_width: 266.0,
        side_panel_min_width: 210.0,
        pane_header_height: 30.0,
        splitter_height: 7.0,
        bottom_bar_height: 32.0,
        row_height: 24.0,
        control_height: 28.0,
        icon_size: 16.0,
        ribbon_icon_size: 20.0,
        tool_padding: 6.0,
        group_gap: 10.0,
    };

    pub const TYPOGRAPHY: Typography = Typography {
        heading: 14.0,
        body: 12.5,
        button: 12.0,
        tool_label: 11.0,
        small: 10.5,
        rail_label: 9.5,
        group_label: 9.5,
        value: 12.0,
    };

    pub const RADII: Radii = Radii { control: 6, field: 5, tool: 7, window: 8, floating: 10, pill: 99 };

    pub const TOKENS: Tokens = Tokens {
        id: "clear_bench",
        label: "Clear Bench (Daylight)",
        is_dark: false,
        palette: PALETTE,
        spacing: SPACING,
        typography: TYPOGRAPHY,
        radii: RADII,
        nav_shadow: egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: rgba(15, 23, 42, 34) },
        popup_shadow: egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: rgba(15, 23, 42, 38) },
    };
}

/// Compatibility constants backed by the active D1 Clear Bench daylight token set.
pub mod edit_suite {
    use super::{Palette, clear_bench, rgb};
    use egui::Color32;

    pub const BAR: Color32 = clear_bench::PALETTE.panel_header;
    pub const CHIP_LINE: Color32 = clear_bench::PALETTE.line_strong;
    pub const POPUP: Color32 = clear_bench::PALETTE.panel;
    pub const POPUP_HOVER: Color32 = clear_bench::PALETTE.hover;
    pub const PRIMARY_HOVER: Color32 = clear_bench::PALETTE.accent_hover;
    pub const DANGER: Color32 = clear_bench::PALETTE.danger;
    pub const ON_DANGER: Color32 = clear_bench::PALETTE.on_danger;
    pub const UNDER_DEFINED: Color32 = rgb(0x1d, 0x4f, 0xe0);
    pub const UNDER_DEFINED_LABEL: Color32 = rgb(0x1d, 0x52, 0xd6);
    /// A form's preview guides: the depth arrow, a draft's plane and pull, copy numbers.
    pub const PREVIEW_EDGE: Color32 = rgb(0xd9, 0x7f, 0x00);
    /// The red of material a form removes, for a hole's ghost at the pointer.
    pub const PREVIEW_REMOVE_EDGE: Color32 = rgb(0xd0, 0x3a, 0x30);
    pub const FULLY_DEFINED: Color32 = rgb(0x0b, 0x0d, 0x11);
    pub const CONSTRUCTION: Color32 = rgb(0x5b, 0x6b, 0x86);
    pub const DIMENSION: Color32 = rgb(0x1d, 0x52, 0xd6);
    pub const SKETCH_SHEET: Color32 = rgb(0xf1, 0xf3, 0xf7);

    pub const PALETTE: Palette = clear_bench::PALETTE;

    pub const TITLE_HEIGHT: f32 = clear_bench::SPACING.top_bar_height;
    pub const OPTIONS_HEIGHT: f32 = clear_bench::SPACING.options_height;
    pub const PALETTE_WIDTH: f32 = 250.0;
    pub const PANE_WIDTH: f32 = clear_bench::SPACING.side_panel_default_width;
    pub const COMMAND_LINE_HEIGHT: f32 = clear_bench::SPACING.bottom_bar_height;
    pub const TOOL_HEIGHT: f32 = 23.0;
    pub const ROW_HEIGHT: f32 = clear_bench::SPACING.row_height;
    pub const CONTROL_HEIGHT: f32 = clear_bench::SPACING.control_height;
    pub const ICON_SIZE: f32 = clear_bench::SPACING.icon_size;
    pub const COLUMN_GAP: f32 = 4.0;
    pub const TOOL_PADDING: f32 = clear_bench::SPACING.tool_padding;
    pub const GROUP_GAP: f32 = clear_bench::SPACING.group_gap;
    pub const BODY_SIZE: f32 = clear_bench::TYPOGRAPHY.body;
    pub const TOOL_LABEL_SIZE: f32 = clear_bench::TYPOGRAPHY.tool_label;
    pub const SMALL_SIZE: f32 = clear_bench::TYPOGRAPHY.small;
    pub const GROUP_LABEL_SIZE: f32 = clear_bench::TYPOGRAPHY.group_label;
    pub const VALUE_SIZE: f32 = clear_bench::TYPOGRAPHY.value;
    pub const RADIUS: u8 = clear_bench::RADII.control;
    pub const FIELD_RADIUS: u8 = clear_bench::RADII.field;
    pub const TOOL_RADIUS: u8 = clear_bench::RADII.tool;
    pub const FLOATING_RADIUS: u8 = clear_bench::RADII.floating;
    pub const NAV_SHADOW: egui::Shadow = clear_bench::TOKENS.nav_shadow;
}

static MODE: AtomicU8 = AtomicU8::new(0);

/// The mode the interface is in.
pub fn mode() -> Mode {
    match MODE.load(Ordering::Relaxed) {
        1 => Mode::Dark,
        _ => Mode::Light,
    }
}

/// The active token set. A future themes lane can swap this to any registered `Tokens`.
pub fn tokens() -> &'static Tokens {
    &clear_bench::TOKENS
}

/// Stable id of the active theme token set.
pub fn active_theme_id() -> &'static str {
    tokens().id
}

/// Whether the active UI chrome is dark.
pub fn is_dark() -> bool {
    tokens().is_dark
}

/// The active palette.
pub fn palette() -> &'static Palette {
    &tokens().palette
}

/// The active spacing metrics.
pub fn spacing() -> &'static Spacing {
    &tokens().spacing
}

/// The active typography sizes.
pub fn typography() -> &'static Typography {
    &tokens().typography
}

/// The active corner radii.
pub fn radii() -> &'static Radii {
    &tokens().radii
}

/// Switches the mode and restyles `ctx`, so the next frame is drawn in it.
pub fn set_mode(ctx: &egui::Context, mode: Mode) {
    MODE.store(mode as u8, Ordering::Relaxed);
    ctx.set_theme(if is_dark() { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.all_styles_mut(apply_style);
}

pub fn chrome() -> Color32 {
    palette().chrome
}
pub fn ribbon() -> Color32 {
    palette().ribbon
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
pub fn line_faint() -> Color32 {
    palette().line_faint
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
pub fn accent_hover() -> Color32 {
    palette().accent_hover
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
pub fn ok_soft() -> Color32 {
    palette().ok_soft
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
pub fn danger() -> Color32 {
    palette().danger
}
pub fn on_danger() -> Color32 {
    palette().on_danger
}
pub fn donate_bg() -> Color32 {
    palette().donate_bg
}
pub fn donate_ink() -> Color32 {
    palette().donate_ink
}
pub fn donate_line() -> Color32 {
    palette().donate_line
}
pub fn form_bg() -> Color32 {
    palette().form_bg
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

pub fn reference_plane() -> Color32 {
    palette().reference_plane
}
/// The colour of an origin axis, in the viewport and the feature tree.
pub fn axis_color(axis: crate::document::AxisKind) -> Color32 {
    match axis {
        crate::document::AxisKind::X => axis_x(),
        crate::document::AxisKind::Y => axis_y(),
        crate::document::AxisKind::Z => axis_z(),
    }
}
/// Dimension and witness lines over the modelling space. A light scene takes the blue an
/// open sketch draws its dimensions in; a dark scene uses light ink.
pub fn dimension_line(dark_scene: bool) -> Color32 {
    if dark_scene { Color32::from_rgb(0xd8, 0xe1, 0xee) } else { edit_suite::DIMENSION }
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
pub const RADIUS: u8 = clear_bench::RADII.control;
/// Corner radius of windows, menus and popups.
pub const WINDOW_RADIUS: u8 = clear_bench::RADII.window;

pub const MENU_HEIGHT: f32 = clear_bench::SPACING.top_bar_height;
pub const COMMAND_LINE_HEIGHT: f32 = clear_bench::SPACING.bottom_bar_height;
/// Keep room for the model and an optional pane in a narrow window.
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
    ctx.set_fonts(fonts());
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
    let t = typography();
    style.text_styles = [
        (TextStyle::Small, FontId::new(t.small, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(t.body, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(t.button, FontFamily::Proportional)),
        (TextStyle::Heading, FontId::new(t.heading, semibold())),
        (TextStyle::Monospace, FontId::new(t.value, FontFamily::Monospace)),
    ]
    .into();

    let spacing = &mut style.spacing;
    spacing.item_spacing = Vec2::new(6.0, 4.0);
    spacing.button_padding = Vec2::new(8.0, 4.0);
    spacing.interact_size = Vec2::new(40.0, ROW_HEIGHT);
    spacing.indent = 16.0;
    spacing.icon_width = 14.0;
    spacing.icon_width_inner = 9.0;
    spacing.menu_margin = Margin::same(6);
    spacing.window_margin = Margin::same(12);
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
    let t = tokens();
    let p = &t.palette;
    let mut v = if t.is_dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let r = CornerRadius::same(t.radii.control);
    v.override_text_color = None;
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.window_stroke = Stroke::new(1.0, p.line_strong);
    v.window_corner_radius = CornerRadius::same(t.radii.window);
    v.window_shadow = t.popup_shadow;
    v.popup_shadow = t.nav_shadow;
    v.menu_corner_radius = CornerRadius::same(t.radii.window);
    v.extreme_bg_color = p.field_bg;
    v.faint_bg_color = p.panel_header;
    v.code_bg_color = p.chip;
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
    v.weak_text_alpha = 0.7;

    // Hints and secondary labels need opaque ink, not an alpha wash over the field.
    v.weak_text_color = Some(p.muted);

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
        weak_bg_fill: p.chip,
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
        bg_stroke: Stroke::new(1.0, p.accent),
        fg_stroke: Stroke::new(1.0, p.selection_ink),
        corner_radius: r,
        expansion: 0.0,
    };
    v
}

/// egui_dock style matched to the D1 Clear Bench chrome. Document tabs live in the top
/// bar, so the center dock's own tab strip is collapsed.
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
    s.tab_bar.height = 0.0;
    s.tab_bar.hline_color = Color32::TRANSPARENT;
    s.tab_bar.fill_tab_bar = false;
    s.tab_bar.corner_radius = CornerRadius::ZERO;
    s.tab_bar.inner_margin = Margin::ZERO;

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
    s.tab.hline_below_active_tab_name = false;
    s.tab.spacing = 0.0;
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
        assert_eq!(active_theme_id(), "clear_bench");
        assert!(!is_dark());
        for m in Mode::ALL {
            assert_eq!(Mode::parse(&m.label().to_uppercase()), Some(m));
        }
        assert_eq!(Mode::parse("sepia"), None);
        assert_eq!(serde_json::to_string(&Mode::Dark).unwrap(), "\"dark\"");
    }
}
