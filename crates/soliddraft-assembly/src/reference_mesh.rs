//! Mesh reference bodies (`.dwmsh`): imported meshes an assembly places and shows.
//!
//! A mesh reference body is what a scene import makes of a mesh: it is drawn, picked,
//! hidden, coloured and moved like a part, and it never answers an engineering question.
//! It has no features and no exact solid, so measure, mass, mates and drawing projection
//! refuse it. Converting it to a solid waits for the kernel.
//!
//! The file is a document envelope with:
//!
//! - `mesh.json` (required): the truth, [`MeshTruth`]: the body's name, where it came
//!   from (file, format, SHA-256 of the file, the node in the file), its counts, and the
//!   BLAKE3 hash of `mesh.bin`;
//! - `mesh.bin` (required): the mesh as a [`DisplayMesh`], in millimetres in the body's
//!   own frame.
//!
//! The semantic hash covers `mesh.json`, which names the mesh's hash, so a changed mesh
//! is a new revision. The geometry hash is the mesh's hash alone. An assembly keeps the
//! same `mesh.bin` bytes as its display copy (`cache/refs/<id>/<revision>/display.bin`).

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use soliddraft_document::manifest::{self, Entry, Manifest, Package, Section};
use soliddraft_document::{ContentHash, DocKind, DocumentId, EnvelopeError, kind::format_major};

use crate::file::{FileError, RefCopy};
use crate::mesh::DisplayMesh;

/// The truth's format tag.
pub const FORMAT: &str = "soliddraft-mesh/1";
/// The truth entry.
pub const TRUTH: &str = "mesh.json";
/// The mesh entry.
pub const MESH: &str = "mesh.bin";
/// What made the mesh, as its display copy records it.
pub const MADE_BY: &str = "import";

/// `mesh.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshTruth {
    /// The format tag, [`FORMAT`].
    pub format: String,
    /// The body's name, from the imported file.
    pub name: String,
    /// Where the mesh came from.
    pub source: MeshSource,
    /// The BLAKE3 hash of `mesh.bin`.
    pub mesh: ContentHash,
    /// Vertices in the mesh.
    pub vertices: u32,
    /// Triangles in the mesh.
    pub triangles: u32,
}

/// Where a mesh reference body came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshSource {
    /// The imported file's name.
    pub file: String,
    /// Its format, such as "usd" or "gltf".
    pub format: String,
    /// The SHA-256 of the imported file's bytes, lower-case hex.
    pub sha256: String,
    /// The node in the file the mesh belongs to, such as a USD prim path.
    pub node: String,
}

/// A mesh reference body in memory.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshDocument {
    /// The document's id.
    pub id: DocumentId,
    /// The truth.
    pub truth: MeshTruth,
    /// The mesh.
    pub mesh: DisplayMesh,
    /// `mesh.bin`'s bytes, as written.
    bytes: Vec<u8>,
}

impl MeshDocument {
    /// A new mesh reference body with a new id.
    pub fn new(name: impl Into<String>, source: MeshSource, mesh: DisplayMesh) -> MeshDocument {
        let bytes = mesh.encode(MADE_BY);
        MeshDocument {
            id: DocumentId::new(),
            truth: MeshTruth {
                format: FORMAT.to_owned(),
                name: name.into(),
                source,
                mesh: ContentHash::of_bytes(&bytes),
                vertices: mesh.positions.len() as u32,
                triangles: mesh.triangle_count() as u32,
            },
            mesh,
            bytes,
        }
    }

    /// The revision: the semantic hash of `mesh.json`.
    pub fn revision(&self) -> ContentHash {
        manifest::semantic_hash(DocKind::Mesh, &self.truth_json())
    }

    fn truth_json(&self) -> Value {
        serde_json::to_value(&self.truth).expect("a mesh truth always serialises")
    }

    /// The file's bytes. The same document always gives the same bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, FileError> {
        let mut m = Manifest::new(self.id, DocKind::Mesh, self.revision());
        m.geometry_hash = Some(self.truth.mesh.clone());
        m.sections = vec![Section::required(TRUTH), Section::required(MESH)];
        let truth = serde_json::to_string_pretty(&self.truth)? + "\n";
        Ok(manifest::write_package(
            &m,
            &[Entry::new(TRUTH, truth.as_bytes()), Entry::new(MESH, self.bytes.as_slice())],
        )?)
    }

    /// Writes the file atomically.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), FileError> {
        Ok(soliddraft_document::save_atomic(path.as_ref(), &self.to_bytes()?)?)
    }

    /// Reads a file's bytes: checks the envelope, the format and that `mesh.bin` is the
    /// mesh `mesh.json` names.
    pub fn from_bytes(bytes: &[u8]) -> Result<MeshDocument, FileError> {
        let mut package = Package::from_bytes(bytes)?;
        let m = package
            .manifest()
            .cloned()
            .ok_or_else(|| EnvelopeError::Unsupported("not a SolidDraft mesh reference: no manifest".into()))?;
        m.check(DocKind::Mesh, &[TRUTH, MESH])?;
        let json = package.read(TRUTH)?.ok_or_else(|| EnvelopeError::Unsupported(format!("no {TRUTH}")))?;
        let truth: MeshTruth = serde_json::from_slice(&json)?;
        if format_major(&truth.format, "soliddraft-mesh") != Some(1) {
            return Err(FileError::Format { found: truth.format });
        }
        let bytes = package.read(MESH)?.ok_or_else(|| EnvelopeError::Unsupported(format!("no {MESH}")))?;
        if ContentHash::of_bytes(&bytes) != truth.mesh {
            return Err(EnvelopeError::Unsupported(format!(
                "{MESH} is not the mesh {TRUTH} names; the file is damaged"
            ))
            .into());
        }
        let (mesh, _) = DisplayMesh::decode(&bytes)?;
        Ok(MeshDocument { id: m.id, truth, mesh, bytes })
    }

    /// Reads a file.
    pub fn open(path: impl AsRef<Path>) -> Result<MeshDocument, FileError> {
        MeshDocument::from_bytes(&std::fs::read(path)?)
    }

    /// The display copy an assembly keeps of this revision: the same `mesh.bin` bytes.
    pub fn copy(&self) -> RefCopy {
        RefCopy {
            document: self.id,
            revision: self.revision(),
            entries: vec![(crate::mesh::DISPLAY_MESH.to_owned(), self.bytes.clone().into())],
        }
    }
}

/// The mesh in a mesh reference file's bytes, with what made it: what an assembly shows
/// when it has no display copy.
pub fn display(bytes: &[u8]) -> Result<(DisplayMesh, String), String> {
    let doc = MeshDocument::from_bytes(bytes).map_err(|e| e.to_string())?;
    Ok((doc.mesh, MADE_BY.to_owned()))
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::*;

    fn sample() -> MeshDocument {
        let source = MeshSource {
            file: "bracket.usda".into(),
            format: "usd".into(),
            sha256: "00".repeat(32),
            node: "/plate/Geometry/plate/plate".into(),
        };
        MeshDocument::new("plate", source, DisplayMesh::cuboid(DVec3::ZERO, DVec3::new(80.0, 40.0, 6.0)))
    }

    #[test]
    fn mesh_references_round_trip_and_name_their_mesh() {
        let doc = sample();
        let bytes = doc.to_bytes().unwrap();
        assert_eq!(doc.to_bytes().unwrap(), bytes, "deterministic");
        let back = MeshDocument::from_bytes(&bytes).unwrap();
        assert_eq!(back, doc);
        assert_eq!(back.revision(), doc.revision());
        let id = soliddraft_document::manifest::identify(std::io::Cursor::new(&bytes)).unwrap();
        assert_eq!((id.kind, id.revision), (DocKind::Mesh, doc.revision()));
        let (mesh, made_by) = display(&bytes).unwrap();
        assert_eq!((mesh.triangle_count(), made_by.as_str()), (12, MADE_BY));
        assert_eq!(doc.copy().revision, doc.revision());
    }

    #[test]
    fn a_renamed_body_is_a_new_revision_of_the_same_geometry() {
        let doc = sample();
        let mut renamed = doc.clone();
        renamed.truth.name = "base".into();
        assert_ne!(renamed.revision(), doc.revision());
        let a = Package::from_bytes(&doc.to_bytes().unwrap()).unwrap().manifest().unwrap().geometry_hash.clone();
        let b = Package::from_bytes(&renamed.to_bytes().unwrap()).unwrap().manifest().unwrap().geometry_hash.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn a_mesh_that_does_not_match_its_truth_is_damage() {
        let doc = sample();
        let mut other = sample();
        other.truth = doc.truth.clone();
        other.truth.mesh = ContentHash::of_bytes(b"something else");
        let err = MeshDocument::from_bytes(&other.to_bytes().unwrap()).unwrap_err();
        assert!(err.to_string().contains("damaged"), "{err}");
        let err = MeshDocument::from_bytes(b"not a zip").unwrap_err();
        assert!(!err.to_string().is_empty());
    }
}
