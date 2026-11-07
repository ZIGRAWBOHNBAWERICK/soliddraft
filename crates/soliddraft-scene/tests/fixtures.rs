//! The committed sand3d and Omniverse samples read with the right tree, names, scale,
//! orientation, materials and joints, and the same object agrees across its formats.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use glam::DVec3;
use soliddraft_scene::{Format, JointKind, Options, Origin, Scene, UpAxis, read};

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ci/fixtures/scene-import").join(rel)
}

fn open(rel: &str) -> Scene {
    read(&fixture(rel), &Options::default()).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// Every body's world box by its node path.
fn boxes(s: &Scene) -> BTreeMap<String, [DVec3; 2]> {
    let mut out = BTreeMap::new();
    for (i, n) in s.nodes.iter().enumerate() {
        let Some(b) = n.body else { continue };
        let w = s.world(i);
        let mut lo = DVec3::splat(f64::INFINITY);
        let mut hi = DVec3::splat(f64::NEG_INFINITY);
        for p in &s.meshes[b.mesh].positions {
            let q = w.transform_point3(*p);
            lo = lo.min(q);
            hi = hi.max(q);
        }
        out.insert(s.path(i), [lo, hi]);
    }
    out
}

/// Every body's world box by its body's name, for comparing formats whose trees differ.
fn by_name(s: &Scene) -> BTreeMap<String, [DVec3; 2]> {
    let named: BTreeMap<String, [DVec3; 2]> =
        boxes(s).into_iter().map(|(path, b)| (path.rsplit('/').next().unwrap_or_default().to_owned(), b)).collect();
    named
}

fn near(a: DVec3, b: DVec3, tol: f64) -> bool {
    (a - b).abs().max_element() <= tol
}

#[test]
fn the_bracket_is_80_by_40_by_6_mm_on_z_0_in_every_format() {
    let lo = DVec3::new(-40.0, -20.0, 0.0);
    let hi = DVec3::new(40.0, 20.0, 6.0);
    for (file, format, path) in [
        ("sand3d-bracket/bracket.usda", Format::Usd, "Geometry/plate/plate"),
        ("sand3d-bracket/bracket.glb", Format::Gltf, "plate"),
        ("sand3d-bracket/bracket.urdf", Format::Urdf, "plate"),
        ("sand3d-bracket/bracket.obj", Format::Obj, "plate"),
        ("sand3d-bracket/bracket.stl", Format::Stl, "plate"),
    ] {
        let s = open(file);
        assert_eq!(s.format, format, "{file}");
        let b = boxes(&s);
        assert_eq!(b.keys().collect::<Vec<_>>(), [path], "{file}: one body, named as the file names it");
        let [l, h] = b[path];
        assert!(near(l, lo, 1e-3) && near(h, hi, 1e-3), "{file}: {l} {h}");
        assert!(s.joints.is_empty(), "{file}");
    }
}

#[test]
fn units_and_up_axes_say_where_they_came_from() {
    let s = open("sand3d-bracket/bracket.usda");
    assert_eq!((s.meters_per_unit, s.units_from, s.up_axis, s.up_from), (1.0, Origin::File, UpAxis::Z, Origin::File));
    assert_eq!(s.name, "plate");
    let s = open("sand3d-bracket/bracket.glb");
    assert_eq!((s.units_from, s.up_axis, s.up_from), (Origin::Format, UpAxis::Y, Origin::Format));
    let s = open("sand3d-bracket/bracket.urdf");
    assert_eq!((s.units_from, s.up_axis), (Origin::Format, UpAxis::Z));
    let s = open("sand3d-bracket/bracket.obj");
    assert_eq!(
        (s.meters_per_unit, s.units_from, s.up_axis, s.up_from),
        (1.0, Origin::Header, UpAxis::Y, Origin::Header)
    );
    let s = open("sand3d-bracket/bracket.stl");
    assert_eq!(
        (s.meters_per_unit, s.units_from, s.up_axis, s.up_from),
        (1.0, Origin::Header, UpAxis::Z, Origin::Header)
    );
    // The caller decides over the header.
    let s =
        read(&fixture("sand3d-bracket/bracket.stl"), &Options { meters_per_unit: Some(0.001), up_axis: None }).unwrap();
    assert_eq!(s.units_from, Origin::Option);
    let [_, hi] = s.bounds().unwrap();
    assert!(near(hi, DVec3::new(0.04, 0.02, 0.006), 1e-6), "{hi}");
}

const WASHER_PARTS: [&str; 7] = ["cabinet", "dial", "drawer", "drum", "glass", "handle", "ring"];

#[test]
fn the_washer_has_the_same_parts_in_the_same_places_in_every_format() {
    let usd = by_name(&open("sand3d-washer/washer.usda"));
    let glb = by_name(&open("sand3d-washer/washer.glb"));
    let urdf = by_name(&open("sand3d-washer/washer.urdf"));
    for b in [&usd, &glb, &urdf] {
        assert_eq!(b.keys().map(String::as_str).collect::<Vec<_>>(), WASHER_PARTS);
    }
    for part in WASHER_PARTS {
        for (other, name) in [(&glb, "glb"), (&urdf, "urdf")] {
            let ([a0, a1], [b0, b1]) = (usd[part], other[part]);
            assert!(near(a0, b0, 0.01) && near(a1, b1, 0.01), "{part} in {name}: {a0} {a1} vs {b0} {b1}");
        }
    }
    let [lo, hi] = usd["cabinet"];
    assert!(near(hi - lo, DVec3::new(600.0, 600.0, 850.0), 0.01), "the cabinet is 600 x 600 x 850 mm: {}", hi - lo);
    assert!(lo.z.abs() < 0.01, "standing on Z = 0: {lo}");
}

#[test]
fn the_washer_trees_follow_their_files() {
    let s = open("sand3d-washer/washer.urdf");
    assert_eq!(s.name, "washing_machine");
    let paths: Vec<String> = (1..s.nodes.len()).map(|i| s.path(i)).collect();
    assert_eq!(
        paths,
        [
            "cabinet",
            "dial",
            "door",
            "drawer",
            "drum",
            "dial/dial",
            "door/glass",
            "door/handle",
            "door/ring",
            "drawer/drawer",
            "drum/drum"
        ]
    );
    let s = open("sand3d-washer/washer.glb");
    let paths: Vec<String> = (1..s.nodes.len()).map(|i| s.path(i)).collect();
    assert!(paths.contains(&"washing_machine/cabinet/door_hinge/door/glass".to_owned()), "{paths:?}");
    let s = open("sand3d-washer/washer.usda");
    let paths: Vec<String> = (1..s.nodes.len()).map(|i| s.path(i)).collect();
    assert!(paths.contains(&"Geometry/door/glass".to_owned()), "{paths:?}");
    assert!(!paths.iter().any(|p| p.starts_with("Looks") || p.starts_with("Joints")), "{paths:?}");
}

#[test]
fn the_washer_joints_agree_in_every_format() {
    // Name, kind, limits and the axis in the root's frame.
    type Seen = BTreeMap<String, (JointKind, Option<[f64; 2]>, DVec3, DVec3)>;
    let joints = |s: &Scene| -> Seen {
        s.joints
            .iter()
            .map(|j| {
                let f = s.world(j.parent) * j.frame;
                let axis = f.transform_vector3(j.axis).normalize();
                (j.name.clone(), (j.kind, j.limits, axis, f.transform_point3(DVec3::ZERO)))
            })
            .collect()
    };
    let usd = joints(&open("sand3d-washer/washer.usda"));
    let glb = joints(&open("sand3d-washer/washer.glb"));
    let urdf = joints(&open("sand3d-washer/washer.urdf"));
    assert_eq!(
        usd.keys().map(String::as_str).collect::<Vec<_>>(),
        ["dial_turn", "door_hinge", "drawer_slide", "drum_spin"]
    );
    for (name, (kind, limits, axis, _)) in &urdf {
        for (other, label) in [(&usd, "usd"), (&glb, "glb")] {
            let (k, l, a, _) = other[name];
            assert_eq!(k, *kind, "{name} in {label}");
            assert!(near(a, *axis, 1e-6), "{name} in {label}: axis {a} vs {axis}");
            match (l, limits) {
                (Some(x), Some(y)) => {
                    assert!((x[0] - y[0]).abs() < 1e-3 && (x[1] - y[1]).abs() < 1e-3, "{name} in {label}")
                }
                (None, None) => {}
                _ => panic!("{name} in {label}: limits {l:?} vs {limits:?}"),
            }
        }
    }
    let (kind, limits, axis, _) = urdf["door_hinge"];
    assert_eq!(kind, JointKind::Revolute);
    let [lo, hi] = limits.unwrap();
    assert!(lo.abs() < 1e-9 && (hi - 143.0).abs() < 1e-6, "{lo} {hi}");
    assert!(near(axis, -DVec3::Z, 1e-6), "the door turns about -Z: {axis}");
    let (kind, limits, axis, _) = urdf["drawer_slide"];
    assert_eq!(kind, JointKind::Prismatic);
    assert!(near(DVec3::from_array([limits.unwrap()[0], limits.unwrap()[1], 0.0]), DVec3::new(0.0, 200.0, 0.0), 1e-6));
    assert!(near(axis, DVec3::X, 1e-6), "the drawer slides out along +X: {axis}");
    assert_eq!(urdf["drum_spin"].0, JointKind::Continuous);
}

#[test]
fn the_washer_keeps_its_materials() {
    // sand3d writes the glass's transmission into USD's opacity, which it authors as 0;
    // the GLB and URDF keep the look's 40 % opacity.
    for (file, opacity) in
        [("sand3d-washer/washer.usda", 0.0), ("sand3d-washer/washer.glb", 0.4), ("sand3d-washer/washer.urdf", 0.4)]
    {
        let s = open(file);
        let glass = s.nodes.iter().find(|n| n.name == "glass" && n.body.is_some()).expect(file);
        let m = &s.materials[glass.body.unwrap().material.expect(file)];
        let [r, g, b, a] = m.base_color;
        // #cfe3e6 at 40 % opacity.
        assert!(
            (r - 207.0 / 255.0).abs() < 0.01 && (g - 227.0 / 255.0).abs() < 0.01 && (b - 230.0 / 255.0).abs() < 0.01,
            "{file}: {m:?}"
        );
        assert!((a - opacity).abs() < 0.01, "{file}: {m:?}");
    }
}

#[test]
fn the_omniverse_box_reads_from_binary_usd() {
    let s = open("omniverse-cubebox/cubebox_a02.usd");
    assert_eq!((s.name.as_str(), s.meters_per_unit, s.up_axis), ("World", 1.0, UpAxis::Z));
    let b = boxes(&s);
    assert_eq!(
        b.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "cubebox_a02_inst/SM_CubeBox_A02_Body_01/M_CubeBox_A01_Body",
            "cubebox_a02_inst/SM_CubeBox_A02_Body_01/M_CubeBox_A01_Details",
            "cubebox_a02_inst/SM_CubeBox_A02_Decal_01",
            "cubebox_a02_inst/SM_CubeBox_A02_Scotch_01",
        ]
    );
    let [lo, hi] = s.bounds().unwrap();
    let size = hi - lo;
    // A 160 mm box, standing on Z = 0.
    assert!(size.max_element() < 170.0 && size.min_element() > 150.0, "{size}");
    assert!(lo.z.abs() < 1.0, "{lo}");
    assert_eq!(s.materials.len(), 2);
    assert!(s.materials.iter().all(|m| m.textured), "{:?}", s.materials);
}
