//! A scene written as SolidDraft documents.
//!
//! The root becomes the assembly at `out`. Every node with children becomes a
//! sub-assembly, and every mesh a mesh reference body (`.dwmsh`), both in a folder
//! `<name>.parts` beside it. A node's component carries its name and its file's hidden
//! flag; its material becomes the assembly's colour on that component.
//!
//! Assembly placements are rigid, so a node's frame is split in two: its rigid part
//! (rotation and translation, from the polar decomposition of its frame in the root's)
//! places it, and what is left (scale, shear or mirror) is baked into its mesh. A tree
//! of rigid frames keeps every frame as the file has it. A mesh used twice with the same
//! baked residual is one mesh reference, placed twice.
//!
//! The root assembly keeps the import's record in `import.json`: the source file and its
//! digest, the units and up axis and where they came from, every node's occurrence path,
//! the materials, the joints in assembly terms and what was left out.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use glam::{DMat3, DMat4, DQuat, DVec3};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use soliddraft_assembly::file::RefCopy;
use soliddraft_assembly::path::{ComponentId, OccurrencePath, Step};
use soliddraft_assembly::placement::Placement;
use soliddraft_assembly::reference_mesh::{MeshDocument, MeshSource};
use soliddraft_assembly::schema::Assembly;
use soliddraft_assembly::{DisplayMesh, MeshFace};
use soliddraft_document::{DocKind, DocRef, DocumentId};
use soliddraft_materials::{Appearance, AppearanceId, Srgb};

use crate::{ImportError, Scene};

/// The record's format tag.
pub const RECORD_FORMAT: &str = "soliddraft-import/1";

/// Adjacent triangles whose normals differ by more than this show an edge between them.
const FEATURE_ANGLE_DEG: f64 = 30.0;

/// What [`write`] made.
#[derive(Clone, Debug)]
pub struct Written {
    /// The root assembly.
    pub assembly: PathBuf,
    /// Every file written, the root assembly last.
    pub files: Vec<PathBuf>,
    /// The import record, as `import.json` holds it.
    pub record: Value,
}

/// Writes `scene`, read from `source`, as the assembly `out` and the documents it
/// places. Files of the same names are replaced.
pub fn write(scene: &Scene, source: &Path, out: &Path) -> Result<Written, ImportError> {
    let io = |what: &Path, e: std::io::Error| ImportError(format!("{}: {e}", what.display()));
    let bytes = std::fs::read(source).map_err(|e| io(source, e))?;
    let sha256: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    let file_name = source.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
    let stem = out.file_stem().and_then(|s| s.to_str()).filter(|s| !s.is_empty()).unwrap_or("Imported").to_owned();
    let dir = out.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let folder_name = format!("{stem}.parts");
    let folder = dir.join(&folder_name);
    std::fs::create_dir_all(&folder).map_err(|e| io(&folder, e))?;

    let n = scene.nodes.len();
    let world: Vec<DMat4> = (0..n).map(|i| scene.world(i)).collect();
    let rigid: Vec<DMat4> = world.iter().map(rigid_part).collect();
    let mut w = Writer {
        scene,
        source: MeshSource {
            file: file_name.clone(),
            format: serde_json::to_value(scene.format)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            sha256: sha256.clone(),
            node: String::new(),
        },
        world,
        rigid,
        names: FileNames::default(),
        meshes: HashMap::new(),
        mesh_order: Vec::new(),
        built: HashMap::new(),
    };
    let mut component: Vec<Option<ComponentId>> = vec![None; n];
    let mut own: Vec<Option<ComponentId>> = vec![None; n];
    let mut colours: Vec<Option<String>> = vec![None; n];
    let materials: Vec<Appearance> = scene.materials.iter().enumerate().map(|(i, m)| appearance(i, m)).collect();
    // Each group's assembly, children first: node indices put parents first.
    let groups: Vec<usize> = (0..n).filter(|&i| w.is_group(i)).rev().collect();
    for g in groups {
        let node = &scene.nodes[g];
        let mut a = Assembly::new(if g == 0 { scene.name.clone() } else { node.name.clone() });
        let mut copies: Vec<RefCopy> = Vec::new();
        let mut placed: Vec<(usize, ComponentId)> = Vec::new();
        if node.body.is_some() {
            let at = w.frame(g).inverse() * w.rigid[g];
            let c = w.place(&mut a, &mut copies, g, g, placement(&at))?;
            own[g] = Some(c);
            placed.push((g, c));
        }
        for &c in &node.children {
            let at = w.frame(g).inverse() * w.rigid[c];
            let id = w.place(&mut a, &mut copies, g, c, placement(&at))?;
            component[c] = Some(id);
            if !w.is_group(c) {
                placed.push((c, id));
            }
        }
        for (i, c) in placed {
            if let Some(m) = scene.nodes[i].body.and_then(|b| b.material) {
                a.set_colour(&OccurrencePath::root().child(Step::component(c)), Some(materials[m].clone()));
                colours[i] = Some(materials[m].name.clone());
            }
        }
        a.normalize();
        let file = if g == 0 {
            String::new()
        } else {
            w.names.take(&node.name, soliddraft_document::kind::ASSEMBLY_EXTENSION)
        };
        let mut seen = HashSet::new();
        copies.retain(|c| seen.insert((c.document, c.revision.clone())));
        w.built.insert(g, Built { id: DocumentId::new(), file, assembly: a, copies });
    }

    // Occurrence paths, parents first.
    let mut paths: Vec<OccurrencePath> = vec![OccurrencePath::root(); n];
    for i in 1..n {
        let p = scene.nodes[i].parent.expect("only the root has no parent");
        paths[i] = paths[p].child(Step::component(component[i].expect("every node is placed")));
    }
    let body_paths: Vec<OccurrencePath> = (0..n)
        .map(|i| match own[i] {
            Some(c) => paths[i].child(Step::component(c)),
            None => paths[i].clone(),
        })
        .collect();

    // Mesh references first, then sub-assemblies, then the root.
    let mut files = Vec::new();
    for key in &w.mesh_order {
        let (doc, name) = &w.meshes[key];
        let path = folder.join(name);
        doc.save(&path).map_err(|e| ImportError(format!("{}: {e}", path.display())))?;
        files.push(path);
    }
    let mut root = w.built.remove(&0).expect("the root is a group");
    write_subassemblies(&root.assembly, &root.copies, &folder, &mut files)?;
    let mut documents: Vec<String> = w.meshes.values().map(|(_, f)| f.clone()).collect();
    documents.sort();
    let record = record(
        scene,
        &file_name,
        &sha256,
        bytes.len(),
        &folder_name,
        &paths,
        &body_paths,
        &documents,
        &colours,
        &w.world,
        &w.rigid,
    );
    root.assembly.import = Some(serde_json::to_string_pretty(&record).expect("a record always serialises") + "\n");
    // The root's references point into the parts folder.
    for s in &mut root.assembly.sources {
        s.document.path = format!("{folder_name}/{}", s.document.path);
    }
    root.assembly
        .save(out, root.id, Some("import".into()), &root.copies)
        .map_err(|e| ImportError(format!("{}: {e}", out.display())))?;
    files.push(out.to_path_buf());
    Ok(Written { assembly: out.to_path_buf(), files, record })
}

/// A group's assembly, built and not yet written.
struct Built {
    id: DocumentId,
    file: String,
    assembly: Assembly,
    copies: Vec<RefCopy>,
}

/// Mesh key: the scene mesh and the bits of its baked residual.
type MeshKey = (usize, [u64; 16]);

/// What writing keeps between nodes.
struct Writer<'a> {
    scene: &'a Scene,
    source: MeshSource,
    world: Vec<DMat4>,
    rigid: Vec<DMat4>,
    names: FileNames,
    meshes: HashMap<MeshKey, (MeshDocument, String)>,
    mesh_order: Vec<MeshKey>,
    built: HashMap<usize, Built>,
}

impl Writer<'_> {
    fn is_group(&self, i: usize) -> bool {
        i == 0 || !self.scene.nodes[i].children.is_empty()
    }

    /// The frame of a group's own assembly: the root's is the root.
    fn frame(&self, i: usize) -> DMat4 {
        if i == 0 { DMat4::IDENTITY } else { self.rigid[i] }
    }

    /// The mesh reference of node `i`'s body, made once per mesh and residual.
    fn mesh(&mut self, i: usize) -> Result<MeshKey, ImportError> {
        let node = &self.scene.nodes[i];
        let body = node.body.expect("only bodies have meshes");
        let baked = self.rigid[i].inverse() * self.world[i];
        let key = (body.mesh, baked.to_cols_array().map(|v| (v + 0.0).to_bits()));
        if !self.meshes.contains_key(&key) {
            let mesh = display_mesh(&self.scene.meshes[body.mesh], &baked)
                .map_err(|e| ImportError(format!("the mesh of {}: {e}", node.source)))?;
            let source = MeshSource { node: node.source.clone(), ..self.source.clone() };
            let doc = MeshDocument::new(node.name.clone(), source, mesh);
            let name = self.names.take(&node.name, soliddraft_document::MESH_EXTENSION);
            self.meshes.insert(key, (doc, name));
            self.mesh_order.push(key);
        }
        Ok(key)
    }

    /// Places node `i` in group `g`'s assembly: its sub-assembly when it is a group
    /// other than `g`, else its body's mesh reference.
    fn place(
        &mut self,
        a: &mut Assembly,
        copies: &mut Vec<RefCopy>,
        g: usize,
        i: usize,
        placement: Placement,
    ) -> Result<ComponentId, ImportError> {
        let reference = if self.is_group(i) && i != g {
            let b = self.built.remove(&i).expect("children are built first");
            let revision = b.assembly.semantic_hash();
            copies.extend(b.copies.iter().cloned());
            copies.push(RefCopy::assembly(b.id, revision.clone(), &b.assembly.truth_entries()));
            DocRef::new(b.id, DocKind::Assembly, b.file.clone(), revision)
        } else {
            let key = self.mesh(i)?;
            let (doc, file) = &self.meshes[&key];
            copies.push(doc.copy());
            DocRef::new(doc.id, DocKind::Mesh, file.clone(), doc.revision())
        };
        let source = a.add_source(reference);
        let c = a.add_component(source, placement);
        let comp = a.components.iter_mut().find(|x| x.id == c).expect("just added");
        comp.name = Some(self.scene.nodes[i].name.clone());
        // The root is the assembly itself and has no component to hide, so a hidden
        // root hides each component placed in it, and through them everything below.
        comp.hidden = self.scene.nodes[i].hidden || (g == 0 && self.scene.nodes[0].hidden);
        Ok(c)
    }
}

/// Writes every sub-assembly a copy list holds to the parts folder, under the file name
/// its parent's reference gives it.
fn write_subassemblies(
    root: &Assembly,
    copies: &[RefCopy],
    folder: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), ImportError> {
    // Every assembly reachable from the root, by its id, with its file name.
    let truths: HashMap<DocumentId, &RefCopy> = copies
        .iter()
        .filter(|c| c.entries.iter().any(|(n, _)| n == "assembly.json"))
        .map(|c| (c.document, c))
        .collect();
    let mut stack: Vec<Assembly> = vec![root.clone()];
    let mut done = HashSet::new();
    while let Some(a) = stack.pop() {
        for s in &a.sources {
            if s.document.kind != DocKind::Assembly {
                continue;
            }
            let id = s.document.id.expect("imported references have ids");
            if !done.insert(id) {
                continue;
            }
            let copy = truths.get(&id).ok_or_else(|| ImportError("a sub-assembly has no copy".into()))?;
            let json = &copy.entries.iter().find(|(n, _)| n == "assembly.json").expect("filtered").1;
            let sub = Assembly::from_truth(json, None).map_err(|e| ImportError(e.to_string()))?;
            // A sub-assembly carries the copies of everything under it.
            let mut below: Vec<RefCopy> = Vec::new();
            collect(&sub, copies, &mut below);
            let path = folder.join(&s.document.path);
            sub.save(&path, id, Some("import".into()), &below)
                .map_err(|e| ImportError(format!("{}: {e}", path.display())))?;
            files.push(path);
            stack.push(sub);
        }
    }
    Ok(())
}

/// The copies an assembly needs: its sources' and, for each sub-assembly, everything
/// under it.
fn collect(a: &Assembly, copies: &[RefCopy], out: &mut Vec<RefCopy>) {
    for s in &a.sources {
        let Some(c) = copies.iter().find(|c| Some(c.document) == s.document.id && c.revision == s.document.pinned)
        else {
            continue;
        };
        if out.iter().any(|o| o.document == c.document && o.revision == c.revision) {
            continue;
        }
        out.push(c.clone());
        if s.document.kind == DocKind::Assembly
            && let Some((_, json)) = c.entries.iter().find(|(n, _)| n == "assembly.json")
            && let Ok(sub) = Assembly::from_truth(json, None)
        {
            collect(&sub, copies, out);
        }
    }
}

/// File names in the parts folder, unique without regard to case.
#[derive(Default)]
struct FileNames(HashSet<String>);

impl FileNames {
    fn take(&mut self, name: &str, extension: &str) -> String {
        let base: String = name
            .chars()
            .map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') { c } else { '_' })
            .collect::<String>()
            .trim_matches(|c: char| c == '.' || c.is_whitespace())
            .to_owned();
        let base = if base.is_empty() { "body".to_owned() } else { base };
        let mut k = 1;
        loop {
            let candidate = if k == 1 { format!("{base}.{extension}") } else { format!("{base} {k}.{extension}") };
            if self.0.insert(candidate.to_lowercase()) {
                return candidate;
            }
            k += 1;
        }
    }
}

/// The rigid part of a frame: its translation, and the rotation nearest its linear part
/// (the polar decomposition), without any mirror.
pub fn rigid_part(m: &DMat4) -> DMat4 {
    let mut a = DMat3::from_mat4(*m);
    if a.determinant() < 0.0 {
        a = -a;
    }
    // Newton's iteration for the polar factor; a few steps reach f64 precision for any
    // frame a file is likely to hold.
    let mut r = a;
    for _ in 0..64 {
        let det = r.determinant();
        if !det.is_finite() || det.abs() < 1e-300 {
            return DMat4::from_translation(m.w_axis.truncate());
        }
        let next = (r + r.inverse().transpose()) * 0.5;
        let delta = (next - r).to_cols_array().iter().map(|v| v.abs()).fold(0.0, f64::max);
        r = next;
        if delta < 1e-15 {
            break;
        }
    }
    let q = DQuat::from_mat3(&r).normalize();
    DMat4::from_rotation_translation(q, m.w_axis.truncate())
}

/// A rigid frame as an assembly placement.
fn placement(m: &DMat4) -> Placement {
    let (_, rotation, translation) = m.to_scale_rotation_translation();
    Placement::new(rotation.normalize(), translation)
}

/// A scene mesh with `baked` applied, as a display mesh: one face, normals per corner
/// (from the file, or each triangle's own), corners with the same position and normal
/// shared, and edges where the surface folds or ends.
pub fn display_mesh(mesh: &crate::Mesh, baked: &DMat4) -> Result<DisplayMesh, String> {
    let linear = DMat3::from_mat4(*baked);
    let flip = linear.determinant() < 0.0;
    let normal_matrix = linear.inverse().transpose();
    let points: Vec<DVec3> = mesh.positions.iter().map(|p| baked.transform_point3(*p)).collect();
    let mut out = DisplayMesh::default();
    let mut corners: HashMap<([u32; 3], [u32; 3]), u32> = HashMap::new();
    let mut kept = 0u32;
    for (t, tri) in mesh.triangles.iter().enumerate() {
        let [a, b, c] = tri.map(|i| points[i as usize]);
        let face = (b - a).cross(c - a);
        if face.length_squared() == 0.0 || !face.is_finite() {
            continue;
        }
        let face = face.normalize() * if flip { -1.0 } else { 1.0 };
        let order = if flip { [0, 2, 1] } else { [0, 1, 2] };
        for k in order {
            let p = points[tri[k] as usize].as_vec3().to_array();
            let n = match &mesh.corner_normals {
                Some(ns) => {
                    let v = normal_matrix * ns[t * 3 + k];
                    if v.length_squared() > 0.0 && v.is_finite() { v.normalize() } else { face }
                }
                None => face,
            };
            let n = n.as_vec3().to_array();
            let key = (p.map(f32::to_bits), n.map(f32::to_bits));
            let index = *corners.entry(key).or_insert_with(|| {
                out.positions.push(p);
                out.normals.push(n);
                out.positions.len() as u32 - 1
            });
            out.indices.push(index);
        }
        kept += 1;
    }
    if kept == 0 {
        return Err("it has no triangle with area".into());
    }
    out.faces = vec![MeshFace { first: 0, count: kept, name: String::new() }];
    out.edges = feature_edges(&out);
    out.check()?;
    Ok(out)
}

/// Edges between triangles that fold by more than [`FEATURE_ANGLE_DEG`], and edges with
/// one triangle, as two-point polylines.
fn feature_edges(m: &DisplayMesh) -> Vec<Vec<[f32; 3]>> {
    let key = |i: u32| m.positions[i as usize].map(f32::to_bits);
    let normal = |t: usize| {
        let [a, b, c] =
            [0, 1, 2].map(|k| glam::Vec3::from_array(m.positions[m.indices[t * 3 + k] as usize]).as_dvec3());
        (b - a).cross(c - a).normalize_or_zero()
    };
    let mut edges: HashMap<([u32; 3], [u32; 3]), Vec<usize>> = HashMap::new();
    for t in 0..m.triangle_count() {
        for k in 0..3 {
            let (a, b) = (key(m.indices[t * 3 + k]), key(m.indices[t * 3 + (k + 1) % 3]));
            edges.entry(if a < b { (a, b) } else { (b, a) }).or_default().push(t);
        }
    }
    let cos = FEATURE_ANGLE_DEG.to_radians().cos();
    let mut out: Vec<Vec<[f32; 3]>> = edges
        .into_iter()
        .filter(|(_, ts)| ts.len() != 2 || normal(ts[0]).dot(normal(ts[1])) < cos)
        .map(|((a, b), _)| vec![a.map(f32::from_bits), b.map(f32::from_bits)])
        .collect();
    out.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// A scene material as an assembly appearance.
fn appearance(index: usize, m: &crate::Material) -> Appearance {
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let id: String = m
        .name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    Appearance {
        id: AppearanceId(format!("import-{}-{}", index + 1, if id.is_empty() { "material" } else { &id })),
        name: m.name.clone(),
        base_color: Srgb { r: byte(m.base_color[0]), g: byte(m.base_color[1]), b: byte(m.base_color[2]) },
        metallic: m.metallic.clamp(0.0, 1.0) as f32,
        roughness: m.roughness.clamp(0.0, 1.0) as f32,
        opacity: m.base_color[3].clamp(0.0, 1.0) as f32,
        emissive: Srgb { r: 0, g: 0, b: 0 },
    }
}

/// Rounds for the record: what the file says to a micrometre or a millionth.
fn round(v: f64) -> f64 {
    let r = (v * 1e6).round() / 1e6;
    if r == 0.0 { 0.0 } else { r }
}

fn vec(v: DVec3) -> [f64; 3] {
    v.to_array().map(round)
}

#[allow(clippy::too_many_arguments)]
fn record(
    scene: &Scene,
    file: &str,
    sha256: &str,
    size: usize,
    folder: &str,
    paths: &[OccurrencePath],
    body_paths: &[OccurrencePath],
    documents: &[String],
    colours: &[Option<String>],
    world: &[DMat4],
    rigid: &[DMat4],
) -> Value {
    let nodes: Vec<Value> = (0..scene.nodes.len())
        .map(|i| {
            let n = &scene.nodes[i];
            let baked = rigid[i].inverse() * world[i];
            let (scale, ..) = baked.to_scale_rotation_translation();
            json!({
                "name": n.name,
                "path": scene.path(i),
                "source": n.source,
                "occurrence": paths[i].to_string(),
                "kind": if i == 0 || !n.children.is_empty() { "assembly" } else { "mesh" },
                "body": n.body.map(|_| body_paths[i].to_string()),
                "material": colours[i],
                "hidden": n.hidden,
                "scale_baked": (!baked.abs_diff_eq(DMat4::IDENTITY, 1e-9)).then(|| vec(scale)),
            })
        })
        .collect();
    let joints: Vec<Value> = scene
        .joints
        .iter()
        .map(|j| {
            // The joint frame in the root's, then in the parent's assembly frame.
            let parent_frame = if j.parent == 0 { DMat4::IDENTITY } else { rigid[j.parent] };
            let in_world = world[j.parent] * j.frame;
            let local = parent_frame.inverse() * rigid_part(&in_world);
            let axis = (parent_frame.inverse() * in_world).transform_vector3(j.axis).normalize_or_zero();
            let (_, rotation, origin) = local.to_scale_rotation_translation();
            let turn = matches!(j.kind, crate::JointKind::Revolute | crate::JointKind::Continuous);
            json!({
                "name": j.name,
                "kind": j.kind,
                "parent": paths[j.parent].to_string(),
                "child": paths[j.child].to_string(),
                "parent_name": scene.path(j.parent),
                "child_name": scene.path(j.child),
                "origin": vec(origin),
                "rotation": rotation.normalize().to_array().map(round),
                "axis": vec(axis),
                "limits": j.limits.map(|l| l.map(round)),
                "units": if turn { "deg" } else if j.kind == crate::JointKind::Prismatic { "mm" } else { "" },
                "mimic": j.mimic,
            })
        })
        .collect();
    let bounds = scene.bounds().map(|[lo, hi]| json!({ "min": vec(lo), "max": vec(hi) }));
    json!({
        "format": RECORD_FORMAT,
        "source": { "file": file, "format": scene.format, "sha256": sha256, "bytes": size },
        "name": scene.name,
        "units": { "meters_per_unit": scene.meters_per_unit, "from": scene.units_from },
        "up_axis": { "axis": scene.up_axis, "from": scene.up_from },
        "folder": folder,
        "bounds": bounds,
        "nodes": nodes,
        "materials": scene.materials,
        "joints": joints,
        "meshes": documents,
        "notes": scene.notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rigid_part_drops_scale_and_mirror() {
        let r = DQuat::from_rotation_z(0.7);
        let t = DVec3::new(1.0, 2.0, 3.0);
        let m = DMat4::from_scale_rotation_translation(DVec3::new(0.001, 0.001, 0.001), r, t);
        let rigid = rigid_part(&m);
        assert!(rigid.abs_diff_eq(DMat4::from_rotation_translation(r, t), 1e-12));
        let mirrored = m * DMat4::from_scale(DVec3::new(-1.0, 1.0, 1.0));
        let rigid = rigid_part(&mirrored);
        assert!((DMat3::from_mat4(rigid).determinant() - 1.0).abs() < 1e-12);
        let residual = rigid.inverse() * mirrored;
        assert!(rigid.mul_mat4(&residual).abs_diff_eq(mirrored, 1e-12));
    }

    #[test]
    fn mirrored_meshes_keep_their_outward_winding() {
        let mesh = crate::Mesh {
            name: "t".into(),
            positions: vec![DVec3::ZERO, DVec3::X, DVec3::Y],
            triangles: vec![[0, 1, 2]],
            corner_normals: None,
        };
        let d = display_mesh(&mesh, &DMat4::from_scale(DVec3::new(-1.0, 1.0, 1.0))).unwrap();
        let [a, b, c] = [0, 1, 2].map(|k| glam::Vec3::from_array(d.positions[d.indices[k] as usize]));
        assert!((b - a).cross(c - a).z > 0.0, "the normal still faces +Z");
        assert_eq!(d.normals[0], [0.0, 0.0, 1.0]);
        assert_eq!(d.edges.len(), 3, "an open triangle shows its three edges");
    }

    #[test]
    fn file_names_stay_unique_and_plain() {
        let mut names = FileNames::default();
        assert_eq!(names.take("plate", "dwmsh"), "plate.dwmsh");
        assert_eq!(names.take("Plate", "dwmsh"), "Plate 2.dwmsh");
        assert_eq!(names.take("a/b:c", "dwasm"), "a_b_c.dwasm");
        assert_eq!(names.take("..", "dwasm"), "body.dwasm");
    }
}
