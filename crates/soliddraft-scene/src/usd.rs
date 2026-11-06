//! OpenUSD (`.usd`, `.usda`, `.usdc`, `.usdz`): the composed stage under its default prim.
//!
//! The stage is composed by the `openusd` crate (references, payloads, variants,
//! instancing). Every imageable prim under the default prim is a node at its local
//! transform; meshes and the cube, sphere and cylinder shapes are bodies. Material
//! bindings resolve to UsdPreviewSurface or OmniPBR constants. UsdPhysics joints become
//! joints between the nodes of their two bodies; one with no body0 hangs from the root
//! at its frame in the stage's world. Stage units and up axis come from the
//! root layer's `metersPerUnit` and `upAxis`, or USD's fallbacks of centimetres and Y up.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;

use glam::{DMat4, DQuat, DVec3};
use openusd::sdf::{self, Value};
use openusd::usd::{self, Prim, Stage};
use openusd_schemas::geom::XformCache;

use crate::{
    Body, Format, ImportError, Joint, JointKind, Material, Mesh, Mimic, Origin, Scene, UpAxis, shapes, srgb,
    triangulate,
};

thread_local! {
    /// The time code every attribute is read at: the stage's start time code, which is
    /// the default value wherever an attribute has no time samples.
    static TIME: Cell<f64> = const { Cell::new(0.0) };
}

fn time() -> usd::TimeCode {
    usd::TimeCode::new(TIME.get())
}

fn err(path: &Path, e: impl std::fmt::Display) -> ImportError {
    ImportError(format!("{}: {e}", path.display()))
}

pub(crate) fn read(path: &Path) -> Result<Scene, ImportError> {
    let file = path.to_str().ok_or_else(|| ImportError(format!("{} is not a UTF-8 path", path.display())))?;
    let stage = Stage::builder()
        .schema_registry(openusd_schemas::schema_registry())
        .open(file)
        .map_err(|e| ImportError(format!("{} is not a USD file this import reads: {e}", path.display())))?;
    let root_path = sdf::Path::abs_root();
    let mpu = stage.field::<Value>(root_path.clone(), "metersPerUnit").ok().flatten().as_ref().and_then(float);
    let up =
        stage.field::<Value>(root_path.clone(), "upAxis").ok().flatten().as_ref().and_then(token).and_then(|t| match t
            .as_str()
        {
            "Y" => Some(UpAxis::Y),
            "Z" => Some(UpAxis::Z),
            _ => None,
        });
    let pseudo = stage.prim(root_path).map_err(|e| err(path, e))?;
    let top: Vec<Prim> = match stage.default_prim() {
        Some(name) => vec![stage.prim(format!("/{}", name.as_str())).map_err(|e| err(path, e))?],
        None => pseudo.children().map_err(|e| err(path, e))?,
    };
    let name = match top.as_slice() {
        [one] => one.path().name().unwrap_or_default().to_owned(),
        _ => crate::stem(path),
    };
    let mut scene = Scene::new(name, Format::Usd, mpu.filter(|m| *m > 0.0).unwrap_or(0.01), up.unwrap_or(UpAxis::Y));
    scene.units_from = if mpu.is_some() { Origin::File } else { Origin::Fallback };
    scene.up_from = if up.is_some() { Origin::File } else { Origin::Fallback };
    TIME.set(stage.start_time_code());
    if stage.has_authored_time_code_range() && stage.end_time_code() != stage.start_time_code() {
        scene.note(format!("the stage is animated; it was read at its start time code, {}", stage.start_time_code()));
    }
    let mut reader = Reader {
        xforms: XformCache::new(time()),
        materials: HashMap::new(),
        meshes: HashMap::new(),
        nodes: HashMap::new(),
        file: path,
    };
    match top.as_slice() {
        [one] => {
            let (local, _) = reader.xforms.local_transformation(one).map_err(|e| err(path, e))?;
            scene.nodes[0].local = matrix(&local);
            scene.nodes[0].source = one.path().to_string();
            reader.nodes.insert(one.path().to_string(), 0);
            // The default prim may be a mesh itself, or bind the material its children use.
            let kind = one.type_name().ok().flatten().map(|t| t.as_str().to_owned()).unwrap_or_default();
            let binding = bound_material(one);
            if let Some(mesh) = reader.body(&mut scene, one, &kind)? {
                let material = reader.body_material(&mut scene, one, &binding)?;
                scene.nodes[0].body = Some(Body { mesh, material });
            }
            scene.nodes[0].hidden = attr_token(one, "visibility").as_deref() == Some("invisible");
            reader.visit_children(&mut scene, one, one, 0, binding)?;
        }
        many => {
            for p in many {
                reader.visit(&mut scene, p, p, 0, None)?;
            }
        }
    }
    reader.joints(&mut scene, &stage)?;
    Ok(scene)
}

/// What the walk keeps between prims.
struct Reader<'a> {
    xforms: XformCache,
    /// Materials read, by prim path.
    materials: HashMap<String, Option<usize>>,
    /// Meshes read, by the prim path that holds them, so instances share one.
    meshes: HashMap<String, Option<usize>>,
    /// The node made for each prim path.
    nodes: HashMap<String, usize>,
    file: &'a Path,
}

/// Prim types that are not part of the tree.
const SKIPPED: &[&str] =
    &["Material", "Shader", "NodeGraph", "GeomSubset", "Camera", "PhysicsScene", "Skeleton", "SkelAnimation"];

impl Reader<'_> {
    fn visit_children(
        &mut self,
        scene: &mut Scene,
        prim: &Prim,
        // The prim whose path names the nodes: an instance, while its prototype is read.
        named: &Prim,
        node: usize,
        binding: Option<String>,
    ) -> Result<(), ImportError> {
        // An instance's children are its prototype's.
        let source = match prim.is_instance() {
            Ok(true) => match prim.prototype() {
                Ok(Some(proto)) => prim.stage().prim(proto).map_err(|e| err(self.file, e))?,
                _ => prim.clone(),
            },
            _ => prim.clone(),
        };
        for child in source.children().map_err(|e| err(self.file, e))? {
            let name = child.path().name().unwrap_or_default().to_owned();
            let named_child = named.stage().prim(format!("{}/{name}", named.path())).map_err(|e| err(self.file, e))?;
            self.visit(scene, &child, &named_child, node, binding.clone())?;
        }
        Ok(())
    }

    fn visit(
        &mut self,
        scene: &mut Scene,
        prim: &Prim,
        named: &Prim,
        parent: usize,
        binding: Option<String>,
    ) -> Result<(), ImportError> {
        // Inactive prims, classes and `over`s that nothing defines are not on the stage.
        if !prim.is_active().unwrap_or(true)
            || prim.is_abstract().unwrap_or(false)
            || !prim.is_defined().unwrap_or(true)
        {
            return Ok(());
        }
        let kind = prim.type_name().ok().flatten().map(|t| t.as_str().to_owned()).unwrap_or_default();
        if SKIPPED.contains(&kind.as_str()) || kind.ends_with("Light") || kind.starts_with("Physics") {
            return Ok(());
        }
        if kind == "PointInstancer" {
            // Its children are the prototypes it scatters, not geometry where they stand.
            scene.note("point instancers are not imported, with their prototypes");
            return Ok(());
        }
        if kind == "SkelRoot" {
            scene.note("skinned meshes are shown in their rest pose; skeletons and animation are not imported");
        }
        match attr_token(prim, "purpose").as_deref() {
            Some("guide") => return Ok(()),
            Some("proxy") => {
                scene.note("proxy geometry is left out; render and default geometry come in");
                return Ok(());
            }
            _ => {}
        }
        let (local, resets) = self.xforms.local_transformation(prim).map_err(|e| err(self.file, e))?;
        let mut local = matrix(&local);
        if resets {
            // The prim ignores its parents' frames: make its frame relative to the parent's.
            local = scene.world(parent).inverse() * local;
        }
        let name = named.path().name().unwrap_or_default().to_owned();
        let node = scene.add(parent, name, named.path().to_string(), local);
        self.nodes.insert(named.path().to_string(), node);
        scene.nodes[node].hidden = attr_token(prim, "visibility").as_deref() == Some("invisible");
        let binding = bound_material(prim).or(binding);
        if let Some(mesh) = self.body(scene, prim, &kind)? {
            let material = self.body_material(scene, prim, &binding)?;
            scene.nodes[node].body = Some(Body { mesh, material });
        }
        self.visit_children(scene, prim, named, node, binding)
    }

    /// The mesh of a gprim, read once per prim (and once per prototype prim).
    fn body(&mut self, scene: &mut Scene, prim: &Prim, kind: &str) -> Result<Option<usize>, ImportError> {
        let key = prim.path().to_string();
        if let Some(m) = self.meshes.get(&key) {
            return Ok(*m);
        }
        let name = prim.path().name().unwrap_or_default().to_owned();
        let mesh = match kind {
            "Mesh" => mesh(scene, prim, &name)?,
            "Cube" => {
                let size = attr_float(prim, "size").unwrap_or(2.0);
                Some(shapes::cuboid(&name, DVec3::splat(size)))
            }
            "Sphere" => Some(shapes::sphere(&name, attr_float(prim, "radius").unwrap_or(1.0))),
            "Cylinder" => {
                let (r, h) = (attr_float(prim, "radius").unwrap_or(1.0), attr_float(prim, "height").unwrap_or(2.0));
                let mut m = shapes::cylinder(&name, r, h);
                turn_to_axis(&mut m, attr_token(prim, "axis").as_deref());
                Some(m)
            }
            "Cone" | "Capsule" | "Capsule_1" | "Cylinder_1" | "Plane" | "BasisCurves" | "HermiteCurves"
            | "NurbsCurves" | "NurbsPatch" | "Points" | "TetMesh" => {
                scene.note(format!("{kind} prims are not imported"));
                None
            }
            _ => None,
        };
        let index = mesh.map(|m| scene.add_mesh(m));
        self.meshes.insert(key, index);
        Ok(index)
    }

    /// A body's material: the one bound to it when this import reads it, else its
    /// constant display colour.
    fn body_material(
        &mut self,
        scene: &mut Scene,
        prim: &Prim,
        binding: &Option<String>,
    ) -> Result<Option<usize>, ImportError> {
        if let Some(path) = binding
            && let Some(m) = self.material(scene, prim, path)?
        {
            return Ok(Some(m));
        }
        Ok(display_color(scene, prim).map(|m| match scene.materials.iter().position(|x| *x == m) {
            Some(i) => i,
            None => {
                scene.materials.push(m);
                scene.materials.len() - 1
            }
        }))
    }

    /// The material at `path` bound to `prim`, read once.
    fn material(&mut self, scene: &mut Scene, prim: &Prim, path: &str) -> Result<Option<usize>, ImportError> {
        if let Some(m) = self.materials.get(path) {
            return Ok(*m);
        }
        let material = prim.stage().prim(path).ok().and_then(|m| read_material(&m));
        if material.is_none() {
            scene.note(
                "materials whose surface is neither UsdPreviewSurface nor OmniPBR are not read; \
                 their bodies take their display colour or the default colour",
            );
        }
        let index = material.map(|m| {
            scene.materials.push(m);
            scene.materials.len() - 1
        });
        self.materials.insert(path.to_owned(), index);
        Ok(index)
    }

    /// Every UsdPhysics joint on the stage between two imported nodes.
    fn joints(&mut self, scene: &mut Scene, stage: &Stage) -> Result<(), ImportError> {
        let mut paths = Vec::new();
        stage.traverse(usd::PrimPredicate::DEFAULT, |p| paths.push(p.clone())).map_err(|e| err(self.file, e))?;
        let mut by_name: HashMap<String, String> = HashMap::new();
        for path in &paths {
            let prim = stage.prim(path.clone()).map_err(|e| err(self.file, e))?;
            let kind = prim.type_name().ok().flatten().map(|t| t.as_str().to_owned()).unwrap_or_default();
            let joint_kind = match kind.as_str() {
                "PhysicsRevoluteJoint" => JointKind::Revolute,
                "PhysicsPrismaticJoint" => JointKind::Prismatic,
                "PhysicsFixedJoint" => JointKind::Fixed,
                "PhysicsSphericalJoint" => JointKind::Spherical,
                "PhysicsJoint" | "PhysicsDistanceJoint" => JointKind::Other,
                _ => continue,
            };
            let body = |rel: &str| {
                prim.relationship(rel).targets().ok().and_then(|t| t.into_iter().next()).map(|p| p.to_string())
            };
            let (b0, b1) = (body("physics:body0"), body("physics:body1"));
            let find = |p: &Option<String>| -> Option<usize> {
                let mut at = p.clone()?;
                loop {
                    if let Some(n) = self.nodes.get(&at) {
                        return Some(*n);
                    }
                    at = at.rsplit_once('/').map(|(head, _)| head.to_owned()).filter(|h| !h.is_empty())?;
                }
            };
            let parent = if b0.is_none() { Some(0) } else { find(&b0) };
            let (Some(parent), Some(child)) = (parent, find(&b1)) else {
                scene.note("joints whose bodies were not imported were left out");
                continue;
            };
            let pos = attr_value(&prim, "physics:localPos0").as_ref().and_then(vec3).unwrap_or(DVec3::ZERO);
            let rot = attr_value(&prim, "physics:localRot0").as_ref().and_then(quat).unwrap_or(DQuat::IDENTITY);
            let frame = DMat4::from_rotation_translation(rot.normalize(), pos);
            // With no body0 the joint is anchored to the stage's world frame, which the
            // root's own placement does not move; with the root as body0 it is the root's.
            let root = scene.world(0);
            let frame = if b0.is_none() && root.determinant() != 0.0 { root.inverse() * frame } else { frame };
            let axis = match attr_token(&prim, "physics:axis").as_deref() {
                Some("Y") => DVec3::Y,
                Some("Z") => DVec3::Z,
                _ => DVec3::X,
            };
            let lo = attr_float(&prim, "physics:lowerLimit");
            let hi = attr_float(&prim, "physics:upperLimit");
            let (kind, limits) = match (joint_kind, lo, hi) {
                (JointKind::Revolute, Some(a), Some(b)) if a.is_finite() && b.is_finite() => {
                    (JointKind::Revolute, Some([a, b]))
                }
                (JointKind::Revolute, ..) => (JointKind::Continuous, None),
                (JointKind::Prismatic, Some(a), Some(b)) if a.is_finite() && b.is_finite() => {
                    (JointKind::Prismatic, Some([a, b]))
                }
                (k, ..) => (k, None),
            };
            let name = path.name().unwrap_or_default().to_owned();
            let mimic = prim.relationship("newton:mimicJoint").targets().ok().and_then(|t| t.into_iter().next()).map(
                |leader| Mimic {
                    joint: leader.name().unwrap_or_default().to_owned(),
                    multiplier: attr_float(&prim, "newton:mimicCoef1").unwrap_or(1.0),
                    offset: attr_float(&prim, "newton:mimicCoef0").unwrap_or(0.0),
                },
            );
            by_name.insert(path.to_string(), name.clone());
            scene.joints.push(Joint { name, kind, parent, child, frame, axis, limits, mimic });
        }
        Ok(())
    }
}

/// A USD row-vector matrix as a column-vector one.
fn matrix(m: &openusd::gf::Matrix4d) -> DMat4 {
    DMat4::from_cols_array(&m.0)
}

/// Turns a Z-axis cylinder to USD's `axis` token.
fn turn_to_axis(mesh: &mut Mesh, axis: Option<&str>) {
    let turn = match axis {
        Some("X") => DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2),
        Some("Y") => DQuat::from_rotation_x(-std::f64::consts::FRAC_PI_2),
        _ => return,
    };
    for p in &mut mesh.positions {
        *p = turn * *p;
    }
}

/// The triangles of a Mesh prim, with the normals it authors.
fn mesh(scene: &mut Scene, prim: &Prim, name: &str) -> Result<Option<Mesh>, ImportError> {
    let Some(points) = attr_value(prim, "points").as_ref().and_then(vec3s) else { return Ok(None) };
    let counts: Vec<usize> = attr_value(prim, "faceVertexCounts")
        .as_ref()
        .and_then(ints)
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.max(0) as usize)
        .collect();
    let indices: Vec<u32> = attr_value(prim, "faceVertexIndices")
        .as_ref()
        .and_then(ints)
        .unwrap_or_default()
        .into_iter()
        .map(|i| i.max(0) as u32)
        .collect();
    let left = attr_token(prim, "orientation").as_deref() == Some("leftHanded");
    if indices.iter().any(|i| *i as usize >= points.len()) {
        return Err(ImportError(format!("mesh {} has an index past its points", prim.path())));
    }
    let Some((mut triangles, mut corners, crossed)) = triangulate(&points, &counts, &indices, left) else {
        return Err(ImportError(format!("mesh {} has fewer indices than its faces need", prim.path())));
    };
    if crossed > 0 {
        scene.note("polygons that cross themselves or have no area are split into fans from their first corner");
    }
    // Which polygon each flat corner belongs to.
    let mut face_of = Vec::with_capacity(indices.len());
    for (f, &n) in counts.iter().enumerate() {
        face_of.extend(std::iter::repeat_n(f, n));
    }
    // Faces named in `holeIndices` are not drawn.
    let holes: std::collections::HashSet<usize> = attr_value(prim, "holeIndices")
        .as_ref()
        .and_then(ints)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|h| usize::try_from(h).ok())
        .collect();
    if !holes.is_empty() {
        let keep: Vec<bool> = corners.iter().map(|c| !holes.contains(&face_of[c[0]])).collect();
        let mut k = keep.iter();
        triangles.retain(|_| *k.next().unwrap());
        let mut k = keep.iter();
        corners.retain(|_| *k.next().unwrap());
    }
    if triangles.is_empty() {
        scene.note("meshes with no faces to draw were left out");
        return Ok(None);
    }
    // Subdivision surfaces come in as their control polygons; bilinear is the polygons.
    if let Some(scheme) = attr_token(prim, "subdivisionScheme").filter(|s| s == "catmullClark" || s == "loop") {
        scene.note(format!(
            "meshes marked for {scheme} subdivision (USD's fallback when none is named) are shown as their \
             control polygons, not subdivided"
        ));
    }
    let subsets = prim.children().unwrap_or_default().into_iter().any(|c| {
        c.type_name().ok().flatten().is_some_and(|t| t.as_str() == "GeomSubset")
            && (attr_token(&c, "familyName").as_deref() == Some("materialBind") || bound_material(&c).is_some())
    });
    if subsets {
        scene.note("per-face materials (GeomSubset) are not imported; each mesh takes its own material");
    }
    // Normals: `primvars:normals` wins over `normals`; their interpolation says what they
    // are per.
    let normals = ["primvars:normals", "normals"].into_iter().find_map(|n| {
        let attr = prim.attribute(n);
        let values = attr.get_at::<Value>(time()).ok().flatten().as_ref().and_then(vec3s)?;
        let interp = attr.get_metadata::<Value>("interpolation").ok().flatten().as_ref().and_then(token);
        let idx = prim.attribute(format!("{n}:indices")).get_at::<Value>(time()).ok().flatten().as_ref().and_then(ints);
        Some((values, interp.unwrap_or_else(|| "vertex".into()), idx))
    });
    let corner_normals = normals.and_then(|(values, interp, idx)| {
        let at = |k: usize| -> Option<DVec3> {
            let k = match &idx {
                Some(i) => usize::try_from(*i.get(k)?).ok()?,
                None => k,
            };
            values.get(k).copied()
        };
        let mut out = Vec::with_capacity(triangles.len() * 3);
        for (t, c) in triangles.iter().zip(&corners) {
            for k in 0..3 {
                let v = match interp.as_str() {
                    "faceVarying" => at(c[k]),
                    "vertex" | "varying" => at(t[k] as usize),
                    "uniform" => at(*face_of.get(c[k])?),
                    "constant" => at(0),
                    _ => None,
                }?;
                // An authored normal is a direction in the mesh's frame: the orientation
                // turns the winding, not the normals.
                out.push(v);
            }
        }
        Some(out)
    });
    Ok(Some(Mesh { name: name.to_owned(), positions: points, triangles, corner_normals }))
}

/// A gprim's constant `primvars:displayColor` and `displayOpacity` as a material.
fn display_color(scene: &mut Scene, prim: &Prim) -> Option<Material> {
    let attr = prim.attribute("primvars:displayColor");
    let colours = attr.get_at::<Value>(time()).ok().flatten().as_ref().and_then(vec3s)?;
    let interp = attr.get_metadata::<Value>("interpolation").ok().flatten().as_ref().and_then(token);
    let constant = interp.as_deref().is_none_or(|i| i == "constant") || colours.len() == 1;
    let (Some(c), true) = (colours.first(), constant) else {
        scene.note("per-face and per-vertex display colours are not imported");
        return None;
    };
    let opacity = attr_value(prim, "primvars:displayOpacity")
        .as_ref()
        .and_then(|v| match v {
            Value::FloatVec(f) => f.first().map(|x| f64::from(*x)),
            Value::DoubleVec(d) => d.first().copied(),
            v => float(v),
        })
        .unwrap_or(1.0);
    let base_color = [srgb(c.x), srgb(c.y), srgb(c.z), opacity];
    let hex = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(Material {
        name: format!("displayColor #{:02x}{:02x}{:02x}", hex(base_color[0]), hex(base_color[1]), hex(base_color[2])),
        base_color,
        metallic: 0.0,
        roughness: 0.5,
        textured: false,
    })
}

/// The path of the material bound directly to `prim`, if any.
fn bound_material(prim: &Prim) -> Option<String> {
    let targets = prim.relationship("material:binding").targets().ok()?;
    targets.into_iter().next().map(|p| p.to_string())
}

/// A Material prim's constant look: UsdPreviewSurface or OmniPBR.
fn read_material(material: &Prim) -> Option<Material> {
    let name = material.path().name().unwrap_or_default().to_owned();
    for output in ["outputs:surface", "outputs:mdl:surface"] {
        let Ok(connections) = material.attribute(output).connections() else { continue };
        let Some(source) = connections.into_iter().next() else { continue };
        let shader = material.stage().prim(source.prim_path()).ok()?;
        let id = attr_token(&shader, "info:id").unwrap_or_default();
        let mdl = attr_value(&shader, "info:mdl:sourceAsset");
        let preview = id == "UsdPreviewSurface";
        let omni = mdl.is_some() || id.starts_with("Omni");
        if !(preview || omni) {
            continue;
        }
        let (color, metal, rough, texture) = if preview {
            ("inputs:diffuseColor", "inputs:metallic", "inputs:roughness", None)
        } else {
            (
                "inputs:diffuse_color_constant",
                "inputs:metallic_constant",
                "inputs:reflection_roughness_constant",
                Some("inputs:diffuse_texture"),
            )
        };
        let c = attr_value(&shader, color).as_ref().and_then(vec3).unwrap_or(DVec3::splat(if preview {
            0.18
        } else {
            0.2
        }));
        let opacity = if preview { attr_float(&shader, "inputs:opacity").unwrap_or(1.0) } else { 1.0 };
        let textured = shader.attribute(color).connections().is_ok_and(|c| !c.is_empty())
            || texture.is_some_and(|t| attr_value(&shader, t).is_some());
        return Some(Material {
            name,
            base_color: [srgb(c.x), srgb(c.y), srgb(c.z), opacity],
            metallic: attr_float(&shader, metal).unwrap_or(0.0),
            roughness: attr_float(&shader, rough).unwrap_or(0.5),
            textured,
        });
    }
    None
}

fn attr_value(prim: &Prim, name: &str) -> Option<Value> {
    prim.attribute(name).get_at::<Value>(time()).ok().flatten()
}

fn attr_float(prim: &Prim, name: &str) -> Option<f64> {
    attr_value(prim, name).as_ref().and_then(float)
}

fn attr_token(prim: &Prim, name: &str) -> Option<String> {
    attr_value(prim, name).as_ref().and_then(token)
}

fn float(v: &Value) -> Option<f64> {
    match v {
        Value::Float(f) => Some(f64::from(*f)),
        Value::Double(d) => Some(*d),
        Value::Half(h) => Some(h.to_f64()),
        Value::Int(i) => Some(f64::from(*i)),
        _ => None,
    }
}

fn token(v: &Value) -> Option<String> {
    match v {
        Value::Token(t) => Some(t.as_str().to_owned()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn ints(v: &Value) -> Option<Vec<i64>> {
    match v {
        Value::IntVec(v) => Some(v.iter().map(|i| i64::from(*i)).collect()),
        Value::Int64Vec(v) => Some(v.clone()),
        Value::UintVec(v) => Some(v.iter().map(|i| i64::from(*i)).collect()),
        _ => None,
    }
}

fn vec3(v: &Value) -> Option<DVec3> {
    match v {
        Value::Vec3f(p) => Some(DVec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z))),
        Value::Vec3d(p) => Some(DVec3::new(p.x, p.y, p.z)),
        Value::Vec3h(p) => Some(DVec3::new(p.x.to_f64(), p.y.to_f64(), p.z.to_f64())),
        _ => None,
    }
}

fn vec3s(v: &Value) -> Option<Vec<DVec3>> {
    match v {
        Value::Vec3fVec(ps) => {
            Some(ps.iter().map(|p| DVec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z))).collect())
        }
        Value::Vec3dVec(ps) => Some(ps.iter().map(|p| DVec3::new(p.x, p.y, p.z)).collect()),
        Value::Vec3hVec(ps) => Some(ps.iter().map(|p| DVec3::new(p.x.to_f64(), p.y.to_f64(), p.z.to_f64())).collect()),
        _ => None,
    }
}

fn quat(v: &Value) -> Option<DQuat> {
    match v {
        Value::Quatf(q) => Some(DQuat::from_xyzw(f64::from(q.x), f64::from(q.y), f64::from(q.z), f64::from(q.w))),
        Value::Quatd(q) => Some(DQuat::from_xyzw(q.x, q.y, q.z, q.w)),
        Value::Quath(q) => Some(DQuat::from_xyzw(q.x.to_f64(), q.y.to_f64(), q.z.to_f64(), q.w.to_f64())),
        _ => None,
    }
}
