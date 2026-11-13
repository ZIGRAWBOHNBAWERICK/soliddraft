//! Edit Suite navigation and scene favourites over the viewport.

use super::{ViewportEvents, camera::Camera, render::DisplayMode, scenes::Scene};
use crate::{
    icons::{self, Icon},
    settings::{self, ReferenceDisplay},
    state::AppAction,
    theme,
};
use egui::{Align2, Color32, CornerRadius, Rect, RectAlign, Sense, Stroke, StrokeKind, pos2, vec2};

pub struct NavigationBar {
    favourites: Vec<Scene>,
    manage: bool,
    error: Option<String>,
}

impl Default for NavigationBar {
    fn default() -> Self {
        Self { favourites: settings::scene_favourites(), manage: false, error: None }
    }
}

impl NavigationBar {
    pub fn consume_escape(&mut self, ctx: &egui::Context) {
        if self.manage && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.manage = false;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        parent: &mut egui::Ui,
        viewport: Rect,
        camera: &mut Camera,
        display: DisplayMode,
        scene: Scene,
        references: &mut ReferenceDisplay,
        all_dimensions: &mut bool,
        sketch_dimensions: Option<bool>,
        allow_dimensions: bool,
        section_active: bool,
        events: &mut ViewportEvents,
    ) {
        let compact = viewport.width() < 350.0;
        egui::Area::new(parent.id().with("viewport_navigation"))
            .order(egui::Order::Foreground)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(pos2(viewport.center().x, viewport.bottom() - 12.0))
            .show(parent.ctx(), |ui| {
                ui.set_max_width(if compact { 324.0 } else { 368.0 });
                egui::Frame::new()
                    .fill(theme::hud_bg())
                    .stroke(Stroke::new(1.0, theme::line_strong()))
                    .corner_radius(theme::edit_suite::FLOATING_RADIUS)
                    .shadow(theme::edit_suite::NAV_SHADOW)
                    .inner_margin(egui::Margin::symmetric(6, 4))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                        ui.horizontal(|ui| {
                            for (label, icon) in [("Orbit", Icon::Orbit), ("Pan", Icon::Pan), ("Zoom", Icon::Search)] {
                                let response = button(ui, label, icon, false, true, true);
                                if response.dragged() {
                                    let delta = ui.input(|i| i.pointer.delta());
                                    match label {
                                        "Orbit" => camera.orbit(delta.x, delta.y),
                                        "Pan" => camera.pan(delta.x, delta.y, viewport.height()),
                                        _ => camera.zoom((-delta.y * 0.01).exp(), None, viewport.aspect_ratio()),
                                    }
                                    ui.ctx().request_repaint();
                                }
                            }
                            if button(ui, "Zoom to fit", Icon::Fit, false, false, true).clicked() {
                                events.fit = true;
                            }
                            ui.separator();
                            let display_button = button(ui, "Display style", Icon::Display, false, false, true);
                            egui::Popup::menu(&display_button).align(RectAlign::TOP_END).show(|ui| {
                                for style in DisplayMode::ALL {
                                    if ui.selectable_label(display == style, style.label()).clicked() {
                                        events.set_display = Some(style);
                                        ui.close();
                                    }
                                }
                            });
                            let scene_button = if compact {
                                button(ui, "Scene", Icon::Material, false, false, true)
                            } else {
                                ui.add_sized(vec2(74.0, 28.0), egui::Button::new("Scene"))
                            };
                            scene_button.clone().on_hover_text("Scene");
                            egui::Popup::menu(&scene_button)
                                .align(RectAlign::TOP_END)
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                .show(|ui| self.gallery(ui, scene, events));
                            ui.separator();
                            if button(ui, "Section view", Icon::Section, section_active, false, allow_dimensions)
                                .clicked()
                            {
                                events.actions.push(AppAction::ToggleSection);
                            }
                            if button(ui, "Origin axes", Icon::Origin, references.always_show_origin_axes, false, true)
                                .clicked()
                            {
                                references.always_show_origin_axes = !references.always_show_origin_axes;
                                if let Err(error) = settings::save_reference_display(references) {
                                    self.error = Some(format!("Axes changed, but could not be saved: {error}"));
                                }
                            }
                            if button(
                                ui,
                                "All dimensions",
                                Icon::SmartDim,
                                sketch_dimensions.unwrap_or(*all_dimensions),
                                false,
                                allow_dimensions,
                            )
                            .clicked()
                            {
                                if let Some(shown) = sketch_dimensions {
                                    events.actions.push(AppAction::SketchDimensions(!shown));
                                } else {
                                    *all_dimensions = !*all_dimensions;
                                }
                            }
                        });
                    });
            });
        self.manage_window(parent.ctx());
    }

    fn gallery(&mut self, ui: &mut egui::Ui, current: Scene, events: &mut ViewportEvents) {
        ui.set_min_width(308.0);
        ui.label(egui::RichText::new("Scenes").font(theme::font_semibold(12.5)));
        if self.favourites.is_empty() {
            ui.label("Choose scenes in Manage favourites.");
        }
        egui::ScrollArea::vertical().max_height((ui.ctx().content_rect().height() - 160.0).clamp(78.0, 260.0)).show(
            ui,
            |ui| {
                egui::Grid::new("scene_thumbnails").num_columns(3).spacing(vec2(8.0, 8.0)).show(ui, |ui| {
                    for (i, scene) in self.favourites.iter().enumerate() {
                        if thumbnail(ui, *scene, current == *scene).clicked() {
                            events.actions.push(AppAction::SetScene(*scene));
                            ui.close();
                        }
                        if i % 3 == 2 {
                            ui.end_row();
                        }
                    }
                });
            },
        );
        ui.separator();
        if ui.button("Manage favourites").clicked() {
            self.manage = true;
            ui.close();
        }
        if let Some(error) = &self.error {
            ui.colored_label(theme::error(), error);
        }
    }

    fn manage_window(&mut self, ctx: &egui::Context) {
        if !self.manage {
            return;
        }
        let mut open = true;
        let mut done = false;
        egui::Window::new("Manage favourites").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
            ui.label("Choose the scenes shown in the gallery.");
            for scene in Scene::ALL {
                let mut chosen = self.favourites.contains(&scene);
                if ui.checkbox(&mut chosen, scene.label()).changed() {
                    if chosen {
                        self.favourites.push(scene);
                    } else {
                        self.favourites.retain(|s| *s != scene);
                    }
                    self.favourites = Scene::ALL.into_iter().filter(|s| self.favourites.contains(s)).collect();
                    self.error = settings::save_scene_favourites(&self.favourites)
                        .err()
                        .map(|error| format!("Favourites changed, but could not be saved: {error}"));
                }
            }
            if let Some(error) = &self.error {
                ui.colored_label(theme::error(), error);
            }
            done = ui.button("Done").clicked();
        });
        self.manage = open && !done;
    }
}

fn button(ui: &mut egui::Ui, label: &str, icon: Icon, active: bool, drag: bool, enabled: bool) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(vec2(30.0, 28.0), Sense::hover());
    let response = ui.interact(
        rect,
        ui.id().with(label),
        if !enabled {
            Sense::hover()
        } else if drag {
            Sense::click_and_drag()
        } else {
            Sense::click()
        },
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, active, label));
    if enabled && (active || response.hovered() || response.has_focus()) {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(6),
            if active { theme::accent_soft() } else { theme::hover() },
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.5, theme::accent()), StrokeKind::Inside);
    }
    icons::paint(
        ui.painter(),
        icon,
        rect.shrink(6.0),
        if !enabled {
            theme::muted()
        } else if active {
            theme::accent()
        } else {
            theme::ink()
        },
    );
    response.on_hover_text(if !enabled {
        "Coming soon".to_owned()
    } else if drag {
        format!("{label}: drag to move the view")
    } else {
        label.to_owned()
    })
}

fn thumbnail(ui: &mut egui::Ui, scene: Scene, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(96.0, 78.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, scene.label()));
    let image = Rect::from_min_size(rect.min, vec2(96.0, 52.0));
    let texture = preview_texture(ui.ctx(), scene);
    egui::Image::new((texture.id(), image.size())).corner_radius(6).paint_at(ui, image);
    let ink = if scene.is_dark() { Color32::WHITE } else { Color32::from_rgb(40, 45, 55) };
    icons::paint(ui.painter(), Icon::Home, Rect::from_center_size(image.center(), vec2(24.0, 24.0)), ink);
    ui.painter().rect_stroke(
        image,
        CornerRadius::same(6),
        Stroke::new(
            if selected { 2.0 } else { 1.0 },
            if selected || response.has_focus() { theme::accent() } else { theme::line_strong() },
        ),
        StrokeKind::Inside,
    );
    ui.painter().text(
        pos2(rect.center().x, image.bottom() + 5.0),
        Align2::CENTER_TOP,
        scene.label(),
        theme::font(10.5),
        theme::muted(),
    );
    response.on_hover_text(scene.label())
}

fn preview_texture(ctx: &egui::Context, scene: Scene) -> egui::TextureHandle {
    let key = egui::Id::new(("scene_thumbnail", scene.id()));
    if let Some(texture) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(key)) {
        return texture;
    }
    let bg = scene.backdrop();
    let mut pixels = Vec::with_capacity(96 * 52);
    for y in 0..52 {
        for x in 0..96 {
            let uv = vec2(x as f32 / 95.0, y as f32 / 51.0);
            let t = if bg.radial[2] > 0.0 {
                vec2((uv.x - bg.radial[0]) / bg.radial[2], (uv.y - bg.radial[1]) / bg.radial[3]).length()
            } else {
                uv.y
            };
            let mut color = egui::Rgba::from(bg.stops[0].0).to_array();
            for pair in bg.stops.windows(2) {
                let [a, b] = [pair[0], pair[1]];
                if b.1 > a.1 {
                    let blend = ((t - a.1) / (b.1 - a.1)).clamp(0.0, 1.0);
                    let next = egui::Rgba::from(b.0).to_array();
                    for (c, next) in color[..3].iter_mut().zip(next) {
                        *c += (next - *c) * blend;
                    }
                }
            }
            if bg.grid[0] > 0.0 {
                let spacing = bg.grid[0] / 4.0;
                if (x as f32 % spacing).min(y as f32 % spacing) < 1.0 {
                    for c in &mut color[..3] {
                        *c += (1.0 - *c) * bg.grid[1];
                    }
                }
            }
            pixels.push(Color32::from(egui::Rgba::from_rgb(color[0], color[1], color[2])));
        }
    }
    let texture = ctx.load_texture(
        format!("scene-{}", scene.id()),
        egui::ColorImage::new([96, 52], pixels),
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|data| data.insert_temp(key, texture.clone()));
    texture
}
