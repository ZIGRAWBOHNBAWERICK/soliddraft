use super::{Change, GRAPHICS, Preferences, ui};
use crate::graphics::{CurrentView, CurvedSurfaces, GraphicsPreset, SectionHatch, ViewTarget};
use crate::viewport::{
    look::{Preset, Tier},
    render::DisplayMode,
};

pub(super) const CONTROLS: &[(&str, &str)] = &[
    ("preset", "Graphics preset: Performance, Quality, Custom"),
    ("curves", "Curved surfaces: Coarse, Normal, Fine, Ultra"),
    ("samples", "Anti-aliasing"),
    ("supersampling", "Supersampling when the view is still"),
    ("grid", "Show grid"),
    ("shadow", "Ground shadow"),
    ("perspective", "Perspective"),
    ("occlusion", "Ambient occlusion"),
    ("shadow_follows", "Ground shadow follows the model"),
    ("display", "Visual style"),
    ("fov", "Field of view"),
    ("angles", "View angles"),
    ("vertices", "Vertex markers"),
    ("measurement", "Quick measurement"),
    ("fps", "Frame rate"),
    ("fps_details", "Frame rate details"),
    ("threads", "Cosmetic thread markings"),
    ("halo", "Halo under sketch curves"),
    ("hatch", "Section hatch style"),
    ("tint", "Tint new surface in a preview"),
    ("removal", "Show removed material in red"),
    ("hidden", "Hidden edges of selected bodies"),
    ("references", "Reference geometry through bodies"),
    ("follow_grid", "Grid follows the view"),
    ("surface_analysis", "Single surface analysis"),
    ("gpu", "GPU and renderer"),
];

fn row<R>(ui: &mut egui::Ui, highlight: Option<&str>, id: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui::control(highlight == Some(id), ui, add)
}

fn toggle(ui: &mut egui::Ui, highlight: Option<&str>, id: &str, label: &str, value: &mut bool) {
    row(ui, highlight, id, |ui| ui.checkbox(value, label));
}

pub(super) fn show(p: &mut Preferences, ui: &mut egui::Ui) {
    ui.heading(super::page(GRAPHICS).unwrap().path());
    let highlight = p.highlight.as_deref();
    let device = &p.graphics_device;
    let mut graphics = p.graphics;
    let mut view = p.current_view;
    row(ui, highlight, "preset", |ui| {
        ui.label("Graphics preset");
        ui.horizontal_wrapped(|ui| {
            for preset in GraphicsPreset::ALL {
                let choice = ui.selectable_label(graphics.preset == preset, preset.label());
                #[cfg(target_arch = "wasm32")]
                crate::web::show_control(format!("Graphics preset: {}", preset.label()), choice.rect);
                if choice.clicked() {
                    graphics.choose_preset(preset, device);
                }
            }
        });
    });
    row(ui, highlight, "curves", |ui| {
        let current = graphics.curved_surfaces;
        ui.label("Curved surfaces");
        ui.horizontal_wrapped(|ui| {
            if ui.selectable_label(current.is_none(), "Use part setting").clicked() && current.is_some() {
                graphics.curved_surfaces = None;
                graphics.preset = GraphicsPreset::Custom;
            }
            for choice in CurvedSurfaces::ALL {
                if ui.selectable_label(current == Some(choice), choice.label()).clicked() && current != Some(choice) {
                    graphics.curved_surfaces = Some(choice);
                    graphics.preset = GraphicsPreset::Custom;
                }
            }
        });
        ui.label(
            current.map_or_else(|| "Follow the display quality saved with each part.".into(), |c| c.description()),
        );
        if p.spline_curves_unavailable {
            ui.label("Spline mesh controls · Coming soon").on_hover_text(
                "Spline faces use their certified precision. The curved-surface setting applies to other bodies.",
            );
        }
    });

    ui.separator();
    ui.heading("Edges and shading");
    row(ui, highlight, "samples", |ui| {
        ui.label("Anti-aliasing");
        for count in [1, 2, 4, 8, 16] {
            let supported = device.samples.contains(&count);
            let label = if count == 1 { "Off".into() } else { format!("{count}× MSAA") };
            let text = if supported { label } else { format!("{label} · Not available on this GPU") };
            if ui.add_enabled(supported, egui::Button::selectable(graphics.samples == count, text)).clicked()
                && graphics.samples != count
            {
                graphics.samples = count;
                graphics.preset = GraphicsPreset::Custom;
            }
        }
        let active = device.active_samples;
        if !device.samples.contains(&graphics.samples) {
            ui.label(format!("Saved level is unavailable on this GPU. Using {active}×."));
        } else if active != graphics.samples {
            let restart = if cfg!(target_arch = "wasm32") { "Reload this tab" } else { "Restart SolidDraft" };
            ui.label(format!("{restart} to use this level. Active level is {active}×."));
        }
    });
    row(ui, highlight, "supersampling", |ui| {
        let caption = if cfg!(target_arch = "wasm32") {
            "Supersampling when the view is still · Desktop only"
        } else if !device.supersampling() {
            "Supersampling when the view is still · Not available with this renderer"
        } else {
            "Supersampling when the view is still"
        };
        if ui
            .add_enabled(device.supersampling(), egui::Checkbox::new(&mut graphics.still_supersampling, caption))
            .changed()
        {
            graphics.preset = GraphicsPreset::Custom;
        }
    });

    ui.separator();
    ui.heading("Current view");
    match p.view_target {
        Some(target) => {
            current_view_controls(ui, &mut view, p.view_preset, device.tier, p.camera_editable, target, highlight, true)
        }
        None => {
            ui.label("Open a part or an assembly to change its view.");
        }
    }

    ui.separator();
    ui.heading("In the view");
    toggle(ui, highlight, "angles", "View angles", &mut graphics.show_view_angles);
    toggle(ui, highlight, "vertices", "Vertex markers", &mut graphics.show_vertex_markers);
    toggle(ui, highlight, "measurement", "Quick measurement", &mut graphics.quick_measurement);
    toggle(ui, highlight, "fps", "Frame rate", &mut graphics.show_frame_rate);
    row(ui, highlight, "fps_details", |ui| {
        ui.add_enabled(
            graphics.show_frame_rate,
            egui::Checkbox::new(&mut graphics.frame_rate_details, "Frame rate details"),
        );
    });

    ui.separator();
    ui.heading("Display aids");
    toggle(ui, highlight, "threads", "Cosmetic thread markings", &mut graphics.cosmetic_threads);
    toggle(ui, highlight, "halo", "Halo under sketch curves", &mut graphics.sketch_halo);
    row(ui, highlight, "hatch", |ui| {
        egui::ComboBox::from_label("Section hatch style").selected_text(graphics.section_hatch.label()).show_ui(
            ui,
            |ui| {
                for hatch in SectionHatch::ALL {
                    ui.selectable_value(&mut graphics.section_hatch, hatch, hatch.label());
                }
            },
        );
    });
    toggle(ui, highlight, "tint", "Tint new surface in a preview", &mut graphics.tint_preview);
    toggle(ui, highlight, "removal", "Show removed material in red", &mut graphics.red_removal);
    toggle(ui, highlight, "hidden", "Hidden edges of selected bodies", &mut graphics.hidden_selected_edges);
    toggle(ui, highlight, "references", "Reference geometry through bodies", &mut graphics.xray_references);
    toggle(ui, highlight, "follow_grid", "Grid follows the view", &mut graphics.view_aligned_grid);
    if row(ui, highlight, "surface_analysis", |ui| {
        ui.button("Single surface analysis · Coming soon")
            .on_hover_text("Single surface analysis needs certified curvature samples for the selected face.")
            .clicked()
    }) {
        p.message = Some(super::Message {
            text: "Single surface analysis is coming soon. Certified curvature samples for the selected face are unavailable in this version."
                .into(),
            error: true,
        });
    }

    ui.separator();
    ui.heading("This computer");
    row(ui, highlight, "gpu", |ui| {
        ui.label(format!("GPU: {}", device.gpu));
        ui.label(format!("Renderer: {}", device.renderer));
    });
    if graphics != p.graphics {
        p.graphics = graphics;
        p.changes.push(Change::Graphics(graphics));
    }
    if view != p.current_view {
        p.current_view = view;
        p.changes.push(Change::CurrentView(view));
    }
}

/// The Current view controls of `target`. The ground shadow and ambient occlusion boxes
/// show what the renderer draws, from the same [`crate::graphics::Lighting::look`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn current_view_controls(
    ui: &mut egui::Ui,
    view: &mut CurrentView,
    preset: Preset,
    tier: Tier,
    camera_editable: bool,
    target: ViewTarget,
    highlight: Option<&str>,
    show_style: bool,
) {
    let lit = target.lit();
    let (look, ground_shadow) = view.lighting.look(preset, tier, view.references.show_floor);
    toggle(ui, highlight, "grid", "Show grid", &mut view.references.show_grid);
    row(ui, highlight, "shadow", |ui| {
        let mut shown = lit && ground_shadow;
        if ui.add_enabled(lit, egui::Checkbox::new(&mut shown, "Ground shadow")).changed() {
            view.references.show_floor = shown;
            view.lighting.ground_shadow = Some(shown);
        }
    });
    row(ui, highlight, "perspective", |ui| {
        ui.add_enabled(camera_editable, egui::Checkbox::new(&mut view.perspective, "Perspective"))
            .on_disabled_hover_text("The sketch uses an orthographic view while you edit it.");
    });
    row(ui, highlight, "occlusion", |ui| {
        let supported = tier.has_ambient_occlusion();
        let mut enabled = lit && supported && look.ambient_occlusion > 0.0;
        let label = if supported { "Ambient occlusion" } else { "Ambient occlusion · Not available on this GPU" };
        if ui.add_enabled(lit && supported, egui::Checkbox::new(&mut enabled, label)).changed() {
            view.lighting.ambient_occlusion = Some(enabled);
        }
    });
    row(ui, highlight, "shadow_follows", |ui| {
        let follows = egui::Checkbox::new(&mut view.lighting.shadow_follows_model, "Ground shadow follows the model");
        ui.add_enabled(lit, follows);
    });
    if !lit {
        ui.colored_label(crate::theme::muted(), "Ground shadow and ambient occlusion · Not available in assemblies");
    }
    if show_style {
        row(ui, highlight, "display", |ui| {
            egui::ComboBox::from_label("Visual style").selected_text(view.display.label()).show_ui(ui, |ui| {
                for display in DisplayMode::ALL {
                    ui.selectable_value(&mut view.display, display, display.label());
                }
            });
        });
    }
    row(ui, highlight, "fov", |ui| {
        ui.add_enabled(
            camera_editable && view.perspective,
            egui::Slider::new(&mut view.field_of_view, 15.0..=90.0).suffix("°").text("Field of view"),
        )
        .on_disabled_hover_text("Turn on Perspective to change the field of view.");
    });
}
