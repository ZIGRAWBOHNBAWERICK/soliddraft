//! A label moves on the view plane through its grab point, never on a screen-space offset.
use super::{ViewportEvents, camera::Camera, dimensions::ShownDimension};
use crate::dimension_edit::DimensionAction;
use crate::document::Document;
use crate::state::AppAction;
use egui::{Key, PointerButton, Pos2, Rect};
use glam::Vec3;

pub(super) struct AnnotationDrag {
    pub key: soliddraft_model::view::AnnotationKey,
    pub position: Vec3,
    initial: Vec3,
    grab: Vec3,
    normal: Vec3,
    pressed_at: Pos2,
    moved: bool,
    cancelled: bool,
    finished: bool,
    document: soliddraft_document::DocumentId,
    revision: u64,
}

fn on_plane(camera: &Camera, rect: Rect, pointer: Pos2, at: Vec3, normal: Vec3) -> Option<Vec3> {
    let (origin, direction) = camera.ray(pointer, rect);
    let denominator = direction.dot(normal);
    if denominator.abs() < 1e-6 {
        return None;
    }
    let position = origin + direction * ((at - origin).dot(normal) / denominator);
    position.is_finite().then_some(position)
}

/// Captures primary label gestures before the navigation router sees them. Preview stays
/// local until release; cancellation never writes a command or changes geometry.
pub(super) fn input(
    drag: &mut Option<AnnotationDrag>,
    shown: &[ShownDimension],
    camera: &Camera,
    rect: Rect,
    ui: &egui::Ui,
    doc: &Document,
    events: &mut ViewportEvents,
) -> bool {
    let (pressed, released, pointer, escape) = ui.input(|i| {
        (
            i.pointer.button_pressed(PointerButton::Primary),
            i.pointer.button_released(PointerButton::Primary),
            i.pointer.interact_pos(),
            i.key_pressed(Key::Escape),
        )
    });
    if drag.as_ref().is_some_and(|d| d.finished) {
        *drag = None;
    }
    if drag.as_ref().is_some_and(|d| {
        d.document != doc.model().id() || d.revision != doc.revision() || !shown.iter().any(|s| s.key == d.key)
    }) {
        *drag = None;
    }
    if pressed
        && drag.is_none()
        && ui.rect_contains_pointer(rect)
        && let Some(pointer) = pointer
        && let Some(dimension) = shown.iter().rev().find(|d| d.rect.contains(pointer))
        && let Some(grab) = on_plane(camera, rect, pointer, dimension.position, camera.forward())
    {
        *drag = Some(AnnotationDrag {
            key: dimension.key,
            position: dimension.position,
            initial: dimension.position,
            grab,
            normal: camera.forward(),
            pressed_at: pointer,
            moved: false,
            cancelled: false,
            finished: false,
            document: doc.model().id(),
            revision: doc.revision(),
        });
    }
    let Some(active) = drag.as_mut() else { return false };
    if escape {
        ui.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, Key::Escape);
        });
        active.position = active.initial;
        active.cancelled = true;
    }
    if active.cancelled {
        if released {
            *drag = None;
        }
        return true;
    }
    if let Some(pointer) = pointer {
        active.moved |= pointer.distance(active.pressed_at) > 3.0;
        if active.moved
            && let Some(at) = on_plane(camera, rect, pointer, active.initial, active.normal)
        {
            active.position = active.initial + at - active.grab;
        }
    }
    if active.moved {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        ui.ctx().request_repaint();
    }
    if released {
        active.finished = true;
        if active.moved && active.position.distance_squared(active.initial) > 1e-8 {
            events
                .actions
                .push(AppAction::Dimension(DimensionAction::Move { key: active.key, position: active.position }));
        }
    }
    true
}
