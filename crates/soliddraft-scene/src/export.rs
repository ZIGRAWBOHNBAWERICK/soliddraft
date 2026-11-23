//! Assemblies written for robotics and simulation: URDF and OpenUSD with joints.
//!
//! Every shown or hidden part occurrence of the assembly becomes a link, with its display
//! mesh as the link's visual and collision geometry. The link nearest the ground (the part
//! with an Origin mate, else the first component) hangs from `base_link`, which sits at the
//! assembly's origin. Mates become joints:
//!
//! | Mates between two parts | Joint |
//! |---|---|
//! | Hinge with angle limits | revolute, between the limits |
//! | Hinge without limits | continuous |
//! | Concentric between axes, without Lock rotation | continuous about the axis |
//! | Concentric between axes, with Lock rotation | prismatic along the axis |
//! | Coincident or Distance between two planes | planar, normal to the planes |
//! | Coincident or Concentric between two points | spherical, about the common point |
//! | Anything else | fixed, at the pose the assembly holds |
//!
//! The prismatic, planar and spherical mappings apply to a pair held by that single
//! mate. Other constraints between the same bodies can remove more freedoms.
//!
//! A Gear mate between two of those joints makes the second follow the first (URDF
//! `mimic`). A disconnected group starts with a fixed joint to `base_link` and keeps its
//! internal joints, including mates authored in sub-assemblies. A mate that would close
//! a loop is left out and named in [`Articulation::notes`]. Lengths are written in
//! metres for URDF and millimetres for OpenUSD (`metersPerUnit = 0.001`, Z up). Each
//! joint frame puts the joint's axis along its X, as UsdPhysics wants.
//!
//! What a joint carries is read from the mates' stored axes, in the part's frame, placed
//! where the assembly places the part; the export does not solve anything.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use glam::{DMat4, DQuat, DVec3};
use serde::Serialize;
use soliddraft_assembly::mates::Mechanism;
use soliddraft_assembly::{
    AssemblyDocument, DisplayMesh, Entity, FaceEndpoint, Flags, Mate, MateId, OccurrenceIndex, OccurrencePath,
    Placement, Step, StoredSupport,
};
use soliddraft_kernel_api::{EnclosedSupport, ExactRational, SupportBounds};

use crate::JointKind;

/// An assembly as links and joints, ready to write.
#[derive(Clone, Debug, Serialize)]
pub struct Articulation {
    /// The robot's name: the assembly's.
    pub name: String,
    /// The links, `base_link` first.
    pub links: Vec<Link>,
    /// The joints; each link but `base_link` is the child of exactly one.
    pub joints: Vec<ExportJoint>,
    /// The meshes the links show, one per part revision.
    #[serde(skip)]
    pub meshes: Vec<ExportMesh>,
    /// What was carried as something less than the assembly says, in words.
    pub notes: Vec<String>,
}

/// One link.
#[derive(Clone, Debug, Serialize)]
pub struct Link {
    /// A name unique in the file.
    pub name: String,
    /// The occurrence it is, `None` for `base_link`.
    pub path: Option<String>,
    /// Its mesh, `None` for `base_link`.
    pub mesh: Option<usize>,
    /// The mesh's frame in the link's, millimetres.
    #[serde(skip)]
    pub visual: DMat4,
    /// Its colour, sRGB 0 to 1, when the assembly gives it one.
    pub colour: Option<[f64; 3]>,
}

/// One joint.
#[derive(Clone, Debug, Serialize)]
pub struct ExportJoint {
    /// A name unique in the file.
    pub name: String,
    /// How the child can move relative to the parent.
    pub kind: JointKind,
    /// The parent link.
    pub parent: usize,
    /// The child link.
    pub child: usize,
    /// The joint's frame in the parent link's, millimetres; the axis is its X.
    #[serde(skip)]
    pub frame: DMat4,
    /// The axis in the assembly's frame, for the record. X when the joint has no axis.
    pub axis: [f64; 3],
    /// Lower and upper limits, in degrees for a revolute joint or millimetres for a slide.
    pub limits: Option<[f64; 2]>,
    /// The mates it comes from.
    pub mates: Vec<String>,
    /// The joint it follows and how: (leader, multiplier).
    pub mimic: Option<(String, f64)>,
}

/// A mesh, millimetres, in its part's frame.
#[derive(Clone, Debug)]
pub struct ExportMesh {
    /// A file-safe name.
    pub name: String,
    /// Vertex positions.
    pub positions: Vec<DVec3>,
    /// Triangles.
    pub triangles: Vec<[u32; 3]>,
}

fn mat(p: &Placement) -> DMat4 {
    DMat4::from_rotation_translation(p.rotation, p.translation)
}

fn safe(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let s = s.trim_matches('_').to_owned();
    match s.chars().next() {
        Some(c) if c.is_ascii_alphabetic() => s,
        _ => format!("n_{s}"),
    }
}

fn enclosed_vector(bounds: &[SupportBounds; 3]) -> DVec3 {
    let f = |r: &ExactRational| {
        r.numerator.parse::<f64>().unwrap_or(f64::NAN) / r.denominator.parse::<f64>().unwrap_or(f64::NAN)
    };
    DVec3::from_array(bounds.each_ref().map(|b| f(&b.lo) * 0.5 + f(&b.hi) * 0.5))
}

/// An endpoint's axis, placed: a point on it and its unit direction in the assembly's frame.
fn axis_of(end: &FaceEndpoint, world: Placement) -> Option<(DVec3, DVec3)> {
    let (through, direction) = match end.support.as_ref()? {
        StoredSupport::Axis { direction, through, .. } => {
            (DVec3::from_array(*through), DVec3::from_array(direction.map(f64::from)))
        }
        StoredSupport::Enclosed { support } => match support.as_ref() {
            EnclosedSupport::Axis { direction, through, .. } => (enclosed_vector(through), enclosed_vector(direction)),
            _ => return None,
        },
        _ => return None,
    };
    let point = world.apply(through);
    let direction = world.rotate(direction).try_normalize()?;
    point.is_finite().then_some((point, direction))
}

fn point_of(end: &FaceEndpoint, world: DMat4) -> Option<DVec3> {
    let point = match end.support.as_ref()? {
        StoredSupport::Point { at, .. } => DVec3::from_array(*at),
        StoredSupport::Enclosed { support } => match support.as_ref() {
            EnclosedSupport::Point { at, .. } => enclosed_vector(at),
            _ => return None,
        },
        _ => return None,
    };
    let point = world.transform_point3(point);
    point.is_finite().then_some(point)
}

fn plane_of(end: &FaceEndpoint, world: DMat4) -> Option<(DVec3, DVec3)> {
    let (point, normal) = match (&end.entity, &end.support) {
        (_, Some(StoredSupport::Enclosed { support })) => match support.as_ref() {
            EnclosedSupport::Plane { normal, through } => (enclosed_vector(through), enclosed_vector(normal)),
            _ => return None,
        },
        (Entity::Plane(plane), None) => (DVec3::ZERO, DVec3::from_array(plane.normal().map(f64::from))),
        (Entity::Face(face), None) => {
            let at = face.fingerprint.centroid;
            let n = face.fingerprint.normal?;
            (DVec3::new(at.x, at.y, at.z), DVec3::new(n.x, n.y, n.z))
        }
        _ => return None,
    };
    let point = world.transform_point3(point);
    let normal = world.transform_vector3(normal).try_normalize()?;
    point.is_finite().then_some((point, normal))
}

/// A frame at `at` whose X is `axis` and whose other axes follow `like` as closely as
/// they can.
fn axis_frame(at: DVec3, axis: DVec3, like: DQuat) -> DMat4 {
    let turn = DQuat::from_rotation_arc(like * DVec3::X, axis);
    DMat4::from_rotation_translation((turn * like).normalize(), at)
}

/// Builds the articulation of the open assembly at its accepted placements.
pub fn articulate(doc: &mut AssemblyDocument) -> Result<Articulation, String> {
    doc.finish_loading();
    let table = doc.table();
    let accepted = doc.accepted_worlds();
    let mut notes = Vec::new();

    // Links: every part or mesh occurrence that is not suppressed.
    let mut worlds = Vec::new();
    let mut bodies: HashMap<OccurrencePath, usize> = HashMap::new();
    let mut links =
        vec![Link { name: "base_link".into(), path: None, mesh: None, visual: DMat4::IDENTITY, colour: None }];
    let mut meshes: Vec<ExportMesh> = Vec::new();
    let mut mesh_of: HashMap<usize, usize> = HashMap::new();
    let mut names: HashSet<String> = HashSet::from(["base_link".to_owned()]);
    for row in (1..table.len() as u32).map(OccurrenceIndex) {
        let f = table.flags(row);
        if f.has(Flags::ASSEMBLY) || f.has(Flags::SUPPRESSED) || f.has(Flags::REMOVED) {
            continue;
        }
        let def = doc.store().get(table.definition(row));
        let Some(mesh) = def.mesh.as_ref() else {
            notes.push(format!("{} has no display mesh loaded and is left out", doc.label(row)));
            continue;
        };
        let d = table.definition(row).0 as usize;
        let m = *mesh_of.entry(d).or_insert_with(|| {
            meshes.push(export_mesh(&format!("{}_{}", safe(&def.label), meshes.len()), mesh));
            meshes.len() - 1
        });
        let path = table.path(row);
        let mut name = safe(&doc.label(row));
        while !names.insert(name.clone()) {
            name = format!("{name}_{}", path.to_string().replace(['/', ':', '.'], "_"));
        }
        let colour = table.colour(row).map(|a| {
            let c = a.base_color;
            [c.r as f64 / 255.0, c.g as f64 / 255.0, c.b as f64 / 255.0]
        });
        bodies.insert(path.clone(), worlds.len());
        worlds.push(accepted[row.0 as usize]);
        links.push(Link { name, path: Some(path.to_string()), mesh: Some(m), visual: DMat4::IDENTITY, colour });
    }
    if worlds.is_empty() {
        return Err("the assembly has no parts to export".into());
    }

    // What each pair of parts is held by.
    let holders = doc.mate_holders();
    let scoped_mates: Vec<_> =
        holders.iter().flat_map(|(path, assembly)| assembly.known_mates().map(move |mate| (path, mate))).collect();
    let body_of = |prefix: &OccurrencePath, path: &OccurrencePath| {
        bodies.get(&OccurrencePath([prefix.steps(), path.steps()].concat())).copied()
    };
    let placement = |prefix: &OccurrencePath, end: &FaceEndpoint| worlds[body_of(prefix, &end.path).unwrap()];
    let mut pairs: BTreeMap<_, Vec<_>> = BTreeMap::new();
    let mut origins = Vec::new();
    let mut grounded = vec![false; worlds.len()];
    for &(prefix, mate) in &scoped_mates {
        if let Mate::Origin { component, .. } = mate {
            let p = prefix.child(Step::component(*component));
            if let Some(&body) = bodies.get(&p) {
                grounded[body] = true;
                origins.push(body);
            }
            continue;
        }
        if matches!(mate, Mate::Mechanical { .. }) {
            continue;
        }
        let mut ends: Vec<_> = mate.ends().into_iter().map(|e| e.path).collect();
        ends.sort();
        ends.dedup();
        match ends[..] {
            [a, b] => {
                if let (Some(a), Some(b)) = (body_of(prefix, a), body_of(prefix, b)) {
                    pairs.entry((a.min(b), a.max(b))).or_default().push((prefix, mate));
                }
            }
            [] | [_] => {}
            _ => notes.push(format!("{} ties more than two parts and is left out", mate.name())),
        }
    }

    // A joint per pair, as the mates say.
    struct Edge<'a> {
        kind: JointKind,
        axis: Option<(DVec3, DVec3)>,
        pivot: Option<DVec3>,
        limits: Option<[f64; 2]>,
        mates: Vec<String>,
        joint_mates: Vec<(&'a OccurrencePath, MateId)>,
    }
    let mut edges: BTreeMap<(usize, usize), Edge> = BTreeMap::new();
    for ((a, b), mates) in &pairs {
        let mut edge = Edge {
            kind: JointKind::Fixed,
            axis: None,
            pivot: None,
            limits: None,
            mates: mates.iter().map(|(_, m)| m.name().to_owned()).collect(),
            joint_mates: mates.iter().map(|&(prefix, m)| (prefix, m.id())).collect(),
        };
        let hinge = mates.iter().find_map(|&(prefix, m)| match m {
            Mate::Hinge { axes, angle_limits, .. } => Some((prefix, axes, *angle_limits)),
            _ => None,
        });
        let concentric = mates.iter().find_map(|&(prefix, m)| match m {
            Mate::Concentric { a, lock_rotation: None, alignment: Some(_), .. } => Some((prefix, a)),
            _ => None,
        });
        if let Some((prefix, axes, limits)) = hinge {
            edge.axis = axes.iter().find_map(|a| axis_of(a, placement(prefix, a)));
            edge.kind = if limits.is_some() { JointKind::Revolute } else { JointKind::Continuous };
            edge.limits = limits;
        } else if let Some((prefix, end)) = concentric {
            edge.axis = axis_of(end, placement(prefix, end));
            edge.kind = JointKind::Continuous;
            let planar = mates
                .iter()
                .any(|(_, m)| matches!(m, Mate::Coincident { .. } | Mate::Distance { .. } | Mate::Tangent { .. }));
            if !planar {
                notes.push(format!(
                    "{} leaves a slide along the axis that URDF has no joint for; it is carried as a turn only",
                    edge.mates.join(", ")
                ));
            }
        }
        if edge.kind == JointKind::Fixed
            && let [(prefix, mate)] = mates.as_slice()
        {
            let world = |end: &FaceEndpoint| mat(&placement(prefix, end));
            match mate {
                Mate::Concentric { a, lock_rotation: Some(_), alignment: Some(_), .. } => {
                    edge.axis = axis_of(a, placement(prefix, a));
                    edge.kind = JointKind::Prismatic;
                }
                Mate::Coincident { a, b, .. } | Mate::Distance { a, b, .. }
                    if let (Some(plane), Some(_)) = (plane_of(a, world(a)), plane_of(b, world(b))) =>
                {
                    edge.axis = Some(plane);
                    edge.kind = JointKind::Planar;
                }
                Mate::Coincident { a, b, .. } | Mate::Concentric { a, b, lock_rotation: None, .. }
                    if let (Some(point), Some(_)) = (point_of(a, world(a)), point_of(b, world(b))) =>
                {
                    edge.pivot = Some(point);
                    edge.kind = JointKind::Spherical;
                }
                _ => {}
            }
        }
        if !matches!(edge.kind, JointKind::Fixed | JointKind::Spherical) && edge.axis.is_none() {
            notes.push(format!("{}: the axis could not be read, so the joint is fixed", edge.mates.join(", ")));
            edge.kind = JointKind::Fixed;
        }
        if edge.kind == JointKind::Fixed {
            let others: Vec<&str> =
                mates.iter().filter(|(_, m)| !matches!(m, Mate::Lock { .. })).map(|(_, m)| m.name()).collect();
            if !others.is_empty() {
                notes.push(format!("{} carried as a fixed joint at the assembly's pose", others.join(", ")));
            }
        }
        edges.insert((*a, *b), edge);
    }

    // Keep a spanning tree for each connected group, preferring an Origin mate as its root.
    let mut adjacency = vec![Vec::new(); worlds.len()];
    for (&(a, b), edge) in &edges {
        adjacency[a].push((b, edge));
        adjacency[b].push((a, edge));
    }
    let mut joints: Vec<ExportJoint> = Vec::new();
    let link_of = |body: usize| body + 1;
    let mut joint_of_mate = HashMap::new();
    let mut reached = vec![false; worlds.len()];
    let mut link_world = vec![DMat4::IDENTITY; links.len()];
    let mut used = HashSet::new();
    for base in origins.into_iter().chain(0..worlds.len()) {
        if reached[base] {
            continue;
        }
        reached[base] = true;
        let child = link_of(base);
        link_world[child] = mat(&worlds[base]);
        if !joints.is_empty() && !grounded[base] {
            notes.push(format!("{} is held by no mate to the rest and is fixed where it stands", links[child].name));
        }
        joints.push(ExportJoint {
            name: format!("{}_fixed", links[child].name),
            kind: JointKind::Fixed,
            parent: 0,
            child,
            frame: link_world[child],
            axis: [1.0, 0.0, 0.0],
            limits: None,
            mates: Vec::new(),
            mimic: None,
        });
        let mut queue = VecDeque::from([base]);
        while let Some(at) = queue.pop_front() {
            for &(next, edge) in &adjacency[at] {
                if !used.insert((at.min(next), at.max(next))) {
                    continue;
                }
                if reached[next] {
                    notes.push(format!("{} would close a loop and is left out", edge.mates.join(", ")));
                    continue;
                }
                reached[next] = true;
                let child_world = worlds[next];
                let joint_world = match edge.axis {
                    Some((point, direction)) => axis_frame(point, direction, child_world.rotation),
                    None => edge.pivot.map_or_else(
                        || mat(&child_world),
                        |point| DMat4::from_rotation_translation(child_world.rotation, point),
                    ),
                };
                let parent_link = link_of(at);
                let child_link = link_of(next);
                link_world[child_link] = joint_world;
                for m in &edge.joint_mates {
                    joint_of_mate.insert(*m, joints.len());
                }
                joints.push(ExportJoint {
                    name: format!("{}_{}", links[child_link].name, joint_word(edge.kind)),
                    kind: edge.kind,
                    parent: parent_link,
                    child: child_link,
                    frame: link_world[parent_link].inverse() * joint_world,
                    axis: edge.axis.map(|(_, d)| d.to_array()).unwrap_or([1.0, 0.0, 0.0]),
                    limits: edge.limits,
                    mates: edge.mates.clone(),
                    mimic: None,
                });
                queue.push_back(next);
            }
        }
    }
    for (i, link) in links.iter_mut().enumerate().skip(1) {
        link.visual = link_world[i].inverse() * mat(&worlds[i - 1]);
    }

    // A gear's mate IDs are local to the assembly occurrence that holds it.
    for (prefix, mate) in scoped_mates {
        let Mate::Mechanical { name, mechanism, joints: coupled, factor, reverse, .. } = mate else { continue };
        let found: Vec<usize> = coupled.iter().filter_map(|m| joint_of_mate.get(&(prefix, *m)).copied()).collect();
        match (mechanism, &found[..]) {
            (Mechanism::Gear, [lead, follow]) if lead != follow && factor[0] != 0.0 => {
                let sign = if *reverse { -1.0 } else { 1.0 };
                let leader = joints[*lead].name.clone();
                joints[*follow].mimic = Some((leader, -sign * factor[1] / factor[0]));
                joints[*follow].mates.push(name.clone());
            }
            _ => notes.push(format!("{name} has no URDF joint coupling and is left out")),
        }
    }
    Ok(Articulation { name: doc.assembly().name.clone(), links, joints, meshes, notes })
}

fn joint_word(kind: JointKind) -> &'static str {
    match kind {
        JointKind::Fixed => "fixed",
        _ => "joint",
    }
}

fn export_mesh(name: &str, m: &DisplayMesh) -> ExportMesh {
    ExportMesh {
        name: safe(name),
        positions: m.positions.iter().map(|p| DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64)).collect(),
        triangles: m.indices.as_chunks::<3>().0.to_vec(),
    }
}

fn rpy(m: &DMat4) -> [f64; 3] {
    let (_, q, _) = m.to_scale_rotation_translation();
    // URDF's rpy: fixed-axis roll about X, pitch about Y, yaw about Z, i.e. R = Rz Ry Rx.
    let (yaw, pitch, roll) = q.to_euler(glam::EulerRot::ZYX);
    [roll, pitch, yaw]
}

fn origin(m: &DMat4, scale: f64) -> String {
    let t = m.w_axis.truncate() * scale;
    let r = rpy(m);
    format!(
        "<origin xyz=\"{} {} {}\" rpy=\"{} {} {}\"/>",
        num(t.x),
        num(t.y),
        num(t.z),
        num(r[0]),
        num(r[1]),
        num(r[2])
    )
}

fn num(v: f64) -> String {
    let v = if v.abs() < 1e-15 { 0.0 } else { v };
    format!("{v}")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The articulation as a URDF at `path` and its meshes as binary STL in millimetres in a
/// `<name>_meshes` folder beside it, scaled to metres in the URDF: every file's path and
/// bytes, the URDF last.
pub fn urdf_files(a: &Articulation, path: &Path) -> Files {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("robot");
    let mesh_dir_name = format!("{stem}_meshes");
    let mesh_dir = dir.join(&mesh_dir_name);
    let mut files = Vec::new();
    for m in &a.meshes {
        files.push((mesh_dir.join(format!("{}.stl", m.name)), stl(m)));
    }
    let mut x = String::new();
    let _ = writeln!(x, "<?xml version=\"1.0\"?>");
    let _ = writeln!(x, "<!-- Written by SolidDraft from the assembly {}. Lengths in metres. -->", escape(&a.name));
    let _ = writeln!(x, "<robot name=\"{}\">", escape(&safe(&a.name)));
    for l in &a.links {
        let _ = writeln!(x, "  <link name=\"{}\">", escape(&l.name));
        if let Some(m) = l.mesh {
            let file = format!("{mesh_dir_name}/{}.stl", a.meshes[m].name);
            for tag in ["visual", "collision"] {
                let _ = writeln!(x, "    <{tag}>");
                let _ = writeln!(x, "      {}", origin(&l.visual, 0.001));
                let _ = writeln!(
                    x,
                    "      <geometry><mesh filename=\"{}\" scale=\"0.001 0.001 0.001\"/></geometry>",
                    escape(&file)
                );
                if tag == "visual"
                    && let Some(c) = l.colour
                {
                    let _ = writeln!(
                        x,
                        "      <material name=\"{}_colour\"><color rgba=\"{} {} {} 1\"/></material>",
                        escape(&l.name),
                        num(c[0]),
                        num(c[1]),
                        num(c[2])
                    );
                }
                let _ = writeln!(x, "    </{tag}>");
            }
        }
        let _ = writeln!(x, "  </link>");
    }
    for j in &a.joints {
        let kind = match j.kind {
            JointKind::Revolute => "revolute",
            JointKind::Continuous => "continuous",
            JointKind::Prismatic => "prismatic",
            JointKind::Planar => "planar",
            JointKind::Spherical => "spherical",
            _ => "fixed",
        };
        let _ = writeln!(x, "  <joint name=\"{}\" type=\"{kind}\">", escape(&j.name));
        if !j.mates.is_empty() {
            let _ = writeln!(x, "    <!-- from the mates {} -->", escape(&j.mates.join(", ")).replace("--", "- -"));
        }
        let _ = writeln!(x, "    <parent link=\"{}\"/>", escape(&a.links[j.parent].name));
        let _ = writeln!(x, "    <child link=\"{}\"/>", escape(&a.links[j.child].name));
        let _ = writeln!(x, "    {}", origin(&j.frame, 0.001));
        if matches!(j.kind, JointKind::Revolute | JointKind::Continuous | JointKind::Prismatic | JointKind::Planar) {
            let _ = writeln!(x, "    <axis xyz=\"1 0 0\"/>");
        }
        let limits = match j.kind {
            JointKind::Revolute => j.limits.map(|limits| limits.map(f64::to_radians)),
            JointKind::Prismatic => {
                Some(j.limits.map_or([f64::NEG_INFINITY, f64::INFINITY], |limits| limits.map(|mm| mm * 0.001)))
            }
            _ => None,
        };
        if let Some([lo, hi]) = limits {
            let _ = writeln!(x, "    <limit lower=\"{}\" upper=\"{}\" effort=\"0\" velocity=\"0\"/>", num(lo), num(hi));
        }
        if let Some((leader, k)) = &j.mimic {
            let _ = writeln!(x, "    <mimic joint=\"{}\" multiplier=\"{}\" offset=\"0\"/>", escape(leader), num(*k));
        }
        let _ = writeln!(x, "  </joint>");
    }
    let _ = writeln!(x, "</robot>");
    files.push((path.to_path_buf(), x.into_bytes()));
    files
}

fn stl(m: &ExportMesh) -> Vec<u8> {
    let mut out = vec![0u8; 80];
    out[..21].copy_from_slice(b"SolidDraft millimetre");
    out.extend((m.triangles.len() as u32).to_le_bytes());
    for t in &m.triangles {
        let [a, b, c] = t.map(|i| m.positions[i as usize]);
        let n = (b - a).cross(c - a).normalize_or_zero();
        for v in [n, a, b, c] {
            for k in v.to_array() {
                out.extend((k as f32).to_le_bytes());
            }
        }
        out.extend(0u16.to_le_bytes());
    }
    out
}

fn usd_matrix(m: &DMat4) -> String {
    // USD writes row vectors: each row is a column of ours.
    let c = m.to_cols_array_2d();
    let row = |r: [f64; 4]| format!("({}, {}, {}, {})", num(r[0]), num(r[1]), num(r[2]), num(r[3]));
    format!("( {}, {}, {}, {} )", row(c[0]), row(c[1]), row(c[2]), row(c[3]))
}

/// The articulation as an OpenUSD text layer: one Xform per link, each with its mesh,
/// nested as the joints nest them, and a UsdPhysics joint per joint.
pub fn usda_text(a: &Articulation) -> String {
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); a.links.len()];
    let mut parent_joint: Vec<Option<usize>> = vec![None; a.links.len()];
    for (i, j) in a.joints.iter().enumerate() {
        children[j.parent].push(j.child);
        parent_joint[j.child] = Some(i);
    }
    let root = safe(&a.name);
    let mut x = String::new();
    let _ = writeln!(x, "#usda 1.0");
    let _ = writeln!(
        x,
        "(\n    defaultPrim = \"{root}\"\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    doc = \"Written by SolidDraft from the assembly {}\"\n)\n",
        escape(&a.name)
    );
    let mut prim_path: Vec<String> = vec![String::new(); a.links.len()];
    let _ = writeln!(x, "def Xform \"{root}\" (\n    prepend apiSchemas = [\"PhysicsArticulationRootAPI\"]\n)\n{{");
    #[allow(clippy::too_many_arguments, reason = "the recursion carries the layer, the tree and the paths it names")]
    fn link(
        x: &mut String,
        a: &Articulation,
        i: usize,
        at: &str,
        depth: usize,
        children: &[Vec<usize>],
        parent_joint: &[Option<usize>],
        prim_path: &mut [String],
    ) {
        let pad = "    ".repeat(depth);
        let l = &a.links[i];
        let here = format!("{at}/{}", l.name);
        prim_path[i] = here.clone();
        let local = parent_joint[i].map(|j| a.joints[j].frame).unwrap_or(DMat4::IDENTITY);
        let _ = writeln!(
            x,
            "{pad}def Xform \"{}\" (\n{pad}    prepend apiSchemas = [\"PhysicsRigidBodyAPI\"]\n{pad})\n{pad}{{",
            l.name
        );
        let _ = writeln!(x, "{pad}    matrix4d xformOp:transform = {}", usd_matrix(&local));
        let _ = writeln!(x, "{pad}    uniform token[] xformOpOrder = [\"xformOp:transform\"]");
        if let Some(m) = l.mesh {
            let mesh = &a.meshes[m];
            let _ = writeln!(
                x,
                "{pad}    def Mesh \"visual\" (\n{pad}        prepend apiSchemas = [\"PhysicsCollisionAPI\"]\n{pad}    )\n{pad}    {{"
            );
            let _ = writeln!(x, "{pad}        matrix4d xformOp:transform = {}", usd_matrix(&l.visual));
            let _ = writeln!(x, "{pad}        uniform token[] xformOpOrder = [\"xformOp:transform\"]");
            let counts = vec!["3"; mesh.triangles.len()].join(", ");
            let idx: Vec<String> = mesh.triangles.iter().flatten().map(u32::to_string).collect();
            let pts: Vec<String> =
                mesh.positions.iter().map(|p| format!("({}, {}, {})", num(p.x), num(p.y), num(p.z))).collect();
            let _ = writeln!(x, "{pad}        int[] faceVertexCounts = [{counts}]");
            let _ = writeln!(x, "{pad}        int[] faceVertexIndices = [{}]", idx.join(", "));
            let _ = writeln!(x, "{pad}        point3f[] points = [{}]", pts.join(", "));
            let _ = writeln!(x, "{pad}        uniform token subdivisionScheme = \"none\"");
            if let Some(c) = l.colour {
                let _ = writeln!(
                    x,
                    "{pad}        color3f[] primvars:displayColor = [({}, {}, {})]",
                    num(c[0]),
                    num(c[1]),
                    num(c[2])
                );
            }
            let _ = writeln!(x, "{pad}    }}");
        }
        for &c in &children[i] {
            link(x, a, c, &here, depth + 1, children, parent_joint, prim_path);
        }
        let _ = writeln!(x, "{pad}}}");
    }
    link(&mut x, a, 0, &format!("/{root}"), 1, &children, &parent_joint, &mut prim_path);
    let _ = writeln!(x, "    def Scope \"joints\"\n    {{");
    for j in &a.joints {
        let kind = match j.kind {
            JointKind::Revolute | JointKind::Continuous => "PhysicsRevoluteJoint",
            JointKind::Prismatic => "PhysicsPrismaticJoint",
            JointKind::Planar => "PhysicsJoint",
            JointKind::Spherical => "PhysicsSphericalJoint",
            _ => "PhysicsFixedJoint",
        };
        let (_, rot, pos) = j.frame.to_scale_rotation_translation();
        let _ = writeln!(x, "        def {kind} \"{}\"", j.name);
        if j.kind == JointKind::Planar {
            let _ = writeln!(
                x,
                "        (prepend apiSchemas = [\"PhysicsLimitAPI:transX\", \"PhysicsLimitAPI:rotY\", \"PhysicsLimitAPI:rotZ\"])"
            );
        }
        let _ = writeln!(x, "        {{");
        let _ = writeln!(x, "            rel physics:body0 = <{}>", prim_path[j.parent]);
        let _ = writeln!(x, "            rel physics:body1 = <{}>", prim_path[j.child]);
        let _ = writeln!(x, "            point3f physics:localPos0 = ({}, {}, {})", num(pos.x), num(pos.y), num(pos.z));
        let _ = writeln!(
            x,
            "            quatf physics:localRot0 = ({}, {}, {}, {})",
            num(rot.w),
            num(rot.x),
            num(rot.y),
            num(rot.z)
        );
        let _ = writeln!(x, "            point3f physics:localPos1 = (0, 0, 0)");
        let _ = writeln!(x, "            quatf physics:localRot1 = (1, 0, 0, 0)");
        if matches!(j.kind, JointKind::Revolute | JointKind::Continuous | JointKind::Prismatic) {
            let _ = writeln!(x, "            uniform token physics:axis = \"X\"");
            if let Some([lo, hi]) = j.limits {
                let _ = writeln!(x, "            float physics:lowerLimit = {}", num(lo));
                let _ = writeln!(x, "            float physics:upperLimit = {}", num(hi));
            }
        }
        if j.kind == JointKind::Planar {
            // The normal is X. A low limit above the high limit locks the DOF in USD.
            for axis in ["transX", "rotY", "rotZ"] {
                let _ = writeln!(x, "            float limit:{axis}:physics:low = 1");
                let _ = writeln!(x, "            float limit:{axis}:physics:high = -1");
            }
        }
        if let Some((leader, k)) = &j.mimic {
            let _ = writeln!(x, "            rel newton:mimicJoint = </{root}/joints/{leader}>");
            let _ = writeln!(x, "            float newton:mimicCoef0 = 0");
            let _ = writeln!(x, "            float newton:mimicCoef1 = {}", num(*k));
        }
        let _ = writeln!(x, "        }}");
    }
    let _ = writeln!(x, "    }}");
    let _ = writeln!(x, "}}");
    x
}

/// Files to write: each path with its bytes.
pub type Files = Vec<(PathBuf, Vec<u8>)>;

/// The open assembly's export to `path`, `.urdf` or `.usda` for OpenUSD, without writing
/// anything: the articulation and every file's path and bytes, the named file last.
pub fn export_files(doc: &mut AssemblyDocument, path: &Path) -> Result<(Articulation, Files), String> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if !matches!(ext.as_str(), "urdf" | "usda") {
        return Err("export an assembly to a .urdf or a .usda file".into());
    }
    let a = articulate(doc)?;
    let files = match ext.as_str() {
        "urdf" => urdf_files(&a, path),
        _ => vec![(path.to_path_buf(), usda_text(&a).into_bytes())],
    };
    Ok((a, files))
}

/// Exports the open assembly to `path`, `.urdf` or `.usda`, writing every file. Returns
/// the articulation and the files, the named one last.
pub fn export(doc: &mut AssemblyDocument, path: &Path) -> Result<(Articulation, Vec<PathBuf>), String> {
    let (a, files) = export_files(doc, path)?;
    let mut written = Vec::new();
    for (p, bytes) in files {
        if let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&p, bytes).map_err(|e| format!("{}: {e}", p.display()))?;
        written.push(p);
    }
    Ok((a, written))
}
