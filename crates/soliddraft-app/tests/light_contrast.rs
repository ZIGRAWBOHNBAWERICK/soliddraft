//! Contrast of the light theme's text and controls on their shared backgrounds.
use egui::Color32;
use soliddraft_app::theme::LIGHT;

fn luminance(color: Color32) -> f64 {
    let linear = |channel: u8| {
        let value = f64::from(channel) / 255.0;
        if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}

fn contrast(foreground: Color32, background: Color32) -> f64 {
    let a = luminance(foreground);
    let b = luminance(background);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn light_text_and_controls_meet_aa() {
    let p = LIGHT;
    let backgrounds = [
        p.chrome,
        p.panel,
        p.panel_header,
        p.field_bg,
        p.chip,
        p.selection_bg,
        p.hover,
        p.accent_soft,
        p.warn_soft,
        p.error_soft,
        p.hud_bg,
        p.command_line,
        p.viewport_top,
        p.viewport_bottom,
        p.canvas_bg,
    ];
    for background in backgrounds {
        for foreground in [p.ink, p.muted, p.accent, p.ok, p.warn, p.error] {
            let ratio = contrast(foreground, background);
            assert!(ratio >= 4.5, "text {foreground:?} on {background:?}: {ratio:.2}:1");
        }
        for foreground in [p.line, p.line_strong, p.axis_x, p.axis_y, p.axis_z] {
            let ratio = contrast(foreground, background);
            assert!(ratio >= 3.0, "control {foreground:?} on {background:?}: {ratio:.2}:1");
        }
    }
    assert!(contrast(p.selection_ink, p.selection_bg) >= 4.5);
    assert!(contrast(p.on_accent, p.accent) >= 4.5);
    assert!(contrast(p.ink, p.line) >= 4.5, "secondary button hover text");
    assert!(contrast(p.command_prompt, p.command_line) >= 4.5);
}

#[test]
fn light_field_hints_remain_readable() {
    let ctx = egui::Context::default();
    soliddraft_app::theme::install(&ctx, soliddraft_app::theme::Mode::Light);
    let style = ctx.global_style();
    let hint = style.visuals.weak_text_color();
    assert_eq!(hint.a(), 255, "hints cannot fade into the field");
    assert!(contrast(hint, style.visuals.extreme_bg_color) >= 4.5);
}
