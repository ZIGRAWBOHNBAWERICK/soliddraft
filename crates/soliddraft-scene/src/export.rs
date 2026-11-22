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
//! | Anything else | fixed, at the pose the assembly holds |
//!
//! A Gear mate between two of those joints makes the second follow the first (URDF
//! `mimic`). The joints form a tree from `base_link`: a mate that would close a loop is
//! left out, and a part no mate reaches is fixed to `base_link` where it stands. Every
//! such choice is in [`Articulation::notes`], naming the mates. Lengths are written in
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
    AssemblyDocument, DisplayMesh, FaceEndpoint, Flags, Mate, OccurrenceIndex, OccurrencePath, StoredSupport,
};

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
    /// How it moves: revolute, continuous or fixed.
    pub kind: JointKind,
    /// The parent link.
    pub parent: usize,
    /// The child link.
    pub child: usize,
    /// The joint's frame in the parent link's, millimetres; the axis is its X.
    #[serde(skip)]
    pub frame: DMat4,
    /// The axis in the assembly's frame, for the record.
    pub axis: [f64; 3],
    /// Lower and upper limits in degrees, for a revolute joint.
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

/// A link's world frame and what the export knows about it while it builds.
struct Body {
    world: DMat4,
}

fn mat(p: &soliddraft_assembly::Placement) -> DMat4 {
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

/// An endpoint's axis, placed: a point on it and its unit direction in the assembly's frame.
fn axis_of(end: &FaceEndpoint, bodies: &HashMap<OccurrencePath, usize>, worlds: &[Body]) -> Option<(DVec3, DVec3)> {
    let body = &worlds[*bodies.get(&end.path)?];
    let (through, direction) = match end.support.as_ref()? {
        StoredSupport::Axis { direction, through, .. } => {
            (DVec3::from_array(*through), DVec3::new(direction[0] as f64, direction[1] as f64, direction[2] as f64))
        }
        StoredSupport::Enclosed { support } => match support.as_ref() {
            soliddraft_kernel_api::EnclosedSupport::Axis { direction, through, .. } => {
                let f = |r: &soliddraft_kernel_api::ExactRational| {
                    r.numerator.parse::<f64>().unwrap_or(f64::NAN) / r.denominator.parse::<f64>().unwrap_or(f64::NAN)
                };
                let mid = |b: &soliddraft_kernel_api::SupportBounds| (f(&b.lo) + f(&b.hi)) * 0.5;
                (DVec3::new(mid(&through[0]), mid(&through[1]), mid(&through[2])), {
                    DVec3::new(mid(&direction[0]), mid(&direction[1]), mid(&direction[2]))
                })
            }
            _ => return None,
        },
        _ => return None,
    };
    let direction = direction.try_normalize()?;
    Some((body.world.transform_point3(through), body.world.transform_vector3(direction).normalize()))
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
    let truth = doc.assembly().clone();
    let table = doc.table();
    let mut notes = Vec::new();

    // Links: every part or mesh occurrence that is not suppressed.
    let mut worlds: Vec<Body> = Vec::new();
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
        worlds.push(Body { world: mat(&table.world(row)) });
        links.push(Link { name, path: Some(path.to_string()), mesh: Some(m), visual: DMat4::IDENTITY, colour });
    }
    if worlds.is_empty() {
        return Err("the assembly has no parts to export".into());
    }

    // What each pair of parts is held by.
    let mut pairs: BTreeMap<(usize, usize), Vec<&Mate>> = BTreeMap::new();
    let mut grounded: Option<usize> = None;
    for mate in truth.known_mates() {
        if let Mate::Origin { component, .. } = mate {
            let p = OccurrencePath::root().child(soliddraft_assembly::Step::component(*component));
            grounded = grounded.or(bodies.get(&p).copied());
            continue;
        }
        if matches!(mate, Mate::Mechanical { .. }) {
            continue;
        }
        let mut ends: Vec<usize> = mate.ends().iter().filter_map(|e| bodies.get(e.path).copied()).collect();
        ends.sort();
        ends.dedup();
        match ends[..] {
            [a, b] => pairs.entry((a, b)).or_default().push(mate),
            [_] => {}
            _ => notes.push(format!("{} ties more than two parts and is carried as fixed", mate.name())),
        }
    }
    let base = grounded.unwrap_or(0);

    // A joint per pair, as the mates say.
    struct Edge {
        kind: JointKind,
        axis: Option<(DVec3, DVec3)>,
        limits: Option<[f64; 2]>,
        mates: Vec<String>,
        joint_mates: Vec<soliddraft_assembly::MateId>,
    }
    let mut edges: BTreeMap<(usize, usize), Edge> = BTreeMap::new();
    for ((a, b), mates) in &pairs {
        let mut edge = Edge {
            kind: JointKind::Fixed,
            axis: None,
            limits: None,
            mates: mates.iter().map(|m| m.name().to_owned()).collect(),
            joint_mates: mates.iter().map(|m| m.id()).collect(),
        };
        let hinge = mates.iter().find_map(|m| match m {
            Mate::Hinge { axes, angle_limits, .. } => Some((axes, *angle_limits)),
            _ => None,
        });
        let concentric = mates.iter().find_map(|m| match m {
            Mate::Concentric { a, lock_rotation: None, alignment: Some(_), .. } => Some(a),
            _ => None,
        });
        if let Some((axes, limits)) = hinge {
            edge.axis = axis_of(&axes[0], &bodies, &worlds).or_else(|| axis_of(&axes[1], &bodies, &worlds));
            edge.kind = if limits.is_some() { JointKind::Revolute } else { JointKind::Continuous };
            edge.limits = limits;
        } else if let Some(end) = concentric {
            edge.axis = axis_of(end, &bodies, &worlds);
            edge.kind = JointKind::Continuous;
            let planar = mates
                .iter()
                .any(|m| matches!(m, Mate::Coincident { .. } | Mate::Distance { .. } | Mate::Tangent { .. }));
            if !planar {
                notes.push(format!(
                    "{} leaves a slide along the axis that URDF has no joint for; it is carried as a turn only",
                    edge.mates.join(", ")
                ));
            }
        }
        if edge.kind != JointKind::Fixed && edge.axis.is_none() {
            notes.push(format!("{}: the axis could not be read, so the joint is fixed", edge.mates.join(", ")));
            edge.kind = JointKind::Fixed;
        }
        if edge.kind == JointKind::Fixed {
            let others: Vec<&str> =
                mates.iter().filter(|m| !matches!(m, Mate::Lock { .. })).map(|m| m.name()).collect();
            if !others.is_empty() {
                notes.push(format!("{} carried as a fixed joint at the assembly's pose", others.join(", ")));
            }
        }
        edges.insert((*a, *b), edge);
    }

    // The tree, from the grounded part out.
    let mut joints: Vec<ExportJoint> = Vec::new();
    let link_of = |body: usize| body + 1;
    let mut joint_of_mate: HashMap<soliddraft_assembly::MateId, usize> = HashMap::new();
    let mut reached = vec![false; worlds.len()];
    reached[base] = true;
    let base_world = worlds[base].world;
    joints.push(ExportJoint {
        name: format!("{}_fixed", links[link_of(base)].name),
        kind: JointKind::Fixed,
        parent: 0,
        child: link_of(base),
        frame: base_world,
        axis: [1.0, 0.0, 0.0],
        limits: None,
        mates: Vec::new(),
        mimic: None,
    });
    // Each link's frame in the world, for frames relative to the parent.
    let mut link_world: Vec<DMat4> = vec![DMat4::IDENTITY; links.len()];
    link_world[link_of(base)] = base_world;
    let mut queue = VecDeque::from([base]);
    let mut used: HashSet<(usize, usize)> = HashSet::new();
    while let Some(at) = queue.pop_front() {
        for (&(a, b), edge) in &edges {
            let next = if a == at {
                b
            } else if b == at {
                a
            } else {
                continue;
            };
            if used.contains(&(a, b)) {
                continue;
            }
            used.insert((a, b));
            if reached[next] {
                notes.push(format!("{} would close a loop and is left out", edge.mates.join(", ")));
                continue;
            }
            reached[next] = true;
            let child_world = worlds[next].world;
            let (rotation, _) = {
                let (_, r, t) = child_world.to_scale_rotation_translation();
                (r, t)
            };
            let joint_world = match edge.axis {
                Some((point, direction)) => axis_frame(point, direction, rotation),
                None => child_world,
            };
            let parent_link = link_of(at);
            let child_link = link_of(next);
            link_world[child_link] = joint_world;
            let id = joints.len();
            for m in &edge.joint_mates {
                joint_of_mate.insert(*m, id);
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
    for (i, r) in reached.iter().enumerate() {
        if !r {
            let child = link_of(i);
            link_world[child] = worlds[i].world;
            notes.push(format!("{} is held by no mate to the rest and is fixed where it stands", links[child].name));
            joints.push(ExportJoint {
                name: format!("{}_fixed", links[child].name),
                kind: JointKind::Fixed,
                parent: 0,
                child,
                frame: worlds[i].world,
                axis: [1.0, 0.0, 0.0],
                limits: None,
                mates: Vec::new(),
                mimic: None,
            });
        }
    }
    for (i, l) in links.iter_mut().enumerate().skip(1) {
        l.visual = link_world[i].inverse() * worlds[i - 1].world;
    }

    // Gears: the second joint follows the first.
    for mate in truth.known_mates() {
        let Mate::Mechanical { name, mechanism, joints: coupled, factor, reverse, .. } = mate else { continue };
        let found: Vec<usize> = coupled.iter().filter_map(|m| joint_of_mate.get(m).copied()).collect();
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
    Ok(Articulation { name: truth.name.clone(), links, joints, meshes, notes })
}

fn joint_word(kind: JointKind) -> &'static str {
    match kind {
        JointKind::Revolute | JointKind::Continuous => "joint",
        _ => "fixed",
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
            _ => "fixed",
        };
        let _ = writeln!(x, "  <joint name=\"{}\" type=\"{kind}\">", escape(&j.name));
        if !j.mates.is_empty() {
            let _ = writeln!(x, "    <!-- from the mates {} -->", escape(&j.mates.join(", ")).replace("--", "- -"));
        }
        let _ = writeln!(x, "    <parent link=\"{}\"/>", escape(&a.links[j.parent].name));
        let _ = writeln!(x, "    <child link=\"{}\"/>", escape(&a.links[j.child].name));
        let _ = writeln!(x, "    {}", origin(&j.frame, 0.001));
        if j.kind != JointKind::Fixed {
            let _ = writeln!(x, "    <axis xyz=\"1 0 0\"/>");
        }
        if let (JointKind::Revolute, Some([lo, hi])) = (j.kind, j.limits) {
            let _ = writeln!(
                x,
                "    <limit lower=\"{}\" upper=\"{}\" effort=\"0\" velocity=\"0\"/>",
                num(lo.to_radians()),
                num(hi.to_radians())
            );
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
            _ => "PhysicsFixedJoint",
        };
        let (_, rot, pos) = j.frame.to_scale_rotation_translation();
        let _ = writeln!(x, "        def {kind} \"{}\"\n        {{", j.name);
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
        if kind == "PhysicsRevoluteJoint" {
            let _ = writeln!(x, "            uniform token physics:axis = \"X\"");
            if let (JointKind::Revolute, Some([lo, hi])) = (j.kind, j.limits) {
                let _ = writeln!(x, "            float physics:lowerLimit = {}", num(lo));
                let _ = writeln!(x, "            float physics:upperLimit = {}", num(hi));
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
