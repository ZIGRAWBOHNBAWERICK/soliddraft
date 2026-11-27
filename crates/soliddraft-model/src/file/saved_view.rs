//! Named 3D PMI views stored with a native part.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::FileError;

pub(super) const ENTRY: &str = "saved-views.json";

/// A named camera pose and the exact set of PMI items shown in it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedView {
    /// The user-facing name of the view.
    pub name: String,
    /// The camera to restore when activating the view.
    pub camera: SavedCamera,
    /// Native PMI item IDs, not positions in the item list or STEP entity numbers.
    /// A missing item keeps its ID here so the reference is never rebound by position.
    pub visible_pmi_ids: BTreeSet<String>,
}

/// Orbit camera data, independent of the viewport and its transient state.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedCamera {
    /// Orbit centre in model coordinates, in millimetres.
    pub target: [f32; 3],
    /// Distance from the orbit centre in millimetres.
    pub distance: f32,
    /// Azimuth in radians from +X towards +Y.
    pub yaw: f32,
    /// Elevation in radians above the XY plane.
    pub pitch: f32,
    /// Vertical field of view in radians.
    pub fov_y: f32,
    /// Orthographic rather than perspective projection.
    pub orthographic: bool,
}

pub(super) fn encode(views: &[SavedView]) -> Result<Vec<u8>, FileError> {
    for view in views {
        let SavedCamera { target, distance, yaw, pitch, fov_y, .. } = view.camera;
        if !target.into_iter().chain([distance, yaw, pitch, fov_y]).all(f32::is_finite) {
            return Err(FileError::Invalid(format!("saved view {:?} has non-finite camera data", view.name)));
        }
    }
    let mut bytes = serde_json::to_vec_pretty(views)?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pmi::PartPmi;
    use crate::{Command, Document};
    use soliddraft_kernel_api::Vec3;
    use soliddraft_kernel_api::pmi::{PmiItem, PmiKind};
    use soliddraft_kernel_catallax::CatallaxKernel;
    use std::path::Path;
    use std::sync::Arc;

    fn put_datum(doc: &mut Document, id: &str, letter: &str, normal: Vec3) {
        let topology = doc.kernel().topology(doc.body().unwrap()).unwrap();
        let face = topology.faces.iter().find(|face| face.normal.is_some_and(|n| n.dot(normal) > 0.999)).unwrap();
        let kind = PmiKind::Datum { letter: letter.into(), feature: doc.pmi_feature(&[face.id]).unwrap() };
        doc.execute(Command::PutPmiItem { item: Box::new(PmiItem { id: id.into(), kind }), replaces: None }).unwrap();
    }

    #[test]
    fn native_saved_views_reopen_in_order_and_keep_item_ids() {
        let kernel = Arc::new(CatallaxKernel::new());
        let old = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ci/fixtures/universal-joint/block.dwprt");
        let mut doc = Document::open(old, kernel.clone()).unwrap();
        assert!(doc.saved_views().is_empty());

        put_datum(&mut doc, "Bottom datum", "A", -Vec3::Z);
        put_datum(&mut doc, "Side datum", "B", Vec3::X);
        let part_pmi: PartPmi = doc.pmi().clone();
        let revision = doc.semantic_hash();
        let geometry = doc.geometry_hash();
        let underside = SavedView {
            name: "Underside".into(),
            camera: SavedCamera {
                target: [0.0, 0.0, 3.0],
                distance: 55.0,
                yaw: 0.0,
                pitch: -std::f32::consts::FRAC_PI_2,
                fov_y: 0.8,
                orthographic: true,
            },
            visible_pmi_ids: ["Bottom datum".into()].into(),
        };
        let oblique = SavedView {
            name: "Oblique".into(),
            camera: SavedCamera {
                target: [1.0, 2.0, 3.0],
                distance: 83.0,
                yaw: 0.7,
                pitch: 0.4,
                fov_y: 0.9,
                orthographic: false,
            },
            visible_pmi_ids: ["Bottom datum".into(), "Side datum".into()].into(),
        };
        doc.saved_views_mut().extend([underside.clone(), oblique.clone()]);
        let path = std::env::temp_dir().join(format!("soliddraft-saved-views-{}.dwprt", std::process::id()));
        doc.save(&path).unwrap();
        let mut doc = Document::open(&path, kernel.clone()).unwrap();
        assert_eq!(doc.saved_views(), &[underside.clone(), oblique.clone()]);
        assert_eq!(doc.pmi(), &part_pmi);
        assert_eq!(doc.semantic_hash(), revision);
        assert_eq!(doc.geometry_hash(), geometry);

        doc.saved_views_mut().swap(0, 1);
        let (_, snapshot) = doc.fork().snapshot().encode().unwrap();
        let mut doc = Document::from_bytes(&snapshot, kernel.clone()).unwrap();
        assert_eq!(doc.saved_views(), &[oblique.clone(), underside]);
        doc.saved_views_mut().remove(1);
        doc.execute(Command::RemovePmiItem { id: "Bottom datum".into() }).unwrap();
        doc.save(&path).unwrap();
        let reopened = Document::open(&path, kernel).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(reopened.saved_views(), &[oblique]);
        assert_eq!(reopened.pmi().items, part_pmi.items[1..]);
    }
}
