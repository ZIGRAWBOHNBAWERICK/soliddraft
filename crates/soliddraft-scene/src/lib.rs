//! Scene import: OpenUSD (`.usd`, `.usda`, `.usdc`, `.usdz`), glTF (`.glb`, `.gltf`),
//! URDF, OBJ and STL files read into one [`Scene`]: a named tree of nodes, some carrying a
//! mesh body, with the materials and joints the file has.
//!
//! [`read`] returns the scene in SolidDraft's frame: millimetres, Z up. Each format's own
//! units and up axis are applied once, in [`Scene::normalise`], and the scene says where
//! they came from (the file, the format's rules, a header comment, a default or the
//! caller). Meshes stay meshes: nothing here makes a solid.
//!
//! What a format carries and this reads:
//!
//! | Format | Tree | Units, up axis | Materials | Joints |
//! |---|---|---|---|---|
//! | OpenUSD | prims under the default prim | `metersPerUnit`, `upAxis` | UsdPreviewSurface, OmniPBR | UsdPhysics joints |
//! | glTF | nodes of the default scene | metres, Y up | metallic-roughness factors | sand3d's `s3d_joint` extras |
//! | URDF | links by their joints, visuals in each | metres, Z up | `<material>` colours | every joint |
//! | OBJ | one node per object | header comment or millimetres, Y up | MTL diffuse colours | none |
//! | STL | one node | header comment or millimetres, Z up | none | none |
//!
//! Texture maps are not read: a textured material keeps its constant factors and says it
//! had a texture.

pub mod assemble;
mod gltf_read;
mod obj;
mod shapes;
mod stl;
mod urdf;
mod usd;

use std::path::Path;

use glam::{DMat3, DMat4, DVec3};
use serde::{Deserialize, Serialize};

/// The file extensions [`read`] opens, without the dot.
pub const EXTENSIONS: &[&str] = &["usd", "usda", "usdc", "usdz", "glb", "gltf", "urdf", "obj", "stl"];

/// Why a file could not be imported.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ImportError(pub String);

impl ImportError {
    fn new(what: impl Into<String>) -> ImportError {
        ImportError(what.into())
    }
}

/// A file format this crate reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// OpenUSD, text, binary or packaged.
    Usd,
    /// glTF 2.0, binary or JSON.
    Gltf,
    /// The Unified Robot Description Format.
    Urdf,
    /// Wavefront OBJ.
    Obj,
    /// STL, binary or ASCII.
    Stl,
}

impl Format {
    /// The format a path's extension names, ignoring case.
    pub fn of_path(path: &Path) -> Option<Format> {
        let e = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match e.as_str() {
            "usd" | "usda" | "usdc" | "usdz" => Format::Usd,
            "glb" | "gltf" => Format::Gltf,
            "urdf" => Format::Urdf,
            "obj" => Format::Obj,
            "stl" => Format::Stl,
            _ => return None,
        })
    }

    /// The format's name in words: "OpenUSD".
    pub fn label(self) -> &'static str {
        match self {
            Format::Usd => "OpenUSD",
            Format::Gltf => "glTF",
            Format::Urdf => "URDF",
            Format::Obj => "OBJ",
            Format::Stl => "STL",
        }
    }
}

/// The axis a file takes as up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpAxis {
    /// +Y is up, as in glTF and most OBJ files.
    Y,
    /// +Z is up, as in SolidDraft, URDF and STL.
    Z,
}

/// Where the units or the up axis of a scene came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Authored in the file, such as USD's `metersPerUnit`.
    File,
    /// Fixed by the format's specification: glTF is metres and Y up, URDF metres and Z up.
    Format,
    /// A comment at the head of the file, such as "metres, Y up".
    Header,
    /// Nothing said: the format's usual convention was assumed.
    Default,
    /// The caller's [`Options`].
    Option,
    /// The format's fallback when the file authors nothing, such as USD's centimetres.
    Fallback,
}

/// What the caller decides instead of the file.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Options {
    /// Metres per file unit, such as 0.001 for millimetres.
    pub meters_per_unit: Option<f64>,
    /// The file's up axis.
    pub up_axis: Option<UpAxis>,
}

/// An imported file: a tree of nodes, the meshes and materials they use, and joints.
#[derive(Clone, Debug)]
pub struct Scene {
    /// The scene's name: the default prim, glTF scene, robot or file name.
    pub name: String,
    /// The file's format.
    pub format: Format,
    /// Metres per file unit.
    pub meters_per_unit: f64,
    /// Where `meters_per_unit` came from.
    pub units_from: Origin,
    /// The file's up axis.
    pub up_axis: UpAxis,
    /// Where `up_axis` came from.
    pub up_from: Origin,
    /// Every node; the first is the root and every parent comes before its children.
    pub nodes: Vec<Node>,
    /// The meshes bodies use.
    pub meshes: Vec<Mesh>,
    /// The materials bodies use.
    pub materials: Vec<Material>,
    /// The joints between nodes.
    pub joints: Vec<Joint>,
    /// What the file had that the import left out, in words.
    pub notes: Vec<String>,
    normalised: bool,
}

/// One node of the tree: a group, a part, or both.
#[derive(Clone, Debug)]
pub struct Node {
    /// Its name in the file.
    pub name: String,
    /// Where it is in the file: a prim path, a glTF node index, a URDF link or visual.
    pub source: String,
    /// The parent node; `None` for the root.
    pub parent: Option<usize>,
    /// The child nodes, in file order.
    pub children: Vec<usize>,
    /// Its frame in its parent's: millimetres and Z up once normalised. It may scale,
    /// shear or mirror.
    pub local: DMat4,
    /// The mesh it shows in its own frame.
    pub body: Option<Body>,
    /// Hidden in the file, such as USD's `invisible`.
    pub hidden: bool,
}

/// A mesh shown by a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Body {
    /// Index into [`Scene::meshes`].
    pub mesh: usize,
    /// Index into [`Scene::materials`].
    pub material: Option<usize>,
}

/// A triangle mesh in its node's frame.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    /// Its name in the file.
    pub name: String,
    /// Vertex positions.
    pub positions: Vec<DVec3>,
    /// Triangles, counter-clockwise seen from outside.
    pub triangles: Vec<[u32; 3]>,
    /// A normal per triangle corner, three per triangle, when the file has them.
    pub corner_normals: Option<Vec<DVec3>>,
}

/// A material's constant look.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Material {
    /// Its name in the file.
    pub name: String,
    /// Base colour, sRGB, with alpha for opacity.
    pub base_color: [f64; 4],
    /// Metallic factor, 0 to 1.
    pub metallic: f64,
    /// Roughness factor, 0 to 1.
    pub roughness: f64,
    /// True when a texture drives the colour in the file; only the constant is kept.
    pub textured: bool,
}

/// How a joint lets its child move.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JointKind {
    /// Turns about its axis within limits.
    Revolute,
    /// Turns about its axis without limits.
    Continuous,
    /// Slides along its axis.
    Prismatic,
    /// Does not move.
    Fixed,
    /// Moves freely.
    Floating,
    /// Slides in the plane normal to its axis.
    Planar,
    /// Turns about its origin.
    Spherical,
    /// A joint whose motion the file does not name.
    Other,
}

/// A joint between two nodes.
#[derive(Clone, Debug)]
pub struct Joint {
    /// Its name in the file.
    pub name: String,
    /// How it moves.
    pub kind: JointKind,
    /// The node it is attached to.
    pub parent: usize,
    /// The node it moves.
    pub child: usize,
    /// The joint's frame in the parent node's frame.
    pub frame: DMat4,
    /// The axis in the joint's frame, a unit vector.
    pub axis: DVec3,
    /// The lower and upper limits: millimetres for a slide, degrees for a turn.
    pub limits: Option<[f64; 2]>,
    /// The joint this one follows, if any.
    pub mimic: Option<Mimic>,
}

/// A joint that follows another: this = multiplier × leader + offset.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Mimic {
    /// The leading joint's name.
    pub joint: String,
    /// The multiplier.
    pub multiplier: f64,
    /// The offset, in this joint's units.
    pub offset: f64,
}

/// Reads `path` into a scene in millimetres with Z up.
pub fn read(path: &Path, options: &Options) -> Result<Scene, ImportError> {
    let format = Format::of_path(path).ok_or_else(|| {
        ImportError::new(format!("{} is not a file this import reads ({})", path.display(), EXTENSIONS.join(", ")))
    })?;
    if !path.is_file() {
        return Err(ImportError::new(format!("{} cannot be found", path.display())));
    }
    let mut scene = match format {
        Format::Usd => usd::read(path)?,
        Format::Gltf => gltf_read::read(path)?,
        Format::Urdf => urdf::read(path)?,
        Format::Obj => obj::read(path)?,
        Format::Stl => stl::read(path)?,
    };
    if let Some(m) = options.meters_per_unit {
        if !(m.is_finite() && m > 0.0) {
            return Err(ImportError::new("the units must be a positive length"));
        }
        scene.meters_per_unit = m;
        scene.units_from = Origin::Option;
    }
    if let Some(up) = options.up_axis {
        scene.up_axis = up;
        scene.up_from = Origin::Option;
    }
    scene.prune();
    if !scene.nodes.iter().any(|n| n.body.is_some()) {
        return Err(ImportError::new(format!("{} has no mesh bodies to import", path.display())));
    }
    scene.normalise();
    scene.check()?;
    Ok(scene)
}

impl Scene {
    /// An empty scene of `format` with a root node called `name`.
    fn new(name: impl Into<String>, format: Format, meters_per_unit: f64, up_axis: UpAxis) -> Scene {
        let name = name.into();
        Scene {
            name: name.clone(),
            format,
            meters_per_unit,
            units_from: Origin::Format,
            up_axis,
            up_from: Origin::Format,
            nodes: vec![Node {
                name,
                source: "/".into(),
                parent: None,
                children: Vec::new(),
                local: DMat4::IDENTITY,
                body: None,
                hidden: false,
            }],
            meshes: Vec::new(),
            materials: Vec::new(),
            joints: Vec::new(),
            notes: Vec::new(),
            normalised: false,
        }
    }

    /// Adds a node under `parent` and returns it.
    fn add(&mut self, parent: usize, name: impl Into<String>, source: impl Into<String>, local: DMat4) -> usize {
        let i = self.nodes.len();
        self.nodes.push(Node {
            name: name.into(),
            source: source.into(),
            parent: Some(parent),
            children: Vec::new(),
            local,
            body: None,
            hidden: false,
        });
        self.nodes[parent].children.push(i);
        i
    }

    /// Adds a mesh and returns its index.
    fn add_mesh(&mut self, mesh: Mesh) -> usize {
        self.meshes.push(mesh);
        self.meshes.len() - 1
    }

    /// Adds a note, once.
    fn note(&mut self, what: impl Into<String>) {
        let what = what.into();
        if !self.notes.contains(&what) {
            self.notes.push(what);
        }
    }

    /// The node's frame in the root's.
    pub fn world(&self, node: usize) -> DMat4 {
        let mut m = self.nodes[node].local;
        let mut at = self.nodes[node].parent;
        while let Some(p) = at {
            m = self.nodes[p].local * m;
            at = self.nodes[p].parent;
        }
        m
    }

    /// The node's path from the root, its names joined by `/`, the root left out.
    pub fn path(&self, node: usize) -> String {
        let mut names = Vec::new();
        let mut at = Some(node);
        while let Some(i) = at {
            if self.nodes[i].parent.is_some() {
                names.push(self.nodes[i].name.as_str());
            }
            at = self.nodes[i].parent;
        }
        names.reverse();
        names.join("/")
    }

    /// The box around every body, in the root's frame: `None` when there is no vertex.
    pub fn bounds(&self) -> Option<[DVec3; 2]> {
        let mut lo = DVec3::splat(f64::INFINITY);
        let mut hi = DVec3::splat(f64::NEG_INFINITY);
        for (i, n) in self.nodes.iter().enumerate() {
            let Some(b) = n.body else { continue };
            let w = self.world(i);
            for p in &self.meshes[b.mesh].positions {
                let q = w.transform_point3(*p);
                lo = lo.min(q);
                hi = hi.max(q);
            }
        }
        lo.is_finite().then_some([lo, hi])
    }

    /// Drops every node with no body below it, the root kept, and the joints and
    /// meshes that lose their nodes.
    fn prune(&mut self) {
        let n = self.nodes.len();
        let mut keep = vec![false; n];
        keep[0] = true;
        for i in (0..n).rev() {
            if self.nodes[i].body.is_some() || self.nodes[i].children.iter().any(|c| keep[*c]) {
                keep[i] = true;
            }
        }
        // Parents come before children, so one forward pass keeps that order.
        let mut map = vec![usize::MAX; n];
        let mut nodes = Vec::new();
        for i in 0..n {
            let parent_kept = self.nodes[i].parent.is_none_or(|p| map[p] != usize::MAX);
            if keep[i] && parent_kept {
                map[i] = nodes.len();
                let mut node = self.nodes[i].clone();
                node.parent = node.parent.map(|p| map[p]);
                node.children.clear();
                nodes.push(node);
            }
        }
        for i in 1..nodes.len() {
            let p = nodes[i].parent.expect("only the root has no parent");
            nodes[p].children.push(i);
        }
        let dropped = self.joints.len();
        self.joints.retain_mut(|j| {
            let (p, c) = (map[j.parent], map[j.child]);
            j.parent = p;
            j.child = c;
            p != usize::MAX && c != usize::MAX
        });
        if self.joints.len() < dropped {
            self.note(format!("{} joints between nodes with no mesh were left out", dropped - self.joints.len()));
        }
        self.nodes = nodes;
        // Meshes no body uses are dropped, the rest renumbered in first-use order.
        let mut used = vec![usize::MAX; self.meshes.len()];
        let mut meshes = Vec::new();
        for node in &mut self.nodes {
            if let Some(b) = &mut node.body {
                if used[b.mesh] == usize::MAX {
                    used[b.mesh] = meshes.len();
                    meshes.push(std::mem::take(&mut self.meshes[b.mesh]));
                }
                b.mesh = used[b.mesh];
            }
        }
        self.meshes = meshes;
    }

    /// Converts the scene to millimetres with Z up: every frame, point, normal, joint and
    /// slide limit. Called once by [`read`].
    fn normalise(&mut self) {
        if self.normalised {
            return;
        }
        self.normalised = true;
        let s = self.meters_per_unit * 1000.0;
        let r = match self.up_axis {
            UpAxis::Z => DMat3::IDENTITY,
            // Y up to Z up: +Y becomes +Z and +Z becomes -Y.
            UpAxis::Y => DMat3::from_cols(DVec3::X, DVec3::Z, -DVec3::Y),
        };
        let c = DMat4::from_mat3(r) * DMat4::from_scale(DVec3::splat(s));
        let c_inv = c.inverse();
        for n in &mut self.nodes {
            n.local = c * n.local * c_inv;
        }
        for m in &mut self.meshes {
            for p in &mut m.positions {
                *p = c.transform_point3(*p);
            }
            if let Some(normals) = &mut m.corner_normals {
                for v in normals.iter_mut() {
                    *v = r * *v;
                }
            }
        }
        for j in &mut self.joints {
            j.frame = c * j.frame * c_inv;
            j.axis = (r * j.axis).normalize_or_zero();
            if j.kind == JointKind::Prismatic {
                j.limits = j.limits.map(|[a, b]| [a * s, b * s]);
                if let Some(m) = &mut j.mimic {
                    m.offset *= s;
                }
            }
        }
    }

    /// Refuses a scene that is not finite or whose indices do not hold.
    fn check(&self) -> Result<(), ImportError> {
        for (i, n) in self.nodes.iter().enumerate() {
            if !n.local.is_finite() {
                return Err(ImportError::new(format!("{} has a transform that is not finite", self.path(i))));
            }
            if let Some(b) = n.body
                && (b.mesh >= self.meshes.len() || b.material.is_some_and(|m| m >= self.materials.len()))
            {
                return Err(ImportError::new(format!("{} refers to a mesh or material it does not have", n.source)));
            }
        }
        for m in &self.meshes {
            if m.positions.iter().any(|p| !p.is_finite()) {
                return Err(ImportError::new(format!("mesh {} has a point that is not finite", m.name)));
            }
            let count = m.positions.len() as u32;
            if m.triangles.iter().flatten().any(|i| *i >= count) {
                return Err(ImportError::new(format!("mesh {} has a triangle past its points", m.name)));
            }
            if m.corner_normals.as_ref().is_some_and(|n| n.len() != m.triangles.len() * 3) {
                return Err(ImportError::new(format!("mesh {} has the wrong number of normals", m.name)));
            }
        }
        Ok(())
    }
}

/// Linear RGB to sRGB, per channel.
pub(crate) fn srgb(linear: f64) -> f64 {
    let c = linear.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// The file's stem, for a scene or node with no name.
pub(crate) fn stem(path: &Path) -> String {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported").to_owned()
}

/// Reads a unit and an up axis from a header comment, such as sand3d's "metres, Y up" or
/// SolidDraft's "millimetres".
pub(crate) fn header_hints(text: &str) -> (Option<f64>, Option<UpAxis>) {
    let t = text.to_ascii_lowercase();
    let units = [
        ("millimetre", 0.001),
        ("millimeter", 0.001),
        ("centimetre", 0.01),
        ("centimeter", 0.01),
        ("inch", 0.0254),
        ("metre", 1.0),
        ("meter", 1.0),
    ]
    .into_iter()
    .find(|(word, _)| t.contains(word))
    .map(|(_, m)| m);
    let up = if t.contains("y up") || t.contains("y-up") {
        Some(UpAxis::Y)
    } else if t.contains("z up") || t.contains("z-up") || t.contains("soliddraft") {
        Some(UpAxis::Z)
    } else {
        None
    };
    (units, up)
}

/// Triangles, the flat corner indices each uses, and how many polygons crossed
/// themselves or had no area and were fanned instead.
pub(crate) type Triangulated = (Vec<[u32; 3]>, Vec<[usize; 3]>, usize);

/// Triangles of a polygon list over `points`, each polygon cut by ear clipping in its own
/// plane, so concave polygons keep their notches and every triangle keeps the polygon's
/// winding; `flip` turns them the other way. Returns, for each triangle, the corner
/// indices it uses into the flat corner list, so per-corner data follows it. A polygon
/// that crosses itself or has no area has no right answer: it is fanned from its first
/// corner and counted. `None` when the counts need more indices than there are.
pub(crate) fn triangulate(points: &[DVec3], counts: &[usize], indices: &[u32], flip: bool) -> Option<Triangulated> {
    let mut tris = Vec::new();
    let mut corners = Vec::new();
    let mut crossed = 0;
    let mut start = 0usize;
    for &n in counts {
        if start + n > indices.len() {
            return None;
        }
        let corner: Vec<usize> = (start..start + n).collect();
        let point = |c: usize| points.get(indices[c] as usize).copied().unwrap_or(DVec3::ZERO);
        let cut = if n > 3 { ear_clip(&corner, &point) } else { None };
        let cut = cut.unwrap_or_else(|| {
            if n > 3 {
                crossed += 1;
            }
            (1..n.saturating_sub(1)).map(|k| [corner[0], corner[k], corner[k + 1]]).collect()
        });
        for [a, b, c] in cut {
            let t = if flip { [a, c, b] } else { [a, b, c] };
            tris.push(t.map(|i| indices[i]));
            corners.push(t);
        }
        start += n;
    }
    Some((tris, corners, crossed))
}

/// Newell's normal of a polygon: its area-weighted normal, following its winding.
fn polygon_normal(corner: &[usize], point: &dyn Fn(usize) -> DVec3) -> DVec3 {
    let mut n = DVec3::ZERO;
    for (i, &c) in corner.iter().enumerate() {
        let (a, b) = (point(c), point(corner[(i + 1) % corner.len()]));
        n += DVec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
    }
    n
}

/// Ear clipping of one polygon, projected onto the plane of its Newell normal so that it
/// winds counter-clockwise there. `None` when the polygon has no area or no ear can be
/// cut, as when it crosses itself.
fn ear_clip(corner: &[usize], point: &dyn Fn(usize) -> DVec3) -> Option<Vec<[usize; 3]>> {
    let normal = polygon_normal(corner, point);
    if normal.length_squared() == 0.0 || !normal.is_finite() {
        return None;
    }
    let w = normal.normalize();
    let u = w.any_orthonormal_vector();
    let v = w.cross(u);
    let flat: Vec<glam::DVec2> = corner.iter().map(|&c| glam::DVec2::new(point(c).dot(u), point(c).dot(v))).collect();
    // Twice the area of the triangle a, b, c: positive when it turns left.
    let turn = |a: usize, b: usize, c: usize| (flat[b] - flat[a]).perp_dot(flat[c] - flat[a]);
    let scale = flat.iter().fold(0.0f64, |m, p| m.max(p.abs().max_element()));
    let eps = scale * scale * 1e-12;
    let mut left: Vec<usize> = (0..corner.len()).collect();
    let mut out = Vec::with_capacity(corner.len() - 2);
    while left.len() > 3 {
        let m = left.len();
        let ear = (0..m).find(|&i| {
            let (a, b, c) = (left[(i + m - 1) % m], left[i], left[(i + 1) % m]);
            if turn(a, b, c) <= eps {
                return false;
            }
            // No other corner on or inside the ear.
            left.iter().all(|&p| {
                p == a
                    || p == b
                    || p == c
                    || flat[p] == flat[a]
                    || flat[p] == flat[b]
                    || flat[p] == flat[c]
                    || turn(a, b, p) < -eps
                    || turn(b, c, p) < -eps
                    || turn(c, a, p) < -eps
            })
        })?;
        let (a, b, c) = (left[(ear + m - 1) % m], left[ear], left[(ear + 1) % m]);
        out.push([corner[a], corner[b], corner[c]]);
        left.remove(ear);
    }
    out.push([corner[left[0]], corner[left[1]], corner[left[2]]]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_name_units_and_axes() {
        assert_eq!(header_hints("plate — written by sand3d, metres, Y up"), (Some(1.0), Some(UpAxis::Y)));
        assert_eq!(header_hints("plate — written by sand3d, metres, Z up"), (Some(1.0), Some(UpAxis::Z)));
        assert_eq!(header_hints("SolidDraft display mesh; millimetres"), (Some(0.001), Some(UpAxis::Z)));
        assert_eq!(header_hints("Exported by Foo"), (None, None));
    }

    #[test]
    fn y_up_metres_become_z_up_millimetres() {
        let mut s = Scene::new("t", Format::Gltf, 1.0, UpAxis::Y);
        let a = s.add(0, "a", "a", DMat4::from_translation(DVec3::new(0.0, 1.0, 0.0)));
        let mesh = s.add_mesh(Mesh {
            name: "m".into(),
            positions: vec![DVec3::ZERO, DVec3::X, DVec3::Z],
            triangles: vec![[0, 1, 2]],
            corner_normals: None,
        });
        s.nodes[a].body = Some(Body { mesh, material: None });
        s.normalise();
        let w = s.world(a);
        assert!(w.transform_point3(DVec3::ZERO).abs_diff_eq(DVec3::new(0.0, 0.0, 1000.0), 1e-9));
        // A point 1 m along +Z (towards the viewer in glTF) is 1000 mm along -Y.
        assert!(s.meshes[0].positions[2].abs_diff_eq(DVec3::new(0.0, -1000.0, 0.0), 1e-9));
    }

    #[test]
    fn pruning_drops_empty_groups_and_their_joints() {
        let mut s = Scene::new("t", Format::Usd, 1.0, UpAxis::Z);
        let looks = s.add(0, "Looks", "/Looks", DMat4::IDENTITY);
        let part = s.add(0, "part", "/part", DMat4::IDENTITY);
        let mesh = s.add_mesh(Mesh::default());
        s.nodes[part].body = Some(Body { mesh, material: None });
        s.joints.push(Joint {
            name: "j".into(),
            kind: JointKind::Fixed,
            parent: looks,
            child: part,
            frame: DMat4::IDENTITY,
            axis: DVec3::X,
            limits: None,
            mimic: None,
        });
        s.prune();
        assert_eq!(s.nodes.len(), 2);
        assert_eq!(s.nodes[0].children, vec![1]);
        assert_eq!(s.nodes[1].name, "part");
        assert!(s.joints.is_empty());
        assert_eq!(s.notes.len(), 1);
    }

    /// Signed areas about +Z of each triangle.
    fn areas(points: &[DVec3], tris: &[[u32; 3]]) -> Vec<f64> {
        tris.iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| points[i as usize]);
                (b - a).cross(c - a).z / 2.0
            })
            .collect()
    }

    #[test]
    fn concave_polygons_keep_their_notch() {
        // A counter-clockwise quad notched at its second corner: area 3, the point
        // (1, 0.25) outside it.
        let p = [DVec3::ZERO, DVec3::new(1.0, 1.0, 0.0), DVec3::new(3.0, 0.0, 0.0), DVec3::new(0.0, 3.0, 0.0)];
        let (t, corners, crossed) = triangulate(&p, &[4], &[0, 1, 2, 3], false).unwrap();
        assert_eq!(crossed, 0);
        let a = areas(&p, &t);
        assert!(a.iter().all(|x| (*x - 1.5).abs() < 1e-12), "{t:?} {a:?}");
        assert!(t.iter().all(|tri| tri.contains(&1) && tri.contains(&3)), "cut along the diagonal 1-3: {t:?}");
        assert_eq!(corners.len(), 2);
        // Flipped, the same cut the other way round.
        let (t, ..) = triangulate(&p, &[4], &[0, 1, 2, 3], true).unwrap();
        assert!(areas(&p, &t).iter().all(|x| (*x + 1.5).abs() < 1e-12));
        // Wound clockwise in the file, it stays clockwise.
        let (t, ..) = triangulate(&p, &[4], &[3, 2, 1, 0], false).unwrap();
        assert!(areas(&p, &t).iter().all(|x| (*x + 1.5).abs() < 1e-12), "{:?}", areas(&p, &t));
    }

    #[test]
    fn convex_and_tilted_polygons_cover_their_area() {
        let square = [DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
        let (t, ..) = triangulate(&square, &[4], &[0, 1, 2, 3], false).unwrap();
        assert!((areas(&square, &t).iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert!(areas(&square, &t).iter().all(|a| *a > 0.0));
        // An L-shaped hexagon standing in the XZ plane: area 3 by its normal.
        let l = [(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (1.0, 1.0), (1.0, 2.0), (0.0, 2.0)]
            .map(|(x, z)| DVec3::new(x, 0.0, z));
        let (t, _, crossed) = triangulate(&l, &[6], &[0, 1, 2, 3, 4, 5], false).unwrap();
        assert_eq!((t.len(), crossed), (4, 0));
        let total: f64 = t
            .iter()
            .map(|tri| {
                let [a, b, c] = tri.map(|i| l[i as usize]);
                (b - a).cross(c - a).y / -2.0
            })
            .sum();
        assert!((total - 3.0).abs() < 1e-12, "{total}");
        assert!(triangulate(&square, &[4], &[0, 1, 2], false).is_none());
    }

    #[test]
    fn a_polygon_that_crosses_itself_is_counted() {
        // A bow tie.
        let p = [DVec3::ZERO, DVec3::new(1.0, 1.0, 0.0), DVec3::X, DVec3::Y];
        let (t, _, crossed) = triangulate(&p, &[4], &[0, 1, 2, 3], false).unwrap();
        assert_eq!((t.len(), crossed), (2, 1));
    }
}
