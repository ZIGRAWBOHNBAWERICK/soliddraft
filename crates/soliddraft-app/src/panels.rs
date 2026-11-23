//! The D1 Clear Bench single side area: stacks any combination of toggled panes
//! (`Tree`, `History`, `Values`, `Props`) vertically top-to-bottom with draggable
//! splitters and a close `×` per pane. Turning all panes off clears the column completely.
//!
//! Layout choices persist in `settings.json` while remaining backwards-compatible with
//! older settings files.

use serde::{Deserialize, Serialize};

/// The side areas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    /// The single left side column (Browser / History / Values / Props).
    Tree,
}

impl Side {
    pub const ALL: [Side; 1] = [Side::Tree];

    pub fn title(self) -> &'static str {
        match self {
            Side::Tree => "Feature tree",
        }
    }

    /// The title as it reads inside a sentence.
    pub fn name(self) -> &'static str {
        match self {
            Side::Tree => "feature tree",
        }
    }

    /// The command that shows or hides the side area.
    pub fn command(self) -> &'static str {
        match self {
            Side::Tree => "toggle_tree",
        }
    }

    pub fn min_width(self) -> f32 {
        match self {
            Side::Tree => 210.0,
        }
    }

    pub fn default_width(self) -> f32 {
        match self {
            Side::Tree => crate::theme::clear_bench::SPACING.side_panel_default_width,
        }
    }

    /// The egui id of the panel while it is shown; its width lives under this id.
    pub fn panel_id(self) -> egui::Id {
        egui::Id::new(("side_panel", self))
    }

    /// The egui id of the strip left at the edge while the panel is hidden.
    pub fn strip_id(self) -> egui::Id {
        egui::Id::new(("side_strip", self))
    }
}

/// Panes that can be toggled and stacked top-to-bottom in the single left side column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pane {
    /// The document's browser / feature tree (part, assembly, or drawing).
    #[default]
    Tree,
    /// Chronological feature history with rollback playback controls.
    History,
    /// Globals, parameters and configurations (mount point for parameters lane).
    Values,
    /// Selection & feature properties (mount point for feature panel lane).
    Props,
}

impl Pane {
    pub const ALL: [Pane; 4] = [Pane::Tree, Pane::History, Pane::Values, Pane::Props];

    pub fn index(self) -> usize {
        match self {
            Pane::Tree => 0,
            Pane::History => 1,
            Pane::Values => 2,
            Pane::Props => 3,
        }
    }
}

/// Which panes are toggled on in the single left side column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SidePanes {
    pub tree: bool,
    pub history: bool,
    pub values: bool,
    pub props: bool,
}

impl Default for SidePanes {
    /// D1 Clear Bench default: Browser (`Tree`) + `History` stacked cleanly.
    fn default() -> Self {
        Self { tree: true, history: true, values: false, props: false }
    }
}

impl SidePanes {
    pub const NONE: Self = Self { tree: false, history: false, values: false, props: false };

    pub fn is_open(self, pane: Pane) -> bool {
        match pane {
            Pane::Tree => self.tree,
            Pane::History => self.history,
            Pane::Values => self.values,
            Pane::Props => self.props,
        }
    }

    pub fn set(&mut self, pane: Pane, open: bool) {
        match pane {
            Pane::Tree => self.tree = open,
            Pane::History => self.history = open,
            Pane::Values => self.values = open,
            Pane::Props => self.props = open,
        }
    }

    pub fn toggle(&mut self, pane: Pane) {
        let now = self.is_open(pane);
        self.set(pane, !now);
    }

    pub fn any_open(self) -> bool {
        self.tree || self.history || self.values || self.props
    }

    pub fn clear(&mut self) {
        *self = Self::NONE;
    }

    pub fn active_list(self) -> Vec<Pane> {
        Pane::ALL.into_iter().filter(|p| self.is_open(*p)).collect()
    }
}

/// The widest a side area gets, whatever the window size.
pub const MAX_WIDTH: f32 = 800.0;
/// The viewport keeps at least this much beside the side area.
pub const VIEWPORT_MIN_WIDTH: f32 = 320.0;
/// Width of the legacy strip id (D1 clears the side column to 0px when all panes are off).
pub const STRIP_WIDTH: f32 = 22.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SidePanel {
    pub shown: bool,
    /// The outer width in logical pixels while shown; kept while hidden.
    pub width: f32,
}

/// Visibility, stacked panes, split weights, and width of the single side column.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub tree: SidePanel,
    pub suite: SuitePanes,
    /// Primary focused pane (kept for backwards settings compatibility).
    pub pane: Pane,
    /// Which panes are currently stacked open in the single side column.
    pub panes: SidePanes,
    /// Relative vertical weights for `[Tree, History, Values, Props]` when stacked.
    pub split_weights: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SuitePanes {
    pub part: bool,
    pub assembly: bool,
    pub drawing: bool,
}

/// Every document starts with its side column shown.
impl Default for SuitePanes {
    fn default() -> Self {
        Self { part: true, assembly: true, drawing: true }
    }
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            tree: SidePanel { shown: true, width: Side::Tree.default_width() },
            suite: SuitePanes::default(),
            pane: Pane::default(),
            panes: SidePanes::default(),
            split_weights: [0.56, 0.44, 0.44, 0.44],
        }
    }
}

impl Layout {
    pub fn panel(&self, side: Side) -> &SidePanel {
        match side {
            Side::Tree => &self.tree,
        }
    }

    pub fn panel_mut(&mut self, side: Side) -> &mut SidePanel {
        match side {
            Side::Tree => &mut self.tree,
        }
    }

    pub fn toggle(&mut self, side: Side) {
        let p = self.panel_mut(side);
        p.shown = !p.shown;
    }

    /// Brings a width or weight that is not finite back to the default and the rest
    /// into range, so a hand-edited or stale file cannot hide a panel off-screen.
    pub fn clamped(mut self) -> Self {
        for side in Side::ALL {
            let p = self.panel_mut(side);
            if !p.width.is_finite() {
                p.width = side.default_width();
            }
            p.width = p.width.clamp(side.min_width(), MAX_WIDTH);
        }
        for w in &mut self.split_weights {
            if !w.is_finite() || *w < 0.1 {
                *w = 0.5;
            }
            *w = w.clamp(0.1, 4.0);
        }
        self
    }

    /// The widest this side may be in a window this wide: what leaves the viewport its
    /// minimum, capped at [`MAX_WIDTH`] and never below the side's own minimum.
    pub fn max_width(&self, side: Side, window_width: f32) -> f32 {
        (window_width - VIEWPORT_MIN_WIDTH).min(MAX_WIDTH).max(side.min_width())
    }

    /// The saved layout, or the default when there is none or it cannot be read.
    pub fn load() -> Self {
        crate::settings::layout()
    }

    pub fn save(&self) {
        if let Err(e) = crate::settings::save_layout(self) {
            log::warn!("cannot save the panel layout: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::settings::{layout_in, save_layout_in};
    use std::path::PathBuf;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("soliddraft-layout-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn the_layout_round_trips_through_the_settings_file_beside_other_keys() {
        let file = temp_file("round-trip.json");
        let _ = std::fs::remove_file(&file);
        assert_eq!(layout_in(&file), Layout::default(), "no file means the default");
        std::fs::write(&file, r#"{"assistant":{"provider":"openrouter"}}"#).unwrap();
        let mut layout = Layout::default();
        layout.toggle(Side::Tree);
        layout.tree.width = 455.5;
        layout.pane = Pane::Values;
        layout.panes.values = true;
        save_layout_in(&file, &layout).unwrap();
        let back = layout_in(&file);
        assert_eq!(back, layout);
        assert_eq!(back.pane, Pane::Values, "the side area remembers its pane");
        assert!(back.panes.values);
        assert!(!back.tree.shown);
        assert_eq!(back.tree.width, 455.5);
        let all: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(all["assistant"]["provider"], "openrouter", "the other sections survive");
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn a_bad_file_falls_back_and_widths_come_into_range() {
        let file = temp_file("bad.json");
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(layout_in(&file), Layout::default());
        std::fs::write(
            &file,
            r#"{"layout":{"tree":{"shown":false,"width":10.0},"assistant":{"shown":true,"width":99999.0}}}"#,
        )
        .unwrap();
        let back = layout_in(&file);
        assert!(!back.tree.shown, "the old assistant key is ignored");
        assert_eq!(back.tree.width, Side::Tree.min_width());
        std::fs::write(&file, r#"{"layout":{"tree":{"shown":true,"width":99999.0}}}"#).unwrap();
        assert_eq!(layout_in(&file).tree.width, MAX_WIDTH);
        let nan = Layout { tree: SidePanel { shown: true, width: f32::NAN }, ..Layout::default() }.clamped();
        assert_eq!(nan.tree.width, Side::Tree.default_width());
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn the_maximum_leaves_room_for_the_viewport() {
        let layout = Layout::default();
        assert_eq!(layout.max_width(Side::Tree, 1440.0), MAX_WIDTH);
        assert_eq!(layout.max_width(Side::Tree, 1000.0), 680.0);
        assert_eq!(layout.max_width(Side::Tree, 400.0), Side::Tree.min_width());
    }
}
