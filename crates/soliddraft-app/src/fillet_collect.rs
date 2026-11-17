//! What the open Fillet or Chamfer panel collects from viewport picks, and the typed
//! size it accepts. An edge pick adds or removes that edge; a face pick adds every sharp
//! edge on its boundary that is not collected yet, or nothing when one of them has no
//! name. A face pick leaves out smooth edges, where two faces meet tangentially, as
//! SolidWorks does: there is no corner there to round. Anything else, and a size that is
//! not a positive number, is refused beside the control it came from and changes nothing.

use soliddraft_kernel_api::{
    BodyId, EdgeId, EnclosedSupport, FaceId, Kernel, SupportEntity, SurfaceKind, Topology, Vec3,
};

use crate::document::{Document, Selection};
use crate::import::middle;
use crate::state::FeatureEditor;

/// The input the panel refused, shown beside the control it came from until the user
/// corrects it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Feedback {
    /// A pick that is not an edge or a face, or one that could not be named.
    pub pick: Option<String>,
    /// A typed radius or distance that is not a size. The parameters keep the last
    /// accepted one, but nothing previews or commits until it is corrected.
    pub size: Option<String>,
    /// A refused second chamfer distance, independent of the first field.
    pub second_size: Option<String>,
}

/// Takes a viewport pick into the open Fillet or Chamfer panel.
pub fn collect(editor: &mut FeatureEditor, doc: &Document, pick: Selection) {
    let refused = match pick {
        Selection::Edge(edge) => match (doc.edge_ref(edge), editor.edges_mut()) {
            (Some(r), Some(list)) => {
                match list.iter().position(|x| x.label == r.label) {
                    Some(i) => {
                        list.remove(i);
                    }
                    None => list.push(r),
                }
                None
            }
            _ => Some("That edge has no name yet; pick it again once the part has rebuilt"),
        },
        Selection::Face(face) => match (doc.face_edge_refs(face), editor.edges_mut()) {
            (Some(refs), Some(_)) if refs.is_empty() => {
                Some("Every edge of that face is smooth, so there is no corner to round; pick a sharp edge")
            }
            (Some(refs), Some(list)) => {
                for r in refs {
                    if !list.iter().any(|x| x.label == r.label) {
                        list.push(r);
                    }
                }
                None
            }
            _ => Some("Not every edge of that face has a name; pick its edges one at a time"),
        },
        Selection::Vertex(_) => Some("A vertex cannot be rounded: pick an edge or a face"),
        _ => Some("Pick an edge or a face of the part"),
    };
    editor.feedback.pick = refused.map(str::to_owned);
}

/// The largest angle between two faces' normals at which the edge they share is smooth.
const SMOOTH_DEG: f64 = 1.0;

/// Whether `edge` is smooth: a seam, or an edge whose two faces have the same outward
/// normal at its midpoint, such as where a fillet meets a wall or a slot's straight
/// wall meets its round end. Read from the kernel's own planes, cylinders and spheres;
/// an edge on any other surface is not known to be smooth and stays sharp.
pub fn smooth(kernel: &dyn Kernel, body: BodyId, topology: &Topology, edge: EdgeId) -> bool {
    let Some(info) = topology.edge(edge) else { return false };
    let [a, b] = info.faces[..] else { return false };
    if a == b {
        return true;
    }
    let normal = |face| outward_normal(kernel, body, topology, face, info.midpoint);
    match (normal(a), normal(b)) {
        (Some(na), Some(nb)) => na.dot(nb) >= SMOOTH_DEG.to_radians().cos(),
        _ => false,
    }
}

/// The unit normal out of the material of `face` at `at`, a point on its boundary.
fn outward_normal(kernel: &dyn Kernel, body: BodyId, topology: &Topology, face: FaceId, at: Vec3) -> Option<Vec3> {
    let info = topology.face(face)?;
    let point =
        |p: &[soliddraft_kernel_api::SupportBounds; 3]| Some(Vec3::new(middle(&p[0])?, middle(&p[1])?, middle(&p[2])?));
    let (radial, outward) = match info.surface {
        SurfaceKind::Plane => return info.normal?.normalized(),
        SurfaceKind::Cylinder | SurfaceKind::Sphere => {
            match kernel.entity_support_enclosed(body, SupportEntity::Face(face)).ok()? {
                EnclosedSupport::Axis { direction, through, radius_squared: Some(_), outward } => {
                    let axis = point(&direction)?.normalized()?;
                    let offset = at - point(&through)?;
                    (offset - axis * offset.dot(axis), outward?)
                }
                EnclosedSupport::Point { at: centre, radius_squared: Some(_), outward } => {
                    (at - point(&centre)?, outward?)
                }
                _ => return None,
            }
        }
        _ => return None,
    };
    let radial = radial.normalized()?;
    Some(if outward { radial } else { -radial })
}

/// Reads a typed radius or distance in millimetres: a positive, finite number.
pub fn parse_size(text: &str, what: &str) -> Result<f64, String> {
    let text = text.trim();
    let number = text.strip_suffix("mm").unwrap_or(text).trim();
    match number.parse::<f64>() {
        Ok(v) if !v.is_finite() => Err(format!("The {what} must be a finite number of mm")),
        Ok(v) if v <= 0.0 => Err(format!("The {what} must be more than 0 mm")),
        Ok(v) => Ok(v),
        Err(_) => Err(format!("\"{text}\" is not a {what}: type a number of mm")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_positive_finite_millimetres() {
        assert_eq!(parse_size("2", "radius"), Ok(2.0));
        assert_eq!(parse_size(" 2.5 mm ", "radius"), Ok(2.5));
        for refused in ["0", "-1", "abc", "NaN", "inf", "-inf", ""] {
            assert!(parse_size(refused, "radius").is_err(), "{refused:?} is refused");
        }
    }
}
