use super::{Change, Message, Preferences};
use crate::platform::{self, Purpose, Uploaded};
use crate::theme::{self, Appearance, Background, BuiltIn, Rgb, Selection, Tokens};
use egui::{Color32, RichText, Stroke, Ui, vec2};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

pub(super) const CONTROLS: &[(&str, &str)] = &[
    ("themes", "Theme gallery"),
    ("background", "Viewport background"),
    ("colors", "3D view colours"),
    ("logo", "Logo colour"),
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Filter {
    #[default]
    All,
    Light,
    Dark,
}

impl Filter {
    fn includes(self, dark: bool) -> bool {
        match self {
            Self::All => true,
            Self::Light => !dark,
            Self::Dark => dark,
        }
    }
}

pub(super) fn show(preferences: &mut Preferences, ui: &mut Ui) {
    ui.heading("Appearance");
    ui.add_space(7.0);
    let mut appearance = preferences.appearance.clone();
    gallery(preferences, ui, &mut appearance);
    ui.separator();
    control_heading(preferences, ui, "background", "Viewport background");
    background(preferences, ui, &mut appearance);
    ui.separator();
    control_heading(preferences, ui, "colors", "3D view colours");
    let mut colors = appearance.colors;
    let tokens = appearance.tokens(ui.ctx().system_theme());
    let p = tokens.palette;
    optional_color(ui, "Model edges", &mut colors.edges, crate::viewport::render::DEFAULT_EDGE);
    optional_color(ui, "Selection", &mut colors.selection, crate::viewport::render::DEFAULT_SELECTION);
    optional_color(ui, "Grid lines", &mut colors.grid, p.muted);
    optional_color(ui, "Reference geometry", &mut colors.references, p.reference_plane);
    ui.separator();
    control_heading(preferences, ui, "logo", "Brand");
    optional_color(ui, "Logo colour", &mut colors.logo, p.accent);
    if colors != appearance.colors {
        Arc::make_mut(&mut appearance).colors = colors;
    }
    preferences.edit_appearance(appearance);
}

fn gallery(preferences: &mut Preferences, ui: &mut Ui, appearance: &mut Arc<Appearance>) {
    control_heading(preferences, ui, "themes", "Theme gallery");
    ui.horizontal_wrapped(|ui| {
        for (filter, label) in [(Filter::All, "All"), (Filter::Light, "Light"), (Filter::Dark, "Dark")] {
            let response = ui.selectable_value(&mut preferences.theme_filter, filter, label);
            register(&response, &format!("theme-filter-{label}"));
        }
        let follow = ui.selectable_label(appearance.selected == Selection::System, "Follow system");
        register(&follow, "theme-follow-system");
        if follow.clicked() {
            Arc::make_mut(appearance).selected = Selection::System;
        }
        let import = ui.button("Import theme…").on_hover_text(
            r#"JSON format example: {"format":"soliddraft-theme/1","name":"My theme","base":"clear_bench","accent":[25,80,170]}"#,
        );
        register(&import, "theme-import");
        if import.clicked() {
            pick(preferences, appearance, true);
        }
    });
    if appearance.selected == Selection::System {
        let system = ui.ctx().system_theme();
        let status = match system {
            Some(egui::Theme::Light) => "System theme is light.",
            Some(egui::Theme::Dark) => "System theme is dark.",
            None => "System theme unavailable. Using Clear Bench Daylight.",
        };
        ui.weak(status);
    }
    ui.add_space(3.0);
    ui.horizontal_wrapped(|ui| {
        for builtin in BuiltIn::ALL {
            let tokens = builtin.tokens();
            if preferences.theme_filter.includes(tokens.is_dark)
                && card(ui, tokens.label, tokens, appearance.selected == Selection::BuiltIn(builtin))
            {
                Arc::make_mut(appearance).selected = Selection::BuiltIn(builtin);
            }
        }
        if let Some(imported) = &appearance.imported {
            let tokens = imported.tokens();
            if preferences.theme_filter.includes(tokens.is_dark)
                && card(ui, &imported.name, tokens, appearance.selected == Selection::Imported)
            {
                Arc::make_mut(appearance).selected = Selection::Imported;
            }
        }
    });
}

fn card(ui: &mut Ui, label: &str, tokens: Tokens, selected: bool) -> bool {
    ui.push_id(tokens.id, |ui| {
        let response = egui::Frame::new()
            .fill(tokens.palette.panel)
            .stroke(if selected { Stroke::new(2.0, theme::accent()) } else { Stroke::new(1.0, theme::line()) })
            .corner_radius(theme::radii().control)
            .inner_margin(egui::Margin::same(7))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(121.0);
                    ui.set_max_width(121.0);
                    ui.horizontal(|ui| {
                        for color in [tokens.palette.chrome, tokens.palette.panel_header, tokens.palette.accent] {
                            let (rect, _) = ui.allocate_exact_size(vec2(24.0, 12.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 2.0, color);
                        }
                        if selected {
                            ui.label(RichText::new("✓").color(tokens.palette.ink));
                        }
                    });
                    ui.add(egui::Label::new(RichText::new(label).color(tokens.palette.ink)).truncate());
                });
            })
            .response
            .interact(egui::Sense::click())
            .on_hover_text(label);
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label));
        register(&response, &format!("theme-{}", tokens.id));
        response.clicked()
    })
    .inner
}

fn background(preferences: &mut Preferences, ui: &mut Ui, appearance: &mut Arc<Appearance>) {
    ui.horizontal_wrapped(|ui| {
        let mut background = appearance.background;
        for (choice, label) in [
            (Background::Gradient, "Theme gradient"),
            (Background::Color, "Colour"),
            (Background::Scene, "Viewport scene"),
        ] {
            let response = ui.selectable_value(&mut background, choice, label);
            register(&response, &format!("theme-background-{label}"));
        }
        if appearance.picture_name().is_some() {
            let response = ui.selectable_value(&mut background, Background::Picture, "Picture");
            register(&response, "theme-background-Picture");
        }
        if background != appearance.background {
            Arc::make_mut(appearance).background = background;
        }
        let choose = ui.button("Choose PNG…");
        register(&choose, "theme-picture-import");
        if choose.clicked() {
            pick(preferences, appearance, false);
        }
    });
    if appearance.background == Background::Color {
        let mut color = appearance.background_color.0;
        ui.horizontal(|ui| {
            ui.label("Background colour");
            if ui.color_edit_button_srgb(&mut color).changed() {
                Arc::make_mut(appearance).background_color = Rgb(color);
            }
        });
    }
    if let Some(name) = appearance.picture_name() {
        let remove = ui.horizontal(|ui| {
            ui.label(egui::RichText::new(name).weak());
            ui.button("Remove picture").clicked()
        });
        if remove.inner {
            Arc::make_mut(appearance).clear_picture();
        }
    }
}

fn optional_color(ui: &mut Ui, label: &str, setting: &mut Option<Rgb>, theme_color: Color32) {
    ui.push_id(label, |ui| {
        ui.horizontal(|ui| {
            ui.add_sized(vec2(130.0, 24.0), egui::Label::new(label));
            let mut rgb = setting.unwrap_or_else(|| theme_color.into()).0;
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                *setting = Some(Rgb(rgb));
            }
            if ui.add_enabled(setting.is_some(), egui::Button::new("Use default")).clicked() {
                *setting = None;
            }
        });
    });
}

fn control_heading(preferences: &Preferences, ui: &mut Ui, id: &str, title: &str) {
    let color = if preferences.highlight.as_deref() == Some(id) { theme::accent() } else { theme::ink() };
    ui.label(RichText::new(title).strong().color(color));
}

fn register(response: &egui::Response, name: &str) {
    #[cfg(target_arch = "wasm32")]
    crate::web::show_control(name, response.rect);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (response, name);
}

fn pick(preferences: &mut Preferences, appearance: &mut Arc<Appearance>, theme: bool) {
    static REQUEST: AtomicU32 = AtomicU32::new(0);
    let id = REQUEST.fetch_add(1, Ordering::Relaxed);
    let purpose = if theme { Purpose::Theme(id) } else { Purpose::ViewportPicture(id) };
    preferences.appearance_picker = Some(purpose);
    if let Some(result) = platform::open_appearance_file(purpose) {
        preferences.appearance_picker = None;
        let outcome = result.and_then(|(name, bytes)| import(appearance, purpose, name, &bytes));
        if outcome.is_ok() {
            preferences.theme_filter = Filter::All;
        }
        preferences.appearance_import_message(outcome);
    }
}

fn import(appearance: &mut Arc<Appearance>, purpose: Purpose, name: String, bytes: &[u8]) -> Result<(), String> {
    match purpose {
        Purpose::Theme(_) => {
            let imported = theme::ThemeFile::read(bytes)?;
            let appearance = Arc::make_mut(appearance);
            appearance.imported = Some(imported);
            appearance.selected = Selection::Imported;
            Ok(())
        }
        Purpose::ViewportPicture(_) => Arc::make_mut(appearance).set_picture(name, bytes),
        _ => unreachable!("the Appearance picker accepts theme files and pictures"),
    }
}

impl Preferences {
    pub fn with_appearance(mut self, appearance: Arc<Appearance>) -> Self {
        self.initial_appearance = appearance.clone();
        self.appearance = appearance;
        self
    }

    pub(super) fn edit_appearance(&mut self, appearance: Arc<Appearance>) {
        if !Arc::ptr_eq(&self.appearance, &appearance) && *self.appearance != *appearance {
            self.appearance = appearance.clone();
            self.changes.push(Change::Appearance { appearance, keep: false });
        }
    }

    fn appearance_import_message(&mut self, result: Result<(), String>) {
        self.message = result.err().map(|text| Message { text, error: true });
    }

    pub fn appearance_upload(&mut self, uploaded: Uploaded) {
        if self.appearance_picker != Some(uploaded.purpose) {
            return;
        }
        self.appearance_picker = None;
        let mut appearance = self.appearance.clone();
        let outcome = if let Some(error) = uploaded.error {
            Err(error)
        } else if let Some((name, bytes)) = uploaded.files.into_iter().next() {
            import(&mut appearance, uploaded.purpose, name, &bytes)
        } else {
            return;
        };
        self.appearance_import_message(outcome);
        self.edit_appearance(appearance);
        self.theme_filter = Filter::All;
    }
}
