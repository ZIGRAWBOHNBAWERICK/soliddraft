//! Material library browser and custom material editor.

use soliddraft_document::DocumentId;
use soliddraft_materials::{Appearance, Catalog, Material, MaterialId, Origin, Property, Srgb};

use crate::document::Document;
use crate::settings;
use crate::state::{AppState, NoteKind};

pub struct Dialog {
    document: DocumentId,
    body: Option<u32>,
    catalog: Catalog,
    selected: Option<MaterialId>,
    search: String,
    draft: Option<Draft>,
    error: Option<String>,
}

struct Draft {
    material: Material,
    fields: Vec<String>,
    colour: [u8; 3],
    original_appearance: Option<Appearance>,
}

impl Draft {
    fn new(mut material: Material, original_appearance: Option<Appearance>, copy: bool) -> Self {
        if copy {
            material.id = MaterialId(format!("user-{}", DocumentId::new()));
            material.name.push_str(" (custom)");
            material.category = "Custom".into();
        }
        let colour = original_appearance.as_ref().map_or(Appearance::default_part().base_color, |a| a.base_color);
        let fields =
            Property::ALL.iter().map(|p| material.value(*p).map_or(String::new(), |v| v.to_string())).collect();
        Self { material, fields, colour: [colour.r, colour.g, colour.b], original_appearance }
    }

    fn save(&self) -> Result<MaterialId, String> {
        let mut material = self.material.clone();
        for (property, text) in Property::ALL.into_iter().zip(&self.fields) {
            let text = text.trim();
            let value = if text.is_empty() {
                None
            } else {
                Some(text.parse::<f64>().map_err(|_| format!("Enter a number for {}", property.label()))?)
            };
            if value != material.value(property) {
                material.set_override(property, value, None);
            }
        }
        let colour = Srgb::new(self.colour[0], self.colour[1], self.colour[2]);
        let appearance = if let Some(original) = &self.original_appearance
            && original.base_color == colour
        {
            original.clone()
        } else {
            Appearance {
                base_color: colour,
                id: soliddraft_materials::AppearanceId(format!("{}-{colour}", material.id)),
                name: format!("{} colour", material.name),
                ..self.original_appearance.clone().unwrap_or_else(Appearance::default_part)
            }
        };
        material.default_appearance = Some(appearance.id.clone());
        if let Some(problem) = material.problems().first() {
            return Err(problem.to_string());
        }
        let mut library = settings::user_library()?;
        let id = material.id.clone();
        library.put_appearance(appearance);
        library.put_material(material);
        settings::save_material_library(&library)?;
        Ok(id)
    }
}

impl Dialog {
    pub fn new(doc: &Document, body: Option<u32>) -> Self {
        let configuration = doc.model().active_configuration();
        let material = match body {
            Some(body) => doc.model().body_material(body, configuration),
            None => doc.material(),
        };
        Self {
            document: doc.instance(),
            body,
            catalog: settings::material_catalog(),
            selected: material.map(|m| m.id.clone()),
            search: String::new(),
            draft: None,
            error: None,
        }
    }
}

pub(crate) fn show(ctx: &egui::Context, state: &mut AppState) {
    let Some(mut dialog) = state.material_dialog.take() else { return };
    if state.doc.instance() != dialog.document {
        return;
    }
    let title = dialog.body.map_or("Part material".into(), |body| format!("{} material", state.doc.body_name(body)));
    let mut open = true;
    let mut close = false;
    egui::Window::new(title).id(egui::Id::new("material_library")).open(&mut open).default_width(470.0).show(
        ctx,
        |ui| {
            if let Some(draft) = &mut dialog.draft {
                ui.label("Custom material");
                ui.text_edit_singleline(&mut draft.material.name).labelled_by(ui.label("Name").id);
                ui.text_edit_multiline(&mut draft.material.description).labelled_by(ui.label("Description").id);
                let source = draft.material.source.get_or_insert_with(String::new);
                ui.text_edit_singleline(source).labelled_by(ui.label("Source").id);
                ui.horizontal(|ui| {
                    ui.label("Default colour");
                    egui::color_picker::color_edit_button_srgb(ui, &mut draft.colour);
                });
                egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                    egui::Grid::new("edit_material_properties").num_columns(2).show(ui, |ui| {
                        for (property, text) in Property::ALL.into_iter().zip(&mut draft.fields) {
                            let label = ui.label(property_label(property));
                            ui.add(egui::TextEdit::singleline(text).hint_text("Unknown").desired_width(120.0))
                                .labelled_by(label.id);
                            ui.end_row();
                        }
                    });
                });
                ui.horizontal(|ui| {
                    if ui.button("Save to my library").clicked() {
                        match draft.save() {
                            Ok(id) => {
                                dialog.catalog = settings::material_catalog();
                                dialog.selected = Some(id);
                                dialog.error = None;
                                close = true;
                            }
                            Err(error) => dialog.error = Some(error),
                        }
                    }
                    if ui.button("Cancel edit").clicked() {
                        close = true;
                        dialog.error = None;
                    }
                });
                if close {
                    close = false;
                    dialog.draft = None;
                }
            } else {
                ui.add(egui::TextEdit::singleline(&mut dialog.search).hint_text("Find a material"));
                let search = dialog.search.to_lowercase();
                egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                    for (library, material) in dialog.catalog.materials() {
                        if !format!("{} {} {}", material.name, material.category, library.name)
                            .to_lowercase()
                            .contains(&search)
                        {
                            continue;
                        }
                        let text = format!("{} · {}", material.name, material.category);
                        ui.selectable_value(&mut dialog.selected, Some(material.id.clone()), text);
                    }
                });
                let selected = dialog.selected.as_ref().and_then(|id| dialog.catalog.material(id));
                if let Some((library, material)) = selected {
                    let appearance = dialog.catalog.default_appearance_of(material).cloned();
                    ui.separator();
                    ui.strong(&material.name);
                    if !material.description.is_empty() {
                        ui.label(&material.description);
                    }
                    if let Some(source) = &material.source {
                        ui.collapsing("Source", |ui| {
                            ui.label(source);
                        });
                    }
                    if library.origin == Origin::BuiltIn {
                        let handbook = material.source.as_deref().is_some_and(|source| source.contains("MIL-HDBK-5J"));
                        ui.label(if handbook {
                            "Handbook reference values. See the source for the strength basis."
                        } else {
                            "Typical reference values"
                        });
                    }
                    if let Some(appearance) = &appearance {
                        let c = appearance.base_color;
                        ui.colored_label(egui::Color32::from_rgb(c.r, c.g, c.b), format!("●  Default colour  {c}"));
                    }
                    egui::ScrollArea::vertical().max_height(235.0).show(ui, |ui| {
                        egui::Grid::new("material_properties").num_columns(2).show(ui, |ui| {
                            for property in Property::ALL {
                                ui.label(property_label(property));
                                ui.label(material.value(property).map_or("Unknown".into(), |v| v.to_string()));
                                ui.end_row();
                            }
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Assign material").clicked() {
                            match state.doc.set_material_for(dialog.body, Some(material.clone()), appearance.clone()) {
                                Ok(()) => close = true,
                                Err(error) => dialog.error = Some(error),
                            }
                        }
                        let custom = library.origin == Origin::User;
                        if ui.button(if custom { "Edit custom material" } else { "Copy to my library" }).clicked() {
                            dialog.draft = Some(Draft::new(material.clone(), appearance, !custom));
                            dialog.error = None;
                        }
                    });
                } else if let Some(material) = state.doc.material() {
                    ui.label(format!("{} is stored in this part", material.name));
                }
                if state.doc.material_assignment_for(dialog.body).is_some()
                    && ui
                        .button(if dialog.body.is_some() {
                            "Use inherited material"
                        } else {
                            "Clear material assignment"
                        })
                        .clicked()
                {
                    match state.doc.set_material_for(dialog.body, None, None) {
                        Ok(()) => close = true,
                        Err(error) => dialog.error = Some(error),
                    }
                }
            }
            if let Some(error) = &dialog.error {
                ui.colored_label(crate::theme::error(), error);
            }
        },
    );
    if open && !close {
        state.material_dialog = Some(dialog);
    } else if close {
        state.note("Material assignment updated", NoteKind::Info);
    }
}

fn property_label(property: Property) -> String {
    if property.unit().is_empty() {
        property.label().into()
    } else {
        format!("{} ({})", property.label(), property.unit())
    }
}
