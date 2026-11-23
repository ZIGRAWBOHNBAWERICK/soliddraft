//! Display data: the part's meshes at its display levels, saved in the part file so an
//! assembly, a drawing preview or a browser can show the part without rebuilding it.
//!
//! A part file keeps its coarse and standard levels (`cache/mesh/coarse.bin`,
//! `cache/mesh/standard.bin`) when its settings say to save display data and the part
//! has a body. Each is a `soliddraft_tessellation::DisplayMesh` whose faces carry the
//! part's persistent names (the model's face reference as canonical JSON), never kernel
//! ids, in canonical order, so the same part always saves to the same bytes. Its header records what made it: the kernel, the level, the exact tessellation
//! parameters and the part's geometry hash. A mesh whose geometry hash is not the
//! file's is never returned, so an edited part never shows an old shape; the kernel and
//! parameters are for the reader to judge, because any kernel's mesh of the same
//! geometry is a true picture of it.
//!
//! The section is optional: a reader that does not want display data drops it, and
//! deleting it never changes the part.

use soliddraft_document::hash::canonical_json;
use soliddraft_document::manifest::Package;
use soliddraft_kernel_api::{FaceId, KernelResult};
use soliddraft_tessellation::mesh::display_mesh;
use soliddraft_tessellation::{DisplayMesh, Level, Levels, Quality};

use crate::document::Document;
use crate::file::FileError;

/// The folder of the display data in a part file.
pub const FOLDER: &str = "cache/mesh/";

/// The levels a part file keeps.
pub const SAVED: [Level; 2] = [Level::Coarse, Level::Standard];

/// The entry that holds the display mesh of `level`.
pub fn entry(level: Level) -> String {
    format!("{FOLDER}{level}.bin")
}

/// One level of display data and what made it.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayData {
    /// The level.
    pub level: Level,
    /// The mesh, faces named by persistent reference.
    pub mesh: DisplayMesh,
    /// What made it: kernel, version, level, parameters and geometry hash, separated by
    /// spaces, the geometry hash last.
    pub made_by: String,
}

impl Document {
    /// The part's persistent name for a face of its body: its face reference as
    /// canonical JSON, or empty when the model cannot name it.
    pub fn face_name(&self, face: FaceId) -> String {
        self.face_ref(face)
            .ok()
            .and_then(|r| serde_json::to_value(r).ok())
            .map(|v| canonical_json(&v))
            .unwrap_or_default()
    }

    /// The part's display meshes at `levels` of `quality`, from one kernel call, with
    /// faces named by persistent reference and edges sampled along their curves. `None`
    /// without a body.
    pub fn display_data(&self, quality: &Quality, levels: &[Level]) -> Option<KernelResult<Vec<DisplayData>>> {
        let bodies = self.live_bodies();
        if bodies.is_empty() {
            return None;
        }
        Some((|| {
            let mut diagonal: f64 = 0.0;
            for (_, body) in &bodies {
                diagonal = diagonal.max(self.kernel.topology(*body)?.bbox.diagonal());
            }
            let all = Levels::new(quality, diagonal);
            let params: Vec<_> = levels.iter().map(|l| *all.params(*l)).collect();
            let backend = self.kernel.backend();
            let geometry = self.geometry_hash();
            let styled = !self.materials().is_empty();
            let styles = styled.then(|| self.appearance_table(self.active_configuration()));
            let mut out: Vec<_> = levels
                .iter()
                .map(|level| DisplayData {
                    level: *level,
                    mesh: DisplayMesh::default(),
                    made_by: format!(
                        "{} {} {level} {} {geometry}",
                        backend.name,
                        backend.version,
                        all.key(*level).describe()
                    ),
                })
                .collect();
            for (_, body) in bodies {
                let meshes = self.kernel.preview_levels(body, &params)?;
                for ((data, mesh), p) in out.iter_mut().zip(meshes).zip(&params) {
                    let edges = self.kernel.preview_edges(body, p).unwrap_or_default();
                    let polylines = edges
                        .iter()
                        .map(|e| e.points.iter().map(|q| [q.x as f32, q.y as f32, q.z as f32]).collect())
                        .collect();
                    let mut faces = Vec::new();
                    let mut mesh = display_mesh(
                        &mesh,
                        |f| {
                            if let Some(table) = &styles {
                                let look = table.of(f);
                                let c = look.base_color;
                                faces.push([c.r, c.g, c.b, packed(look.metallic), packed(look.roughness)]);
                            }
                            self.face_name(f)
                        },
                        polylines,
                    );
                    for (face, style) in mesh.faces.iter_mut().zip(faces) {
                        face.style = Some(style);
                    }
                    let first_vertex = data.mesh.positions.len() as u32;
                    let first_triangle = data.mesh.triangle_count() as u32;
                    data.mesh.positions.extend(mesh.positions);
                    data.mesh.normals.extend(mesh.normals);
                    data.mesh.indices.extend(mesh.indices.into_iter().map(|index| first_vertex + index));
                    data.mesh.faces.extend(mesh.faces.into_iter().map(|mut face| {
                        face.first += first_triangle;
                        face
                    }));
                    data.mesh.edges.extend(mesh.edges);
                }
            }
            for data in &mut out {
                data.mesh = data.mesh.canonical();
            }
            Ok(out)
        })())
    }
}

fn packed(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The display mesh of `level` in a part file's bytes, with what made it, when the file
/// holds one made from its current geometry. `Ok(None)` when it holds none, or one
/// from other geometry.
pub fn read(part_file: &[u8], level: Level) -> Result<Option<(DisplayMesh, String)>, FileError> {
    let mut package = Package::from_bytes(part_file)?;
    let Some(geometry) = package.manifest().and_then(|m| m.geometry_hash.clone()) else { return Ok(None) };
    let Some(bytes) = package.read(&entry(level))? else { return Ok(None) };
    let (mesh, made_by) = DisplayMesh::decode(&bytes).map_err(|e| FileError::Archive(e.to_string()))?;
    let current = made_by.rsplit(' ').next() == Some(geometry.as_str());
    if current
        && mesh.faces.iter().all(|face| face.style.is_none())
        && let Some(truth) = package.read_text("part.json")?
        && let Ok(recipe) = serde_json::from_str::<serde_json::Value>(&truth)
        && (recipe.get("material").is_some()
            || recipe.get("appearances").is_some()
            || recipe.get("display_states").is_some())
    {
        return Ok(None);
    }
    Ok(current.then_some((mesh, made_by)))
}
