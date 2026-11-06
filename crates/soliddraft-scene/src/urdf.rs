//! URDF: links nested by their joints, each link's visuals inside it.
//!
//! The root link is the scene's root: its visuals sit directly under the robot. Every
//! other link is a node at its joint's origin in its parent link, and every visual a node
//! at its own origin in its link. Meshes are OBJ or STL files beside the URDF, in metres
//! like the URDF, scaled by the visual's `scale`. Collision geometry and inertia are not
//! read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use glam::{DMat4, DQuat, DVec3, EulerRot};

use crate::{Body, Format, ImportError, Joint, JointKind, Material, Mimic, Scene, UpAxis, obj, shapes, stl};

/// The frame of a URDF origin: `xyz` then roll, pitch, yaw about the fixed axes.
fn pose(p: &urdf_rs::Pose) -> DMat4 {
    let [x, y, z] = p.xyz.0;
    let [roll, pitch, yaw] = p.rpy.0;
    DMat4::from_rotation_translation(DQuat::from_euler(EulerRot::ZYX, yaw, pitch, roll), DVec3::new(x, y, z))
}

pub(crate) fn read(path: &Path) -> Result<Scene, ImportError> {
    let robot = urdf_rs::read_file(path)
        .map_err(|e| ImportError(format!("{} is not a valid URDF file: {e}", path.display())))?;
    let name = if robot.name.is_empty() { crate::stem(path) } else { robot.name.clone() };
    let mut scene = Scene::new(name, Format::Urdf, 1.0, UpAxis::Z);
    let base = path.parent().unwrap_or(Path::new("."));
    let named: HashMap<&str, &urdf_rs::Material> = robot.materials.iter().map(|m| (m.name.as_str(), m)).collect();
    let children: HashMap<&str, Vec<&urdf_rs::Joint>> = robot.joints.iter().fold(HashMap::new(), |mut map, j| {
        map.entry(j.parent.link.as_str()).or_default().push(j);
        map
    });
    let is_child: std::collections::HashSet<&str> = robot.joints.iter().map(|j| j.child.link.as_str()).collect();
    let roots: Vec<&urdf_rs::Link> = robot.links.iter().filter(|l| !is_child.contains(l.name.as_str())).collect();
    let links: HashMap<&str, &urdf_rs::Link> = robot.links.iter().map(|l| (l.name.as_str(), l)).collect();
    let mut cache: HashMap<PathBuf, Vec<(usize, Option<usize>)>> = HashMap::new();
    let mut node_of: HashMap<String, usize> = HashMap::new();
    // One root link merges into the robot; several sit side by side under it.
    let mut stack: Vec<(&urdf_rs::Link, usize)> = Vec::new();
    match roots.as_slice() {
        [one] => {
            scene.nodes[0].source = format!("link {}", one.name);
            node_of.insert(one.name.clone(), 0);
            stack.push((one, 0));
        }
        many => {
            for l in many {
                let n = scene.add(0, l.name.clone(), format!("link {}", l.name), DMat4::IDENTITY);
                node_of.insert(l.name.clone(), n);
                stack.push((l, n));
            }
        }
    }
    while let Some((link, node)) = stack.pop() {
        for (i, visual) in link.visual.iter().enumerate() {
            let label = visual.name.clone().unwrap_or_else(|| {
                if link.visual.len() == 1 { link.name.clone() } else { format!("{} visual {}", link.name, i + 1) }
            });
            let material = visual.material.as_ref().and_then(|m| material(&mut scene, m, &named));
            let v =
                scene.add(node, label.clone(), format!("link {} visual {}", link.name, i + 1), pose(&visual.origin));
            match &visual.geometry {
                urdf_rs::Geometry::Mesh { filename, scale } => {
                    let file = resolve(base, filename).ok_or_else(|| {
                        ImportError(format!("the mesh {filename} of link {} cannot be found", link.name))
                    })?;
                    let loaded = match cache.get(&file) {
                        Some(l) => l.clone(),
                        None => {
                            let l = load(&mut scene, &file)?;
                            cache.insert(file.clone(), l.clone());
                            l
                        }
                    };
                    if let Some(s) = scale {
                        scene.nodes[v].local *= DMat4::from_scale(DVec3::from_array(s.0));
                    }
                    match loaded.as_slice() {
                        [(mesh, own)] => scene.nodes[v].body = Some(Body { mesh: *mesh, material: material.or(*own) }),
                        many => {
                            for (k, (mesh, own)) in many.iter().enumerate() {
                                let part = scene.meshes[*mesh].name.clone();
                                let name = if part.is_empty() { format!("{label} {}", k + 1) } else { part };
                                let c = scene.add(v, name, format!("{} object {}", filename, k + 1), DMat4::IDENTITY);
                                scene.nodes[c].body = Some(Body { mesh: *mesh, material: material.or(*own) });
                            }
                        }
                    }
                }
                urdf_rs::Geometry::Box { size } => {
                    let mesh = scene.add_mesh(shapes::cuboid(&label, DVec3::from_array(size.0)));
                    scene.nodes[v].body = Some(Body { mesh, material });
                }
                urdf_rs::Geometry::Cylinder { radius, length } => {
                    let mesh = scene.add_mesh(shapes::cylinder(&label, *radius, *length));
                    scene.nodes[v].body = Some(Body { mesh, material });
                }
                urdf_rs::Geometry::Sphere { radius } => {
                    let mesh = scene.add_mesh(shapes::sphere(&label, *radius));
                    scene.nodes[v].body = Some(Body { mesh, material });
                }
                urdf_rs::Geometry::Capsule { radius, length } => {
                    scene.note("capsules are shown as cylinders of the same length and radius");
                    let mesh = scene.add_mesh(shapes::cylinder(&label, *radius, length + 2.0 * radius));
                    scene.nodes[v].body = Some(Body { mesh, material });
                }
            }
        }
        if !link.collision.is_empty() {
            scene.note("collision geometry is not imported");
        }
        // Child links are numbered in file order and pushed the other way, so they pop in
        // file order too.
        let mut next = Vec::new();
        for joint in children.get(link.name.as_str()).into_iter().flatten() {
            let Some(child) = links.get(joint.child.link.as_str()) else {
                return Err(ImportError(format!(
                    "joint {} moves link {}, which is not in the file",
                    joint.name, joint.child.link
                )));
            };
            if node_of.contains_key(&child.name) {
                return Err(ImportError(format!("link {} has two parents; a URDF is a tree", child.name)));
            }
            let c = scene.add(node, child.name.clone(), format!("link {}", child.name), pose(&joint.origin));
            node_of.insert(child.name.clone(), c);
            next.push((*child, c));
        }
        stack.extend(next.into_iter().rev());
    }
    if node_of.len() < robot.links.len() {
        scene.note("links that no joint reaches from the root were left out");
    }
    for joint in &robot.joints {
        let (Some(&parent), Some(&child)) = (node_of.get(&joint.parent.link), node_of.get(&joint.child.link)) else {
            continue;
        };
        let kind = match joint.joint_type {
            urdf_rs::JointType::Revolute => JointKind::Revolute,
            urdf_rs::JointType::Continuous => JointKind::Continuous,
            urdf_rs::JointType::Prismatic => JointKind::Prismatic,
            urdf_rs::JointType::Fixed => JointKind::Fixed,
            urdf_rs::JointType::Floating => JointKind::Floating,
            urdf_rs::JointType::Planar => JointKind::Planar,
            urdf_rs::JointType::Spherical => JointKind::Spherical,
        };
        let limits = match kind {
            JointKind::Revolute => Some([joint.limit.lower.to_degrees(), joint.limit.upper.to_degrees()]),
            JointKind::Prismatic => Some([joint.limit.lower, joint.limit.upper]),
            _ => None,
        };
        let mimic = joint.mimic.as_ref().map(|m| {
            let offset = m.offset.unwrap_or(0.0);
            Mimic {
                joint: m.joint.clone(),
                multiplier: m.multiplier.unwrap_or(1.0),
                offset: if kind == JointKind::Prismatic { offset } else { offset.to_degrees() },
            }
        });
        scene.joints.push(Joint {
            name: joint.name.clone(),
            kind,
            parent,
            child,
            frame: scene.nodes[child].local,
            axis: DVec3::from_array(joint.axis.xyz.0).normalize_or_zero(),
            limits,
            mimic,
        });
    }
    Ok(scene)
}

/// A visual's material: its own colour, or the robot's material of that name.
fn material(scene: &mut Scene, m: &urdf_rs::Material, named: &HashMap<&str, &urdf_rs::Material>) -> Option<usize> {
    let color = m.color.as_ref().or_else(|| named.get(m.name.as_str()).and_then(|n| n.color.as_ref()))?;
    let textured = m.texture.is_some() || named.get(m.name.as_str()).is_some_and(|n| n.texture.is_some());
    let material = Material { name: m.name.clone(), base_color: color.rgba.0, metallic: 0.0, roughness: 0.5, textured };
    if let Some(i) = scene.materials.iter().position(|x| *x == material) {
        return Some(i);
    }
    scene.materials.push(material);
    Some(scene.materials.len() - 1)
}

/// Loads a mesh file a visual names: its meshes, each with the material its file gives.
fn load(scene: &mut Scene, file: &Path) -> Result<Vec<(usize, Option<usize>)>, ImportError> {
    match crate::Format::of_path(file) {
        Some(Format::Obj) => Ok(obj::objects(file)?
            .into_iter()
            .filter(|(m, _)| !m.triangles.is_empty())
            .map(|(mesh, material)| {
                let material = material.map(|m| {
                    scene.materials.push(m);
                    scene.materials.len() - 1
                });
                (scene.add_mesh(mesh), material)
            })
            .collect()),
        Some(Format::Stl) => {
            let (mesh, _) = stl::mesh(file)?;
            Ok(vec![(scene.add_mesh(mesh), None)])
        }
        _ => Err(ImportError(format!(
            "{} is a mesh format URDF import does not read; OBJ and STL are read",
            file.display()
        ))),
    }
}

/// Where a URDF mesh file name points: relative to the URDF, a `file://` path, or a
/// `package://` path found in a folder of that package's name above the URDF.
fn resolve(base: &Path, filename: &str) -> Option<PathBuf> {
    if let Some(rest) = filename.strip_prefix("file://") {
        let p = PathBuf::from(rest);
        return p.is_file().then_some(p);
    }
    if let Some(rest) = filename.strip_prefix("package://") {
        let (package, inner) = rest.split_once('/')?;
        let mut dir = Some(base);
        while let Some(d) = dir {
            for candidate in [d.join(package).join(inner), d.join(inner)] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
            if d.file_name().and_then(|n| n.to_str()) == Some(package) && d.join(inner).is_file() {
                return Some(d.join(inner));
            }
            dir = d.parent();
        }
        return None;
    }
    let p = base.join(filename);
    p.is_file().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_turn_about_fixed_axes_in_roll_pitch_yaw_order() {
        let p = urdf_rs::Pose {
            xyz: urdf_rs::Vec3([1.0, 2.0, 3.0]),
            rpy: urdf_rs::Vec3([0.0, 0.0, std::f64::consts::FRAC_PI_2]),
        };
        let m = pose(&p);
        assert!(m.transform_vector3(DVec3::X).abs_diff_eq(DVec3::Y, 1e-12));
        assert!(m.transform_point3(DVec3::ZERO).abs_diff_eq(DVec3::new(1.0, 2.0, 3.0), 1e-12));
        let p = urdf_rs::Pose {
            xyz: urdf_rs::Vec3([0.0; 3]),
            rpy: urdf_rs::Vec3([std::f64::consts::FRAC_PI_2, 0.0, std::f64::consts::FRAC_PI_2]),
        };
        // Roll first about X, then yaw about Z: Y goes to Z, then stays.
        assert!(pose(&p).transform_vector3(DVec3::Y).abs_diff_eq(DVec3::Z, 1e-12));
    }
}
