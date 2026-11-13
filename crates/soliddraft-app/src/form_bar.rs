//! Shared layout for an editing form: primary fields and actions on the bar, the rest on a drop sheet.

use crate::{
    icons::{self, Icon},
    theme,
};
use egui::{Align, Align2, CornerRadius, Id, Rect, Sense, Stroke, pos2, vec2};
use std::hash::Hash;

const GAP: f32 = 8.0;
const MORE_WIDTH: f32 = 86.0;
const FIELD_HEIGHT: f32 = 30.0;
const UNIT_WIDTH: f32 = 30.0;

/// A framed value cell: the widget fills it, the unit sits at its right. The frame
/// follows the widget: the accent while it has the keyboard, a stronger line on hover.
pub(crate) fn value_cell(
    ui: &mut egui::Ui,
    cell: Rect,
    unit: &str,
    add: impl FnOnce(&mut egui::Ui, Rect) -> egui::Response,
) -> egui::Response {
    let frame = ui.painter().add(egui::Shape::Noop);
    // Every value keeps the unit's room, with a unit or without, so the column stays one width.
    let inner = Rect::from_min_max(
        pos2(cell.left() + 2.0, cell.top() + 1.0),
        pos2(cell.right() - UNIT_WIDTH, cell.bottom() - 1.0),
    );
    let response = ui
        .scope(|ui| {
            let v = ui.visuals_mut();
            for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
                w.bg_fill = egui::Color32::TRANSPARENT;
                w.weak_bg_fill = egui::Color32::TRANSPARENT;
                w.bg_stroke = Stroke::NONE;
                w.expansion = 0.0;
            }
            v.selection.stroke = Stroke::NONE;
            ui.spacing_mut().interact_size = inner.size();
            ui.spacing_mut().button_padding = vec2(6.0, 2.0);
            add(ui, inner)
        })
        .inner;
    let (fill, stroke) = if response.has_focus() {
        (theme::chip(), Stroke::new(1.5, theme::accent()))
    } else if response.hovered() {
        (theme::field_bg(), Stroke::new(1.0, theme::line_strong()))
    } else {
        (theme::field_bg(), Stroke::new(1.0, theme::line()))
    };
    ui.painter()
        .set(frame, egui::epaint::RectShape::new(cell, CornerRadius::same(6), fill, stroke, egui::StrokeKind::Inside));
    if !unit.is_empty() {
        ui.painter().text(
            pos2(cell.right() - 8.0, cell.center().y),
            Align2::RIGHT_CENTER,
            unit,
            theme::font(11.0),
            theme::muted(),
        );
    }
    response
}

pub struct Field<K> {
    pub id: K,
    pub label: String,
    pub group: &'static str,
    pub width: f32,
    pub primary: bool,
    pub count: usize,
}

impl<K> Field<K> {
    pub fn new(id: K, label: impl Into<String>, group: &'static str, width: f32) -> Self {
        Self { id, label: label.into(), group, width, primary: true, count: 1 }
    }

    pub fn sheet(mut self) -> Self {
        self.primary = false;
        self
    }

    pub fn count(mut self, count: usize) -> Self {
        self.count = count;
        self
    }
}

#[derive(Default)]
pub struct State {
    key: Option<Id>,
    pub expanded: bool,
}

impl State {
    pub fn reset(&mut self) {
        self.key = None;
        self.expanded = false;
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub bar: Vec<usize>,
    pub sheet: Vec<usize>,
}

pub fn plan<K>(fields: &[Field<K>], width: f32) -> Plan {
    let primary_width: f32 = fields.iter().filter(|f| f.primary).map(|f| f.width + GAP).sum();
    let fits = fields.iter().all(|f| f.primary) && primary_width <= width;
    let mut room = (width - if fits { 0.0 } else { MORE_WIDTH + GAP }).max(0.0);
    let mut plan = Plan { bar: Vec::new(), sheet: Vec::new() };
    for (index, field) in fields.iter().enumerate() {
        if field.primary && field.width + GAP <= room {
            plan.bar.push(index);
            room -= field.width + GAP;
        } else {
            plan.sheet.push(index);
        }
    }
    plan
}

pub struct Form<'a, K> {
    pub key: Id,
    pub title: &'a str,
    pub icon: Icon,
    pub fields: &'a [Field<K>],
    pub can_confirm: bool,
    pub error: Option<&'a str>,
}

#[derive(Default)]
pub struct Response {
    pub confirm: bool,
    pub cancel: bool,
}

/// The field renderer returns true when a summary field asks to open its details sheet.
pub fn show<K: Copy + Eq + Hash>(
    ui: &mut egui::Ui,
    state: &mut State,
    form: Form<'_, K>,
    mut render: impl FnMut(&mut egui::Ui, K) -> bool,
) -> Response {
    if state.key != Some(form.key) {
        state.reset();
        state.key = Some(form.key);
    }
    let available = ui.available_rect_before_wrap();
    let title_width = (available.width() * 0.2).clamp(110.0, 226.0);
    let actions_width = if form.can_confirm { 154.0 } else { 76.0 };
    let plan = plan(form.fields, available.width() - title_width - actions_width - 2.0 * GAP);
    let mut anchor = Rect::NOTHING;
    let mut open_details = false;
    let mut response = Response::default();
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        let (title, title_response) = ui.allocate_exact_size(vec2(title_width, FIELD_HEIGHT), Sense::hover());
        ui.painter().rect_filled(title, theme::RADIUS, theme::panel());
        icons::paint(
            ui.painter(),
            form.icon,
            Rect::from_min_size(title.min + vec2(8.0, 6.0), vec2(18.0, 18.0)),
            theme::accent(),
        );
        let mut job = egui::text::LayoutJob::simple(
            form.title.into(),
            theme::font_medium(12.0),
            theme::ink(),
            title.width() - 42.0,
        );
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let text = ui.painter().layout_job(job);
        ui.painter().galley(title.min + vec2(34.0, (title.height() - text.size().y) / 2.0), text, theme::ink());
        title_response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("Active tool: {}", form.title))
        });
        title_response.on_hover_text(form.title);
        for &index in &plan.bar {
            let field = &form.fields[index];
            let (rect, _) = ui.allocate_exact_size(vec2(field.width, FIELD_HEIGHT), Sense::hover());
            ui.painter().rect_filled(rect, theme::RADIUS, theme::panel());
            ui.scope_builder(
                egui::UiBuilder::new()
                    .id_salt((form.key, field.id))
                    .max_rect(rect.shrink2(vec2(8.0, 2.0)))
                    .layout(egui::Layout::left_to_right(Align::Center)),
                |ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(rect));
                    if !field.label.is_empty() {
                        let caption_width = ui
                            .painter()
                            .layout_no_wrap(field.label.clone(), theme::font(11.5), theme::muted())
                            .size()
                            .x
                            .min(field.width * 0.5);
                        ui.allocate_ui_with_layout(
                            vec2(caption_width, 24.0),
                            egui::Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.add(
                                    egui::Label::new(egui::RichText::new(&field.label).color(theme::muted()))
                                        .truncate(),
                                )
                                .on_hover_text(&field.label);
                            },
                        );
                    }
                    open_details |= render(ui, field.id);
                },
            );
        }
        if !plan.sheet.is_empty() {
            let count: usize = plan.sheet.iter().map(|&i| form.fields[i].count).sum();
            let more = ui.add_sized(
                vec2(MORE_WIDTH, FIELD_HEIGHT),
                egui::Button::new(format!("More ({count})")).selected(state.expanded),
            );
            anchor = more.rect;
            if more.clicked() {
                state.expanded = !state.expanded;
            }
            more.on_hover_text("The rest of this form");
        }
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), FIELD_HEIGHT),
            egui::Layout::right_to_left(Align::Center),
            |ui| {
                if form.can_confirm {
                    response.confirm = ui
                        .add_sized(
                            vec2(70.0, FIELD_HEIGHT),
                            egui::Button::new(egui::RichText::new("OK").color(theme::on_accent()))
                                .fill(theme::accent()),
                        )
                        .on_hover_text("OK (Enter)")
                        .clicked();
                }
                response.cancel = ui
                    .add_sized(vec2(70.0, FIELD_HEIGHT), egui::Button::new("Cancel"))
                    .on_hover_text("Cancel (Esc)")
                    .clicked();
            },
        );
    });
    if response.cancel {
        state.reset();
    } else if open_details && !plan.sheet.is_empty() {
        state.expanded = true;
    }
    if state.expanded && !plan.sheet.is_empty() {
        let mut groups = Vec::new();
        for &index in &plan.sheet {
            let group = form.fields[index].group;
            if !groups.contains(&group) {
                groups.push(group);
            }
        }
        let screen = ui.ctx().input(|i| i.content_rect());
        let columns = groups.len().min(((screen.width() - 32.0) / 280.0) as usize).clamp(1, 3);
        let width = (columns as f32 * 280.0 + 24.0).min(screen.width() - 16.0);
        let left = anchor.left().clamp(screen.left() + 8.0, (screen.right() - width - 8.0).max(screen.left() + 8.0));
        egui::Area::new(form.key.with("sheet"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(left, available.bottom() + 6.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(theme::panel())
                    .stroke(egui::Stroke::new(1.0, theme::line()))
                    .corner_radius(theme::RADIUS)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.set_width(width - 24.0);
                        egui::ScrollArea::vertical()
                            .id_salt(form.key.with("sheet_scroll"))
                            .max_height((screen.bottom() - available.bottom() - 32.0).max(80.0))
                            .show(ui, |ui| {
                                if let Some(error) = form.error {
                                    ui.colored_label(theme::error(), error);
                                }
                                ui.columns(columns, |uis| {
                                    for (group_index, group) in groups.iter().enumerate() {
                                        let ui = &mut uis[group_index % columns];
                                        ui.add_space(4.0);
                                        ui.label(egui::RichText::new(*group).font(theme::font_medium(11.5)));
                                        ui.add_space(6.0);
                                        for &index in &plan.sheet {
                                            let field = &form.fields[index];
                                            if field.group != *group {
                                                continue;
                                            }
                                            ui.push_id((form.key, field.id), |ui| {
                                                if field.label.is_empty() {
                                                    render(ui, field.id);
                                                } else {
                                                    ui.horizontal(|ui| {
                                                        ui.allocate_ui_with_layout(
                                                            vec2(96.0, 24.0),
                                                            egui::Layout::left_to_right(Align::Center),
                                                            |ui| {
                                                                ui.add(egui::Label::new(&field.label).truncate())
                                                                    .on_hover_text(&field.label);
                                                            },
                                                        );
                                                        render(ui, field.id);
                                                    });
                                                }
                                            });
                                            ui.add_space(6.0);
                                        }
                                        ui.add_space(8.0);
                                    }
                                });
                            });
                    });
            });
    } else if let Some(error) = form.error {
        egui::Area::new(form.key.with("error"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(available.left() + title_width + GAP, available.bottom() + 4.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(theme::panel())
                    .stroke(egui::Stroke::new(1.0, theme::error()))
                    .corner_radius(theme::RADIUS)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui.set_max_width((available.width() - title_width - 24.0).min(680.0));
                        ui.colored_label(theme::error(), error);
                    });
            });
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_spill_without_losing_their_order_or_finish_actions() {
        let fields =
            [Field::new(0, "A", "Main", 100.0), Field::new(1, "B", "Main", 120.0), Field::new(2, "C", "Main", 80.0)];
        assert_eq!(plan(&fields, 324.0), Plan { bar: vec![0, 1, 2], sheet: vec![] });
        assert_eq!(plan(&fields, 310.0), Plan { bar: vec![0, 2], sheet: vec![1] });
        assert_eq!(plan(&fields, 0.0), Plan { bar: vec![], sheet: vec![0, 1, 2] });
        let fields = [
            Field::new(0, "Pick", "Main", 140.0),
            Field::new(1, "Size", "Main", 120.0),
            Field::new(2, "Extra", "More", 100.0).sheet(),
        ];
        assert_eq!(plan(&fields, 500.0), Plan { bar: vec![0, 1], sheet: vec![2] });
    }
}
