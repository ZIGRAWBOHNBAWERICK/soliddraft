//! Edit Suite right-click targets and their command groups.

use crate::commands::{self, Context};
use crate::document::{Document, Selection};
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

const FACE: Group =
    Group { target: Target::Face, label: "Face", commands: &["sketch", "normal_to", "measure", "hole", "draft"] };
const EDGE: Group = Group { target: Target::Edge, label: "Edge", commands: &["fillet", "chamfer", "measure"] };
const FEATURE: Group = Group {
    target: Target::Feature,
    label: "Feature",
    commands: &["edit_feature", "repair_feature", "rename", "suppress", "rollback_here", "zoom_fit", "delete_feature"],
};
const BODY: Group =
    Group { target: Target::Body, label: "Body", commands: &["zoom_fit", "mass_properties", "shell", "mirror"] };
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
    commands: &["display_shaded_edges", "display_shaded", "display_wireframe", "orthographic"],
};
const SHOW: Group = Group { target: Target::Empty, label: "Show", commands: &["toggle_tree"] };
const LINE: Group = Group {
    target: Target::SketchLine,
    label: "Line",
    commands: &[
        "smart_dimension",
        "construction",
        "sketch_trim",
        "sketch_extend",
        "sketch_move",
        "sketch_copy",
        "delete",
    ],
};
const SKETCH: Group = Group {
    target: Target::EmptySketch,
    label: "Sketch",
    commands: &["exit_sketch", "normal_to", "sketch_dimensions", "sketch_shading", "relations"],
};
const SKETCH_TOOLS: Group = Group {
    target: Target::EmptySketch,
    label: "Sketch tools",
    commands: &[
        "smart_dimension",
        "sketch_line",
        "sketch_centreline",
        "sketch_rectangle",
        "sketch_circle",
        "sketch_arc",
    ],
};

/// The menu contents, independent of their current plain-list presentation.
pub const MENUS: [(Target, &[Group]); 7] = [
    (Target::Face, &[FACE, FEATURE, BODY]),
    (Target::Edge, &[EDGE, FEATURE, BODY]),
    (Target::Feature, &[FEATURE, BODY]),
    (Target::Body, &[BODY]),
    (Target::Empty, &[VIEW, SCENE, DISPLAY, SHOW]),
    (Target::SketchLine, &[LINE, SKETCH]),
    (Target::EmptySketch, &[SKETCH_TOOLS, SKETCH, SCENE]),
];

pub fn groups(target: Target) -> &'static [Group] {
    MENUS.iter().find(|(key, _)| *key == target).expect("every target has a menu").1
}

fn group_selection(doc: &Document, group: &Group, clicked: Option<Selection>) -> Option<Selection> {
    match group.target {
        Target::Feature => clicked.and_then(|s| doc.selection_feature(s)).map(Selection::Feature),
        Target::Body => doc.display_body().map(|_| Selection::Body),
        _ => clicked,
    }
}

#[derive(Clone, Copy)]
struct OpenMenu {
    target: Target,
    selection: Option<Selection>,
    at: egui::Pos2,
    open: bool,
}

/// Opens on a secondary click and retains the clicked target while the pointer moves.
pub fn show(state: &mut AppState, response: &egui::Response, target: Target, selection: Option<Selection>) {
    let at = response.secondary_clicked().then(|| response.interact_pointer_pos()).flatten();
    show_at(state, response, target, selection, at);
}

/// Opens at a resolved pick, which may arrive after the secondary-click frame.
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
    let menu = at
        .map(|at| OpenMenu { target, selection, at, open: true })
        .or_else(|| response.ctx.data(|data| data.get_temp::<OpenMenu>(key)));
    let Some(mut menu) = menu else { return };
    if !menu.open {
        return;
    }
    if at.is_some() && !state.in_sketch() {
        // Secondary selection must not feed an editor's primary-click pick collector.
        state.doc.selection = selection;
    }
    let OpenMenu { target, selection, at, .. } = menu;
    let mut chosen = None;
    egui::Popup::new(
        response.id.with("context_menu"),
        response.ctx.clone(),
        egui::Rect::from_min_size(at, egui::Vec2::ZERO),
        response.layer_id,
    )
    .kind(egui::PopupKind::Menu)
    .open_bool(&mut menu.open)
    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
    .layout(egui::Layout::top_down_justified(egui::Align::Min))
    .gap(0.0)
    .show(|ui| {
        let original = state.doc.selection;
        for group in groups(target) {
            state.doc.selection = group_selection(&state.doc, group, selection);
            let context = Context::of(state);
            let commands: Vec<_> = group
                .commands
                .iter()
                .filter_map(|id| commands::get(id))
                .filter(|c| c.available(context) && (c.shown)(state))
                .collect();
            if commands.is_empty() {
                continue;
            }
            ui.label(group.label);
            for command in commands {
                let label = if command.is_working() {
                    command.label_in(state)
                } else {
                    format!("{} · Coming soon", command.label_in(state))
                };
                let mut button = egui::Button::new(label);
                if let Some(shortcut) = state.shortcuts.shortcut_text(command.id) {
                    button = button.shortcut_text(shortcut);
                }
                if ui.add_enabled(command.is_enabled(state), button).on_hover_text(command.tip(state)).clicked() {
                    chosen = Some((command, state.doc.selection));
                    ui.close();
                }
            }
            ui.separator();
        }
        state.doc.selection = original;
    });
    if let Some((command, selection)) = chosen {
        menu.open = false;
        if !state.in_sketch() {
            state.doc.selection = selection;
        }
        state.run_command(command);
    }
    response.ctx.data_mut(|data| data.insert_temp(key, menu));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_targets_have_registered_commands_and_sketches_have_no_solid_tools() {
        let mut seen = Vec::new();
        for (target, groups) in MENUS {
            assert!(!seen.contains(&target));
            seen.push(target);
            for group in groups {
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
