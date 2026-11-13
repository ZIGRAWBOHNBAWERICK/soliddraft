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
        events: &mut ViewportEvents,
    ) {
        let compact = viewport.width() < 350.0;
        egui::Area::new(parent.id().with("viewport_navigation"))
            .order(egui::Order::Foreground)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(pos2(viewport.center().x, viewport.bottom() - 12.0))
            .show(parent.ctx(), |ui| {
                egui::Frame::new()
                    .fill(theme::hud_bg())
                    .stroke(Stroke::new(1.0, theme::line_strong()))
                    .corner_radius(theme::edit_suite::FLOATING_RADIUS)
                    .inner_margin(egui::Margin::symmetric(6, 4))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                        ui.horizontal(|ui| {
                            for (label, icon) in [("Orbit", Icon::Orbit), ("Pan", Icon::Pan), ("Zoom", Icon::Search)] {
                                let response = button(ui, label, icon, false, true);
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
                            if button(ui, "Zoom to fit", Icon::Fit, false, false).clicked() {
                                events.fit = true;
                            }
                            ui.separator();
                            let display_button = button(ui, "Display style", Icon::Display, false, false);
                            egui::Popup::menu(&display_button).align(RectAlign::TOP_END).show(|ui| {
                                for style in DisplayMode::ALL {
                                    if ui.selectable_label(display == style, style.label()).clicked() {
                                        events.set_display = Some(style);
                                        ui.close();
                                    }
                                }
                            });
                            let scene_button = if compact {
                                button(ui, "Scene", Icon::Material, false, false)
                            } else {
                                ui.add_sized(vec2(74.0, 28.0), egui::Button::new("Scene"))
                            };
                            scene_button.clone().on_hover_text("Scene");
                            egui::Popup::menu(&scene_button)
                                .align(RectAlign::TOP_END)
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                .show(|ui| self.gallery(ui, scene, events));
                            ui.separator();
                            if button(ui, "Origin axes", Icon::Origin, references.always_show_origin_axes, false)
                                .clicked()
                            {
                                references.always_show_origin_axes = !references.always_show_origin_axes;
                                if let Err(error) = settings::save_reference_display(references) {
                                    self.error = Some(format!("Axes changed, but could not be saved: {error}"));
                                }
                            }
                            if button(ui, "All dimensions", Icon::SmartDim, *all_dimensions, false).clicked() {
                                *all_dimensions = !*all_dimensions;
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

fn button(ui: &mut egui::Ui, label: &str, icon: Icon, active: bool, drag: bool) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(vec2(30.0, 28.0), Sense::hover());
    let response = ui.interact(rect, ui.id().with(label), if drag { Sense::click_and_drag() } else { Sense::click() });
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, label));
    if active || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(6),
            if active { theme::accent_soft() } else { theme::hover() },
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.5, theme::accent()), StrokeKind::Inside);
    }
    icons::paint(ui.painter(), icon, rect.shrink(6.0), if active { theme::accent() } else { theme::ink() });
    response.on_hover_text(if drag { format!("{label}: drag to move the view") } else { label.to_owned() })
}

fn thumbnail(ui: &mut egui::Ui, scene: Scene, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(96.0, 78.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, scene.label()));
    let image = Rect::from_min_size(rect.min, vec2(96.0, 52.0));
    let backdrop = scene.backdrop();
    for row in 0..52 {
        let y = row as f32 / 51.0;
        let [a, b] = backdrop
            .stops
            .windows(2)
            .find(|pair| y <= pair[1].1)
            .map(|p| [p[0], p[1]])
            .unwrap_or([backdrop.stops[3], backdrop.stops[4]]);
        let t = ((y - a.1) / (b.1 - a.1).max(0.0001)).clamp(0.0, 1.0);
        let channel = |x: u8, z: u8| (f32::from(x) + (f32::from(z) - f32::from(x)) * t).round() as u8;
        let color = Color32::from_rgb(channel(a.0.r(), b.0.r()), channel(a.0.g(), b.0.g()), channel(a.0.b(), b.0.b()));
        ui.painter().rect_filled(Rect::from_min_size(image.min + vec2(0.0, row as f32), vec2(96.0, 1.0)), 0, color);
    }
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
