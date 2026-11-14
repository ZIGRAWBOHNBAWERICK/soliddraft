//! Contrast of text on the shared Edit Suite chrome.
use egui::Color32;
use soliddraft_app::theme::edit_suite::PALETTE;

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
fn edit_suite_body_text_meets_aa() {
    let p = PALETTE;
    let backgrounds = [p.chrome, p.panel, p.panel_header, p.field_bg, p.chip, p.hud_bg, p.command_line];
    for background in backgrounds {
        for foreground in [p.ink, p.muted, p.accent, p.ok, p.warn, p.error] {
            let ratio = contrast(foreground, background);
            assert!(ratio >= 4.5, "text {foreground:?} on {background:?}: {ratio:.2}:1");
        }
    }
    assert!(contrast(p.on_accent, p.accent) >= 4.5);
    assert!(contrast(p.command_prompt, p.command_line) >= 4.5);
}

#[test]
fn field_hints_remain_readable() {
    for mode in soliddraft_app::theme::Mode::ALL {
        let ctx = egui::Context::default();
        soliddraft_app::theme::install(&ctx, mode);
        let style = ctx.global_style();
        let hint = style.visuals.weak_text_color();
        assert_eq!(hint.a(), 255, "hints cannot fade into the field");
        let ratio = contrast(hint, style.visuals.extreme_bg_color);
        assert!(ratio >= 4.5, "{mode:?} field hint contrast {ratio:.12}:1 must reach 4.5:1");
        assert_eq!(style.visuals.extreme_bg_color, PALETTE.field_bg);
    }
}

#[test]
fn workspace_and_sketch_banner_text_meet_aa() {
    use soliddraft_app::theme::edit_suite::{WORKSPACE_BG, WORKSPACE_SELECTED};
    assert!(contrast(PALETTE.ink, WORKSPACE_SELECTED) >= 4.5);
    assert!(contrast(PALETTE.muted, WORKSPACE_BG) >= 4.5);
    assert!(contrast(PALETTE.error, WORKSPACE_BG) >= 4.5);
}
