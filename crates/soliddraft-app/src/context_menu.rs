//! Edit Suite right-click targets, their shared commands, and the ring's registry adapter.

use crate::commands::{self, Context};
use crate::document::{Document, Selection};
use crate::ring_menu;
use crate::settings::FrameLayout;
use crate::state::AppState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Face,
    Edge,
    Feature,
    Body,
    Empty,
    SketchLine,
    EmptySketch,
}

impl Target {
    pub fn of(selection: Option<Selection>) -> Self {
        match selection {
            Some(Selection::Face(_)) => Self::Face,
            Some(Selection::Edge(_)) => Self::Edge,
            Some(Selection::Feature(_)) => Self::Feature,
            Some(Selection::Body | Selection::Vertex(_)) => Self::Body,
            _ => Self::Empty,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Group {
    pub target: Target,
    pub label: &'static str,
    pub commands: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuickCommand {
    pub target: Target,
    pub command: &'static str,
    pub label: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct MenuSpec {
    pub groups: &'static [Group],
    pub quick: [QuickCommand; 8],
}

const fn q(target: Target, command: &'static str, label: &'static str) -> QuickCommand {
    QuickCommand { target, command, label }
}
const FACE: Group = Group {
    target: Target::Face,
    label: "Face",
    commands: &["sketch", "normal_to", "measure", "reference_plane", "select_loop", "appearance"],
};
const EDGE: Group = Group {
    target: Target::Edge,
    label: "Edge",
    commands: &["fillet", "chamfer", "measure", "select_tangent_chain", "select_loop", "convert_entities"],
};
const FEATURE: Group = Group {
    target: Target::Feature,
    label: "Feature",
    commands: &[
        "edit_feature",
        "replace_import",
        "edit_sketch",
        "repair_feature",
        "suppress",
        "rollback_here",
        "rename",
        "delete_feature",
    ],
};
const BODY: Group = Group {
    target: Target::Body,
    label: "Body",
    commands: &["hide_body", "isolate", "show_dimensions", "mass_properties", "export_step", "material"],
};
const VIEW: Group = Group {
    target: Target::Empty,
    label: "View",
    commands: &["zoom_fit", "view_isometric", "view_front", "view_top", "view_right"],
};
const SCENE: Group = Group {
    target: Target::Empty,
    label: "Scene",
    commands: &["scene_white", "scene_studio", "scene_faded", "scene_spot", "scene_graphite"],
};
const DISPLAY: Group = Group {
    target: Target::Empty,
    label: "Display",
    commands: &["display_shaded_edges", "display_shaded", "display_wireframe"],
};
const SHOW: Group = Group {
    target: Target::Empty,
    label: "Show",
    commands: &["origin_axes", "origin_planes", "show_dimensions", "show_all_bodies"],
};
const LINE: Group = Group {
    target: Target::SketchLine,
    label: "Line",
    commands: &["smart_dimension", "sketch_vertical", "sketch_horizontal", "sketch_fix", "construction", "delete"],
};
const SKETCH: Group = Group {
    target: Target::EmptySketch,
    label: "Sketch",
    commands: &["exit_sketch", "normal_to", "sketch_dimensions", "zoom_fit"],
};
const SKETCH_TOOLS: Group = Group {
    target: Target::EmptySketch,
    label: "Sketch tools",
    commands: &["sketch_select", "sketch_line", "sketch_rectangle", "sketch_circle", "smart_dimension", "sketch_trim"],
};

/// One command model for every click target, shared by the shortcuts and columns.
pub static MENUS: [(Target, MenuSpec); 7] = [
    (
        Target::Face,
        MenuSpec {
            groups: &[FACE, FEATURE, BODY],
            quick: [
                q(Target::Face, "sketch", "Sketch"),
                q(Target::Face, "measure", ""),
                q(Target::Feature, "edit_feature", ""),
                q(Target::Body, "hide_body", "Hide"),
                q(Target::Face, "normal_to", ""),
                q(Target::Body, "isolate", ""),
                q(Target::Feature, "edit_sketch", ""),
                q(Target::Face, "select_other", ""),
            ],
        },
    ),
    (
        Target::Edge,
        MenuSpec {
            groups: &[EDGE, FEATURE, BODY],
            quick: [
                q(Target::Edge, "fillet", ""),
                q(Target::Edge, "measure", ""),
                q(Target::Edge, "chamfer", ""),
                q(Target::Body, "hide_body", "Hide"),
                q(Target::Edge, "select_tangent_chain", "Tangent chain"),
                q(Target::Body, "isolate", ""),
                q(Target::Feature, "edit_feature", ""),
                q(Target::Edge, "select_other", ""),
            ],
        },
    ),
    (
        Target::Feature,
        MenuSpec {
            groups: &[FEATURE, BODY],
            quick: [
                q(Target::Feature, "edit_feature", ""),
                q(Target::Feature, "suppress", ""),
                q(Target::Feature, "edit_sketch", ""),
                q(Target::Feature, "delete_feature", "Delete"),
                q(Target::Feature, "rollback_here", "Roll back"),
                q(Target::Body, "hide_body", "Hide"),
                q(Target::Feature, "rename", ""),
                q(Target::Feature, "repair_feature", "Repair"),
            ],
        },
    ),
    (
        Target::Body,
        MenuSpec {
            groups: &[BODY],
            quick: [
                q(Target::Body, "hide_body", "Hide"),
                q(Target::Body, "isolate", ""),
                q(Target::Body, "show_dimensions", "Dimensions"),
                q(Target::Body, "mass_properties", "Mass properties"),
                q(Target::Body, "export_step", "Export STEP"),
                q(Target::Body, "material", ""),
                q(Target::Body, "appearance", ""),
                q(Target::Body, "zoom_fit", "Zoom to fit"),
            ],
        },
    ),
    (
        Target::Empty,
        MenuSpec {
            groups: &[VIEW, SCENE, DISPLAY, SHOW],
            quick: [
                q(Target::Empty, "zoom_fit", "Zoom to fit"),
                q(Target::Empty, "sketch", "New sketch"),
                q(Target::Empty, "view_isometric", "Isometric"),
                q(Target::Empty, "ai", "AI session"),
                q(Target::Empty, "display_wireframe", "Wireframe"),
                q(Target::Empty, "measure", ""),
                q(Target::Empty, "view_front", "Front view"),
                q(Target::Empty, "origin_axes", ""),
            ],
        },
    ),
    (
        Target::SketchLine,
        MenuSpec {
            groups: &[LINE, SKETCH],
            quick: [
                q(Target::SketchLine, "smart_dimension", "Smart Dimension"),
                q(Target::SketchLine, "sketch_vertical", ""),
                q(Target::SketchLine, "sketch_trim", ""),
                q(Target::EmptySketch, "exit_sketch", "Close sketch"),
                q(Target::SketchLine, "construction", ""),
                q(Target::SketchLine, "sketch_horizontal", ""),
                q(Target::SketchLine, "sketch_offset", ""),
                q(Target::SketchLine, "sketch_fix", ""),
            ],
        },
    ),
    (
        Target::EmptySketch,
        MenuSpec {
            groups: &[SKETCH_TOOLS, SKETCH, SCENE],
            quick: [
                q(Target::EmptySketch, "sketch_line", ""),
                q(Target::EmptySketch, "smart_dimension", "Smart Dimension"),
                q(Target::EmptySketch, "sketch_rectangle", ""),
                q(Target::EmptySketch, "exit_sketch", "Close sketch"),
                q(Target::EmptySketch, "sketch_circle", ""),
                q(Target::EmptySketch, "sketch_trim", ""),
                q(Target::EmptySketch, "sketch_arc", ""),
                q(Target::EmptySketch, "sketch_select", ""),
            ],
        },
    ),
];

pub fn spec(target: Target) -> &'static MenuSpec {
    &MENUS.iter().find(|(key, _)| *key == target).expect("every target has a menu").1
}

pub fn groups(target: Target) -> &'static [Group] {
    spec(target).groups
}

fn selection_for(doc: &Document, target: Target, clicked: Option<Selection>) -> Option<Selection> {
    match target {
        Target::Feature => clicked.and_then(|s| doc.selection_feature(s)).map(Selection::Feature),
        Target::Body => doc.display_body().map(|_| Selection::Body),
        _ => clicked,
    }
}

fn sketch_caption(state: &AppState) -> String {
    use crate::sketch::EntityKind;
    use crate::state::SketchSel;
    let Some(sketch) = state.sketch.as_ref() else { return "Sketch".into() };
    let id = match sketch.selection.first() {
        Some(SketchSel::Entity(id)) => *id,
        Some(SketchSel::Point(point)) => point.entity,
        Some(SketchSel::Constraint(id)) => {
            return sketch.sketch.constraint(*id).map(|c| c.name().to_owned()).unwrap_or_default();
        }
        None => return state.doc.feature(sketch.feature).map(|f| f.name.clone()).unwrap_or_else(|| "Sketch".into()),
    };
    if id == 0 {
        return "Origin".into();
    }
    let name = sketch
        .sketch
        .entity(id)
        .map(|entity| match &entity.kind {
            EntityKind::Line { .. } => "Line",
            EntityKind::Circle { .. } => "Circle",
            EntityKind::Arc { .. } => "Arc",
            EntityKind::Point { .. } => "Point",
            EntityKind::Spline { .. } => "Spline",
            EntityKind::Text { .. } => "Text",
            _ => "Entity",
        })
        .unwrap_or("Entity");
    format!("{name}{id}")
}

fn caption(state: &AppState, target: Target, selection: Option<Selection>) -> String {
    match target {
        Target::Empty => "Empty space".into(),
        Target::Body => crate::document::BODY_NAME.into(),
        Target::SketchLine => sketch_caption(state),
        Target::EmptySketch => state
            .sketch
            .as_ref()
            .and_then(|s| state.doc.feature(s.feature))
            .map(|f| f.name.clone())
            .unwrap_or_else(|| "Sketch".into()),
        _ => selection.and_then(|s| state.doc.describe(s)).map(|info| info.title).unwrap_or_default(),
    }
}

fn item(
    state: &mut AppState,
    id: &'static str,
    target: Target,
    clicked: Option<Selection>,
    short: &str,
) -> ring_menu::Item {
    state.doc.selection = selection_for(&state.doc, target, clicked);
    let command = commands::get(id).expect("context commands are registered");
    let label = if id == "hide_body" && !short.is_empty() {
        if crate::tree_visibility::visible(state).body { "Hide".into() } else { "Show".into() }
    } else if short.is_empty() {
        command.label_in(state)
    } else {
        short.to_owned()
    };
    ring_menu::Item {
        command,
        label,
        enabled: command.available(Context::of(state)) && command.is_enabled(state),
        soon: !command.is_working(),
        selected: (command.active)(state),
        selection: state.doc.selection,
        shortcut: state.shortcuts.shortcut_text(id).unwrap_or_default(),
        tip: command.tip(state),
    }
}

fn snapshot(state: &mut AppState, target: Target, clicked: Option<Selection>) -> ring_menu::Menu {
    let original = state.doc.selection;
    let title = caption(state, target, clicked);
    let quick = spec(target).quick.map(|q| item(state, q.command, q.target, clicked, q.label));
    let columns = groups(target)
        .iter()
        .filter_map(|group| {
            state.doc.selection = selection_for(&state.doc, group.target, clicked);
            let subtitle = if group.target == Target::Empty {
                String::new()
            } else {
                caption(state, group.target, state.doc.selection)
            };
            let context = Context::of(state);
            let ids = group
                .commands
                .iter()
                .copied()
                .filter(|id| {
                    let command = commands::get(id).expect("context commands are registered");
                    command.available(context) && (command.shown)(state)
                })
                .collect::<Vec<_>>();
            let items = ids.into_iter().map(|id| item(state, id, group.target, clicked, "")).collect::<Vec<_>>();
            (!items.is_empty()).then_some(ring_menu::Column { label: group.label, subtitle, items })
        })
        .collect();
    state.doc.selection = original;
    ring_menu::Menu { title, quick, columns }
}

#[derive(Clone, Copy)]
struct OpenMenu {
    target: Target,
    selection: Option<Selection>,
    at: egui::Pos2,
    document: soliddraft_document::DocumentId,
    sketch: Option<crate::document::FeatureId>,
}

pub fn show(state: &mut AppState, response: &egui::Response, target: Target, selection: Option<Selection>) {
    let at = response.secondary_clicked().then(|| response.interact_pointer_pos()).flatten();
    show_at(state, response, target, selection, at);
}

pub fn show_at(
    state: &mut AppState,
    response: &egui::Response,
    target: Target,
    selection: Option<Selection>,
    at: Option<egui::Pos2>,
) {
    if state.frame_layout != FrameLayout::EditSuite {
        return;
    }
    let key = response.id.with("context_target");
    let popup_id = response.id.with("context_menu");
    let opening = at.is_some();
    let document = state.doc.model().id();
    let sketch = state.sketch.as_ref().map(|s| s.feature);
    let menu = at
        .map(|at| OpenMenu { target, selection, at, document, sketch })
        .or_else(|| response.ctx.data(|data| data.get_temp::<OpenMenu>(key)));
    let Some(menu) = menu else { return };
    if menu.document != document || menu.sketch != sketch {
        egui::Popup::close_id(&response.ctx, popup_id);
        return;
    }
    if opening {
        egui::Popup::open_id(&response.ctx, popup_id);
        if !state.in_sketch() {
            state.doc.selection = selection;
        }
        response.ctx.data_mut(|data| data.insert_temp(key, menu));
    }
    if !egui::Popup::is_id_open(&response.ctx, popup_id) {
        return;
    }
    let model = snapshot(state, menu.target, menu.selection);
    let layout = ring_menu::Layout::measure(&response.ctx, &model, menu.at);
    let chosen = egui::Popup::new(
        popup_id,
        response.ctx.clone(),
        egui::Rect::from_min_size(layout.bounds.min, egui::Vec2::ZERO),
        response.layer_id,
    )
    .kind(egui::PopupKind::Menu)
    .open_memory(None)
    .frame(egui::Frame::NONE)
    .gap(0.0)
    .width(layout.bounds.width())
    .align_alternatives(&[])
    .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
    .show(|ui| ring_menu::show(ui, &model, &layout, opening))
    .and_then(|shown| shown.inner);
    if let Some(choice) = chosen {
        egui::Popup::close_id(&response.ctx, popup_id);
        if !state.in_sketch() {
            state.doc.selection = choice.selection;
        }
        state.run_command(choice.command);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_targets_have_registered_commands_and_sketches_have_no_solid_tools() {
        let mut seen = Vec::new();
        for (target, menu) in MENUS {
            assert!(!seen.contains(&target));
            seen.push(target);
            let mut quick = Vec::new();
            for q in menu.quick {
                assert!(!quick.contains(&q.command), "repeated ring command {}", q.command);
                quick.push(q.command);
                let command = commands::get(q.command).expect("registered ring command");
                if matches!(target, Target::SketchLine | Target::EmptySketch) {
                    assert!(command.available(Context { sketch: true, ..Context::default() }));
                    assert!(!matches!(q.command, "extrude" | "cut" | "edit_feature" | "delete_feature"));
                }
            }
            assert_eq!(quick.len(), 8);
            for group in menu.groups {
                for id in group.commands {
                    let command = commands::get(id).unwrap_or_else(|| panic!("unknown context command {id}"));
                    if matches!(target, Target::SketchLine | Target::EmptySketch) {
                        assert!(command.available(Context { sketch: true, ..Context::default() }), "{id}");
                        assert!(!matches!(*id, "extrude" | "cut" | "revolve" | "sweep"));
                    }
                }
            }
        }
        assert_eq!(seen.len(), 7);
        assert_eq!(groups(Target::Face).iter().map(|g| g.label).collect::<Vec<_>>(), ["Face", "Feature", "Body"]);
        assert_eq!(groups(Target::Edge).iter().map(|g| g.label).collect::<Vec<_>>(), ["Edge", "Feature", "Body"]);
    }
}
