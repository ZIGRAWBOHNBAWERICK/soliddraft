//! Body names and visibility choices for a part, keyed by stable identity.
//!
//! The choices are display data, not part of the model. A part file keeps them in
//! `view.json`, an optional section outside both hashes, so hiding a feature never gives
//! the part a new revision and an undo never takes a choice back. A choice outlives its
//! feature: an id an undo removed keeps it, so the redo that brings the feature back
//! brings it back hidden. An item with no choice follows the application's defaults.
//!
//! A reader that cannot read the section opens the part with no choices, as a file
//! without one does.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use crate::feature::DatumAxis;
use crate::feature::{DatumPlane, FeatureId};

/// The entry a part keeps its view choices in.
pub const ENTRY: &str = "view.json";
/// The format tag of the view choices this version writes.
pub const FORMAT: &str = "soliddraft-view/2";
const LEGACY_FORMAT: &str = "soliddraft-view/1";

/// Something the user can show or hide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewItem {
    /// A feature, by its id.
    Feature(FeatureId),
    /// A reference plane.
    Plane(DatumPlane),
    /// An origin axis.
    Axis(DatumAxis),
    /// A body, by its stable key in the feature history.
    Body(u32),
}

/// Stable identity of a viewport dimension. It does not depend on formatted text or list order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AnnotationKey {
    /// The owning feature, including the sketch for sketch constraints.
    pub feature: FeatureId,
    /// Which existing dimension of that feature is shown.
    pub value: AnnotationValue,
}

/// Existing values that can own a viewport label, not new measurement types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AnnotationValue {
    /// A sketch dimension's persistent constraint identity.
    Sketch(soliddraft_sketch::ConstraintId),
    /// Extrusion or hole depth.
    Depth,
    /// Revolve, draft or pattern angle.
    Angle,
    /// Fillet radius.
    Radius,
    /// Chamfer distance.
    Distance,
    /// Shell thickness.
    Thickness,
    /// Hole diameter.
    Diameter,
    /// Hole tip angle.
    Tip,
    /// Counterbore dimensions.
    Counterbore,
    /// Countersink dimensions.
    Countersink,
    /// Pattern instance count.
    Count,
    /// Linear pattern spacing.
    Spacing,
}

/// The part's explicit show and hide choices.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Visibility {
    choices: BTreeMap<ViewItem, bool>,
    body_names: BTreeMap<u32, String>,
    // Free 3D label placement is session-local; view.json has no annotation schema.
    pub(crate) annotations: BTreeMap<AnnotationKey, [f32; 3]>,
}

/// `view.json`: the items shown and hidden, each list in item order.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    format: String,
    #[serde(default)]
    shown: Vec<ViewItem>,
    #[serde(default)]
    hidden: Vec<ViewItem>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    body_names: BTreeMap<u32, String>,
}

impl Visibility {
    /// A dimension label's model-space position, independent of camera and geometry.
    pub fn annotation_position(&self, key: AnnotationKey) -> Option<[f32; 3]> {
        self.annotations.get(&key).copied()
    }

    /// The choice for `item`: shown, hidden, or `None` to follow the defaults.
    pub fn choice(&self, item: ViewItem) -> Option<bool> {
        self.choices.get(&item).copied()
    }

    /// Records a choice for `item`, or forgets it with `None`.
    pub fn set(&mut self, item: ViewItem, shown: Option<bool>) {
        match shown {
            Some(shown) => self.choices.insert(item, shown),
            None => self.choices.remove(&item),
        };
    }

    pub fn body_name(&self, key: u32) -> Option<&str> {
        self.body_names.get(&key).map(String::as_str)
    }

    pub(crate) fn set_body_name(&mut self, key: u32, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            self.body_names.remove(&key);
        } else {
            self.body_names.insert(key, name.to_owned());
        }
    }

    /// True when nothing was chosen, so a file needs no `view.json`.
    pub fn is_empty(&self) -> bool {
        self.choices.is_empty() && self.body_names.is_empty()
    }

    /// The entry's text: pretty JSON with a final newline, the same bytes for the same
    /// choices.
    pub fn to_json(&self) -> String {
        let pick = |shown: bool| self.choices.iter().filter(|(_, s)| **s == shown).map(|(item, _)| *item).collect();
        let legacy = self.body_names.is_empty() && !self.choices.keys().any(|item| matches!(item, ViewItem::Body(_)));
        let format = if legacy { LEGACY_FORMAT } else { FORMAT };
        let entry = Entry {
            format: format.into(),
            shown: pick(true),
            hidden: pick(false),
            body_names: self.body_names.clone(),
        };
        serde_json::to_string_pretty(&entry).expect("view choices always serialise") + "\n"
    }

    /// Reads the entry's text. Another format, an unknown key or an item both shown and
    /// hidden is refused rather than misread.
    pub fn from_json(text: &str) -> Result<Visibility, String> {
        let entry: Entry = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if entry.format != FORMAT && entry.format != LEGACY_FORMAT {
            return Err(format!("the view choices are {:?}; this version reads {FORMAT}", entry.format));
        }
        if entry.format == LEGACY_FORMAT
            && (!entry.body_names.is_empty()
                || entry.shown.iter().chain(&entry.hidden).any(|item| matches!(item, ViewItem::Body(_))))
        {
            return Err(format!("body names and visibility require {FORMAT}"));
        }
        let mut view = Visibility::default();
        for (key, name) in entry.body_names {
            view.set_body_name(key, &name);
        }
        for (items, shown) in [(entry.shown, true), (entry.hidden, false)] {
            for item in items {
                if view.choices.insert(item, shown).is_some() {
                    return Err(format!("{item:?} is listed twice"));
                }
            }
        }
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_round_trip_and_unreadable_entries_are_refused() {
        let mut view = Visibility::default();
        assert!(view.is_empty());
        view.set(ViewItem::Feature(FeatureId(3)), Some(false));
        view.set(ViewItem::Axis(DatumAxis::Y), Some(true));
        view.set(ViewItem::Plane(DatumPlane::Top), Some(false));
        let text = view.to_json();
        assert_eq!(Visibility::from_json(&text).unwrap(), view);
        assert!(text.contains("\"feature\": 3") && text.contains("\"axis\": \"y\""), "{text}");
        view.set(ViewItem::Axis(DatumAxis::Y), None);
        assert_eq!(view.choice(ViewItem::Axis(DatumAxis::Y)), None);

        assert!(text.contains(LEGACY_FORMAT));
        view.set(ViewItem::Body(8), Some(false));
        view.set_body_name(8, "  Tool  ");
        let text = view.to_json();
        assert!(text.contains(FORMAT));
        assert_eq!(Visibility::from_json(&text).unwrap(), view);
        assert_eq!(view.body_name(8), Some("Tool"));
        assert!(Visibility::from_json(&text.replace(FORMAT, "soliddraft-view/3")).is_err());
        assert!(Visibility::from_json(&text.replace(FORMAT, LEGACY_FORMAT)).is_err());
        assert!(
            Visibility::from_json(r#"{"format":"soliddraft-view/1","shown":[{"axis":"x"}],"hidden":[{"axis":"x"}]}"#)
                .is_err()
        );
        assert!(Visibility::from_json(r#"{"format":"soliddraft-view/1","colour":[]}"#).is_err());
    }
}
