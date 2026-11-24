//! In-context references: geometry a part takes from another component of an assembly
//! while it is edited in that assembly.
//!
//! A part has no assembly of its own, so it keeps a copy of what it took: the plane of
//! the referenced face in the part's own frame, with the names that say where the plane
//! came from. The part rebuilds from that copy anywhere, with or without the assembly.
//! While the part is edited in the assembly again, the application reads the face afresh
//! and updates the copy through an ordinary feature edit, or marks the reference broken
//! when the face cannot be found; a broken reference keeps its last plane.

use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::{Plane, Vec3};

use crate::feature::{Feature, FeatureId, FeatureKind, SketchPlane};

/// How far a stored plane may stray from unit length or square axes before it is
/// refused: the planes are made from unit vectors in double precision.
const SQUARE: f64 = 1e-9;

/// A planar face of another component, as the part last read it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextFace {
    /// The assembly's document id, as text.
    pub assembly: String,
    /// The assembly's name when the face was last read.
    pub assembly_name: String,
    /// This part's own occurrence in the assembly, whose frame the plane is in.
    pub own: String,
    /// The component, by its occurrence path in the assembly.
    pub component: String,
    /// The component's name in the assembly tree when the face was last read.
    pub component_name: String,
    /// The face's persistent name on that component's part, as its display mesh names it.
    pub face: String,
    /// The face's label, such as "Boss-Extrude1 · End cap", for the tree and messages.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub face_label: String,
    /// A point of the face's plane, in this part's frame, in millimetres.
    pub origin: [f64; 3],
    /// The plane's unit normal, out of the other part's material.
    pub normal: [f64; 3],
    /// The plane's unit X axis.
    pub x_dir: [f64; 3],
    /// Why the last update could not follow the face. The plane stays as it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broken: Option<String>,
}

fn array(v: Vec3) -> [f64; 3] {
    [v.x, v.y, v.z]
}

fn vector(a: [f64; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

impl ContextFace {
    /// The reference to `face` of `component` in `assembly`, lying in `plane` in the frame
    /// of the part's own occurrence `own`. `assembly` and `component` pair an id or path
    /// with a name.
    pub fn new(
        assembly: (impl Into<String>, impl Into<String>),
        own: impl Into<String>,
        component: (impl Into<String>, impl Into<String>),
        face: (impl Into<String>, impl Into<String>),
        plane: Plane,
    ) -> ContextFace {
        ContextFace {
            assembly: assembly.0.into(),
            assembly_name: assembly.1.into(),
            own: own.into(),
            component: component.0.into(),
            component_name: component.1.into(),
            face: face.0.into(),
            face_label: face.1.into(),
            origin: array(plane.origin),
            normal: array(plane.normal),
            x_dir: array(plane.x_dir),
            broken: None,
        }
    }

    /// The plane the part builds on. Refused when the stored numbers are not a plane:
    /// not finite, a normal or axis not of unit length, or axes not square.
    pub fn plane(&self) -> Result<Plane, String> {
        let finite = self.origin.iter().chain(&self.normal).chain(&self.x_dir).all(|v| v.is_finite());
        if !finite {
            return Err("the stored plane is not finite".into());
        }
        let (origin, normal, x_dir) = (vector(self.origin), vector(self.normal), vector(self.x_dir));
        if (normal.length() - 1.0).abs() > SQUARE || (x_dir.length() - 1.0).abs() > SQUARE {
            return Err("the stored plane's axes are not of unit length".into());
        }
        if normal.dot(x_dir).abs() > SQUARE {
            return Err("the stored plane's axes are not square".into());
        }
        Ok(Plane { origin, normal, x_dir })
    }

    /// "Bracket<1> · End cap": the component and the face, for the tree and messages.
    pub fn label(&self) -> String {
        let face = if self.face_label.is_empty() { &self.face } else { &self.face_label };
        format!("{} · {face}", self.component_name)
    }

    /// This reference read afresh: the same names with the plane and names now found.
    pub fn followed(&self, assembly_name: &str, component_name: &str, plane: Plane) -> ContextFace {
        ContextFace {
            assembly_name: assembly_name.to_owned(),
            component_name: component_name.to_owned(),
            origin: array(plane.origin),
            normal: array(plane.normal),
            x_dir: array(plane.x_dir),
            broken: None,
            ..self.clone()
        }
    }

    /// This reference marked broken for `why`, its plane kept.
    pub fn broken(&self, why: impl Into<String>) -> ContextFace {
        ContextFace { broken: Some(why.into()), ..self.clone() }
    }
}

/// The in-context references of a feature: a sketch or a reference plane on another
/// component's face.
pub fn of_feature(kind: &FeatureKind) -> Option<&ContextFace> {
    match kind {
        FeatureKind::Sketch(s) => match &s.plane {
            SketchPlane::Context { context, .. } => Some(context),
            _ => None,
        },
        FeatureKind::ReferencePlane(SketchPlane::Context { context, .. }) => Some(context),
        _ => None,
    }
}

/// Every in-context reference of the part, with its feature, in tree order.
pub fn references(features: &[Feature]) -> Vec<(FeatureId, &ContextFace)> {
    features.iter().filter_map(|f| of_feature(&f.kind).map(|c| (f.id, c))).collect()
}

/// `kind` with its in-context reference replaced by `context`; `None` when it has none.
pub fn with_reference(kind: &FeatureKind, context: ContextFace) -> Option<FeatureKind> {
    let mut kind = kind.clone();
    let slot = match &mut kind {
        FeatureKind::Sketch(s) => match &mut s.plane {
            SketchPlane::Context { context, .. } => context,
            _ => return None,
        },
        FeatureKind::ReferencePlane(SketchPlane::Context { context, .. }) => context,
        _ => return None,
    };
    *slot = context;
    Some(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face() -> ContextFace {
        ContextFace::new(
            ("asm-id", "Rig"),
            "1",
            ("2", "Plate<1>"),
            ("base-end-cap", "Base · End cap"),
            Plane { origin: Vec3::new(0.0, 0.0, 10.0), normal: Vec3::Z, x_dir: Vec3::X },
        )
    }

    #[test]
    fn a_stored_plane_reads_back_and_bad_numbers_are_refused() {
        let c = face();
        let p = c.plane().unwrap();
        assert_eq!((p.origin, p.normal, p.x_dir), (Vec3::new(0.0, 0.0, 10.0), Vec3::Z, Vec3::X));
        assert_eq!(c.label(), "Plate<1> · Base · End cap");
        let tilted = ContextFace { x_dir: [0.0, 0.6, 0.8], ..face() };
        assert!(tilted.plane().unwrap_err().contains("square"));
        let long = ContextFace { normal: [0.0, 0.0, 2.0], ..face() };
        assert!(long.plane().unwrap_err().contains("unit"));
        let lost = ContextFace { origin: [f64::NAN, 0.0, 0.0], ..face() };
        assert!(lost.plane().is_err());
    }

    #[test]
    fn following_keeps_the_names_and_breaking_keeps_the_plane() {
        let c = face();
        let moved = c.followed(
            "Rig 2",
            "Plate<2>",
            Plane { origin: Vec3::new(1.0, 2.0, 3.0), normal: Vec3::X, x_dir: Vec3::Y },
        );
        assert_eq!(
            (moved.assembly.as_str(), moved.own.as_str(), moved.component.as_str(), moved.face.as_str()),
            ("asm-id", "1", "2", "base-end-cap")
        );
        assert_eq!((moved.assembly_name.as_str(), moved.component_name.as_str()), ("Rig 2", "Plate<2>"));
        assert_eq!(moved.origin, [1.0, 2.0, 3.0]);
        let broken = moved.broken("the face is gone");
        assert_eq!(broken.broken.as_deref(), Some("the face is gone"));
        assert_eq!(broken.plane().unwrap(), moved.plane().unwrap());
        assert_eq!(broken.followed("Rig 2", "Plate<2>", moved.plane().unwrap()).broken, None);
    }

    #[test]
    fn a_reference_round_trips_through_its_file_text() {
        let c = face().broken("gone");
        let text = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<ContextFace>(&text).unwrap(), c);
        let kept = serde_json::to_string(&face()).unwrap();
        assert!(!kept.contains("broken"), "{kept}");
    }
}
