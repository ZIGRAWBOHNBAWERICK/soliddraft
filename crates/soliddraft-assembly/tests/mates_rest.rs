//! The remaining mates on real parts: a Path that holds a position along its outline and
//! steers its part's pitch, yaw and roll, a pin in an arc slot, and the Mate Controller.
//! Each is built, edited, saved, reopened and has a later mate added on top.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{DQuat, DVec3};
use soliddraft_assembly::{
    Alignment, AssemblyDocument, ComponentId, EndpointSelector, FaceSelector, GeneratorChoice, KernelSource, Mate,
    MateEdit, MateEntry, OccurrencePath, OpenOptions, PathOptions, Placement, RefusalCode, SlotInput, Step,
    StoredSupport,
};
use soliddraft_document::{CacheOnlyStore, DocRef, identify_file};
use soliddraft_kernel_api::{Kernel, SurfaceKind};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{ConstraintKind, EntityKind, P2, PointRef, Sketch, SplineMode, solve::solve};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureId, FeatureKind, ProfileSource,
    SketchFeature, SketchPlane,
};

fn kernel() -> Arc<dyn Kernel> {
    Arc::new(CatallaxKernel::new())
}

fn options() -> OpenOptions {
    OpenOptions {
        display: Some(Arc::new(KernelSource::new(kernel()))),
        revisions: Some(Arc::new(CacheOnlyStore::new())),
        check_freshness: true,
        ..OpenOptions::default()
    }
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliddraft-mates-rest-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn add(doc: &mut Document, kind: FeatureKind) -> FeatureId {
    doc.execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false })
        .unwrap()
        .created
        .unwrap()
}

fn sketch(doc: &mut Document, draw: impl FnOnce(&mut Sketch)) -> FeatureId {
    let mut sketch = Sketch::new();
    draw(&mut sketch);
    solve(&mut sketch, None, true);
    let plane = SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 };
    add(doc, FeatureKind::Sketch(SketchFeature { plane, sketch }))
}

fn extrude(doc: &mut Document, s: FeatureId, operation: ExtrudeOp, depth: f64) {
    add(
        doc,
        FeatureKind::Extrude(ExtrudeFeature {
            profile: ProfileSource::Sketch { feature: s },
            operation,
            end: EndCondition::Blind { depth },
            reverse: false,
        }),
    );
}

/// A part made of one sketch extruded `h` up from the top plane.
fn prism(path: &Path, h: f64, draw: impl FnOnce(&mut Sketch)) {
    let mut doc = Document::new("Part", kernel());
    let s = sketch(&mut doc, draw);
    extrude(&mut doc, s, ExtrudeOp::Boss, h);
    doc.save(path).unwrap();
}

/// An 80 x 80 x 10 plate with an arc slot cut through it: its centre line a quarter circle
/// of radius 25 about the origin from +X to +Y, 8 wide, so its round ends' axes stand at
/// (25, 0) and (0, 25).
fn arc_slotted(path: &Path) {
    let mut doc = Document::new("Arc slotted", kernel());
    let s = sketch(&mut doc, |s| drop(s.rectangle(P2::new(0.0, 0.0), 80.0, 80.0).unwrap()));
    extrude(&mut doc, s, ExtrudeOp::Boss, 10.0);
    let s = sketch(&mut doc, |s| {
        let o = P2::new(0.0, 0.0);
        let outer = s.add(EntityKind::Arc { center: o, start: P2::new(29.0, 0.0), end: P2::new(0.0, 29.0) }, false);
        let top = s.add(
            EntityKind::Arc { center: P2::new(0.0, 25.0), start: P2::new(0.0, 29.0), end: P2::new(0.0, 21.0) },
            false,
        );
        let inner = s.add(EntityKind::Arc { center: o, start: P2::new(21.0, 0.0), end: P2::new(0.0, 21.0) }, false);
        let right = s.add(
            EntityKind::Arc { center: P2::new(25.0, 0.0), start: P2::new(21.0, 0.0), end: P2::new(29.0, 0.0) },
            false,
        );
        for (a, b) in
            [((outer, 2), (top, 1)), ((top, 2), (inner, 2)), ((inner, 1), (right, 1)), ((right, 2), (outer, 1))]
        {
            s.constrain(ConstraintKind::Coincident { a: PointRef::new(a.0, a.1), b: PointRef::new(b.0, b.1) });
        }
    });
    extrude(&mut doc, s, ExtrudeOp::Cut, 10.0);
    doc.save(path).unwrap();
}

fn reference(from: &Path, to: &Path) -> DocRef {
    let i = identify_file(to).unwrap();
    let mut r = DocRef::new(i.id, i.kind, "", i.revision);
    r.set_path(from, to);
    r
}

struct Rig {
    dir: PathBuf,
    file: PathBuf,
    doc: AssemblyDocument,
    ids: Vec<ComponentId>,
    parts: Vec<PathBuf>,
}

/// The first part at the origin under an Origin mate, the others where they are put.
/// A part to make, and where to place it.
type Made<'a> = (&'a dyn Fn(&Path), Placement);

fn rig(name: &str, parts: &[Made]) -> Rig {
    let dir = folder(name);
    let file = dir.join("rig.dwasm");
    let mut doc = AssemblyDocument::new_with("Rig", options());
    doc.save(Some(&file)).unwrap();
    let mut ids = Vec::new();
    let mut paths = Vec::new();
    for (i, (make, at)) in parts.iter().enumerate() {
        let p = dir.join(format!("p{i}.dwprt"));
        make(&p);
        ids.push(doc.insert(reference(&file, &p), *at).unwrap());
        paths.push(p);
    }
    doc.finish_loading();
    doc.mate_origin("O", ids[0], true, false).unwrap();
    Rig { dir, file, doc, ids, parts: paths }
}

impl Rig {
    fn placement(&self, i: usize) -> Placement {
        self.doc.assembly().component(self.ids[i]).unwrap().placement
    }

    fn generator(&self, i: usize, generator: GeneratorChoice) -> EndpointSelector {
        EndpointSelector {
            component: self.ids[i],
            face: FaceSelector::Generator { feature: "Boss-Extrude1".into(), generator, normal: None },
        }
    }

    fn label(&self, i: usize, label: &str) -> EndpointSelector {
        EndpointSelector { component: self.ids[i], face: FaceSelector::Label(label.into()) }
    }

    /// The face of a kind on part `i` whose centroid lies nearest `near`.
    fn nearest(&self, i: usize, kind: SurfaceKind, near: DVec3) -> EndpointSelector {
        let doc = Document::open(&self.parts[i], kernel()).unwrap();
        let t = doc.kernel().topology(doc.body().unwrap()).unwrap();
        let face = t
            .faces
            .iter()
            .filter(|f| f.surface == kind)
            .min_by(|a, b| {
                let d = |f: &&soliddraft_kernel_api::FaceInfo| {
                    (DVec3::new(f.centroid.x, f.centroid.y, f.centroid.z) - near).length()
                };
                d(a).partial_cmp(&d(b)).unwrap()
            })
            .unwrap();
        EndpointSelector { component: self.ids[i], face: FaceSelector::Picked(doc.face_ref(face.id).unwrap()) }
    }

    fn reopen(&mut self) {
        self.doc.save(None).unwrap();
        let mut back = AssemblyDocument::open(&self.file, options()).unwrap();
        back.finish_loading();
        self.doc = back;
    }

    fn mate(&self, name: &str) -> Mate {
        self.doc.assembly().mates.iter().filter_map(MateEntry::known).find(|m| m.name() == name).unwrap().clone()
    }
}

fn certified(doc: &mut AssemblyDocument, name: &str) {
    let status = doc.solver();
    assert!(status.diagnostics.is_empty(), "{status:?}");
    let p = status.bounded.iter().find(|p| p.mate == name).unwrap_or_else(|| panic!("{name}: {status:?}"));
    assert_eq!(p.certainty, "bounded");
    assert!(p.residual_mm <= 1e-9 && p.residual_angle <= 1e-9, "{p:?}");
}

/// Where a point `s` along a stored outline lies, and the way the outline runs there.
fn along(support: &StoredSupport, s: f64) -> (DVec3, DVec3) {
    let StoredSupport::Outline { normal, pieces, .. } = support else { panic!("an outline") };
    let n = DVec3::new(normal[0] as f64, normal[1] as f64, normal[2] as f64);
    let mut left = s;
    for (i, p) in pieces.iter().enumerate() {
        let from = DVec3::from_array(p.from);
        let to = DVec3::from_array(pieces[(i + 1) % pieces.len()].from);
        match p.centre.zip(p.radius) {
            None => {
                let len = (to - from).length();
                if left <= len {
                    let d = (to - from) / len;
                    return (from + d * left, d);
                }
                left -= len;
            }
            Some((c, r)) => {
                let c = DVec3::from_array(c);
                let sign = if p.anticlockwise { 1.0 } else { -1.0 };
                let (u, v) = (from - c, to - c);
                let mut sweep = (u.cross(v).dot(n) * sign).atan2(u.dot(v));
                if sweep <= 0.0 {
                    sweep += std::f64::consts::TAU;
                }
                if left <= r * sweep {
                    let q = DQuat::from_axis_angle(n * sign, left / r);
                    let w = q * u;
                    return (c + w, (n * sign).cross(w) / r);
                }
                left -= r * sweep;
            }
        }
    }
    panic!("{s} passes the end of the outline")
}

fn length(support: &StoredSupport) -> f64 {
    let StoredSupport::Outline { normal, pieces, .. } = support else { panic!("an outline") };
    let n = DVec3::new(normal[0] as f64, normal[1] as f64, normal[2] as f64);
    let mut total = 0.0;
    for (i, p) in pieces.iter().enumerate() {
        let (from, to) = (DVec3::from_array(p.from), DVec3::from_array(pieces[(i + 1) % pieces.len()].from));
        total += match p.centre.zip(p.radius) {
            None => (to - from).length(),
            Some((c, r)) => {
                let (u, v) = (from - DVec3::from_array(c), to - DVec3::from_array(c));
                let sign = if p.anticlockwise { 1.0 } else { -1.0 };
                let mut sweep = (u.cross(v).dot(n) * sign).atan2(u.dot(v));
                if sweep <= 0.0 {
                    sweep += std::f64::consts::TAU;
                }
                r * sweep
            }
        };
    }
    total
}

fn track(p: &Path) {
    prism(p, 5.0, |s| drop(s.slot(P2::new(0.0, 0.0), 60.0, 20.0).unwrap()))
}

fn cube(p: &Path) {
    prism(p, 6.0, |s| drop(s.rectangle(P2::new(0.0, 0.0), 6.0, 6.0).unwrap()))
}

fn shaft(d: f64, h: f64) -> impl Fn(&Path) {
    move |p| prism(p, h, |s| drop(s.circle_at(P2::new(0.0, 0.0), d).unwrap()))
}

/// The block's corner at (3, 3, 0) in its own frame.
fn corner(r: &Rig, i: usize) -> EndpointSelector {
    EndpointSelector {
        component: r.ids[i],
        face: FaceSelector::Vertex(vec![
            "Boss-Extrude1 · Start cap".into(),
            "Boss-Extrude1 · Side of line 2".into(),
            "Boss-Extrude1 · Side of line 3".into(),
        ]),
    }
}

#[test]
fn a_path_holds_a_position_and_steers_pitch_yaw_and_roll() {
    let start = Placement::new(DQuat::from_rotation_z(0.3), DVec3::new(5.0, 18.0, 9.0));
    let mut r = rig("path-position", &[(&track, Placement::IDENTITY), (&cube, start)]);
    let on = PathOptions {
        position: SlotInput::Distance { distance: "10 mm".into(), flipped: false },
        ..Default::default()
    };
    r.doc.mate_path_with("P", &r.generator(0, GeneratorChoice::EndCap), &corner(&r, 1), &on, false).unwrap();
    certified(&mut r.doc, "P");
    let Mate::Path { path, .. } = r.mate("P") else { panic!("a path") };
    let outline = path.support.clone().unwrap();
    let total = length(&outline);
    assert!((total - (80.0 + 20.0 * std::f64::consts::PI)).abs() < 1e-9, "{total}");
    let corner_at = |r: &Rig| r.placement(1).apply(DVec3::new(3.0, 3.0, 0.0));
    assert!((corner_at(&r) - along(&outline, 10.0).0).length() < 1e-7, "{}", corner_at(&r));
    assert_eq!(r.doc.solver().remaining_dof, 3, "the corner is held; the block turns about it");

    // Pitch and yaw follow the path with the block's +X side; roll keeps its top up.
    let before = r.doc.assembly().clone();
    let refused = r.doc.mate_path_with(
        "P2",
        &r.generator(0, GeneratorChoice::EndCap),
        &corner(&r, 1),
        &PathOptions { follow: Some((r.generator(0, GeneratorChoice::EndCap), Alignment::Aligned)), ..on.clone() },
        false,
    );
    assert_eq!(refused.unwrap_err().code, RefusalCode::Invalid, "a direction of the path's own part steers nothing");
    assert_eq!(r.doc.assembly(), &before);
    r.doc.delete_mate("P").unwrap();
    let steered = PathOptions {
        follow: Some((r.label(1, "Boss-Extrude1 · Side of line 2"), Alignment::Aligned)),
        up: Some((r.generator(1, GeneratorChoice::EndCap), Alignment::Aligned)),
        ..on.clone()
    };
    r.doc.mate_path_with("P", &r.generator(0, GeneratorChoice::EndCap), &corner(&r, 1), &steered, false).unwrap();
    certified(&mut r.doc, "P");
    assert_eq!(r.doc.solver().remaining_dof, 0);
    let steer = |r: &Rig, s: f64| {
        let (at, tangent) = along(&outline, s);
        let p = r.placement(1);
        assert!((corner_at(r) - at).length() < 1e-7, "{} against {at}", corner_at(r));
        assert!((p.rotation * DVec3::X - tangent).length() < 1e-7, "{} against {tangent}", p.rotation * DVec3::X);
        assert!((p.rotation * DVec3::Z - DVec3::Z).length() < 1e-7, "{}", p.rotation * DVec3::Z);
    };
    steer(&r, 10.0);

    // Edited round the first bend, then measured from the other way round.
    r.doc.edit_mates(&[("P".into(), MateEdit::Distance("50 mm".into()))]).unwrap();
    certified(&mut r.doc, "P");
    steer(&r, 50.0);
    r.doc.edit_mates(&[("P".into(), MateEdit::Flipped(true))]).unwrap();
    let (at, _) = along(&outline, total - 50.0);
    assert!((corner_at(&r) - at).length() < 1e-7);
    r.doc.edit_mates(&[("P".into(), MateEdit::Flipped(false))]).unwrap();

    // Past the end of the path is refused by name and changes nothing.
    let before = r.doc.assembly().clone();
    let refused = r.doc.edit_mates(&[("P".into(), MateEdit::Distance("400 mm".into()))]).unwrap_err();
    assert!(refused.message.contains("path"), "{}", refused.message);
    assert_eq!(r.doc.assembly(), &before);

    // Saved and reopened it holds; a later mate goes on top: a second block halfway round.
    r.reopen();
    certified(&mut r.doc, "P");
    steer(&r, 50.0);
    let saved = serde_json::to_value(r.mate("P")).unwrap();
    assert!(saved.get("follow").is_some() && saved.get("up").is_some(), "{saved}");
    let third = r.doc.insert(reference(&r.file, &r.parts[1]), Placement::translation(DVec3::new(-30.0, -20.0, 8.0)));
    let third = third.unwrap();
    r.ids.push(third);
    let half = PathOptions { position: SlotInput::Percent { percent: 50.0, flipped: false }, ..Default::default() };
    r.doc.mate_path_with("H", &r.generator(0, GeneratorChoice::EndCap), &corner(&r, 2), &half, false).unwrap();
    certified(&mut r.doc, "H");
    let (at, _) = along(&outline, total / 2.0);
    assert!((r.placement(2).apply(DVec3::new(3.0, 3.0, 0.0)) - at).length() < 1e-7);
    steer(&r, 50.0);

    // A free Path written before these options keeps its bytes.
    let mut f = rig("path-free", &[(&track, Placement::IDENTITY), (&cube, start)]);
    f.doc.mate_path("P", &f.generator(0, GeneratorChoice::EndCap), &corner(&f, 1), false).unwrap();
    f.doc.save(None).unwrap();
    let saved = serde_json::to_value(f.mate("P")).unwrap();
    let keys: Vec<&String> = saved.as_object().unwrap().keys().collect();
    assert!(["constraint", "follow", "up"].iter().all(|k| saved.get(k).is_none()), "{keys:?}");
    std::fs::remove_dir_all(f.dir).unwrap();
    std::fs::remove_dir_all(r.dir).unwrap();
}

#[test]
fn a_pin_rides_an_arc_slot_in_each_position_mode() {
    let pin = shaft(8.0, 20.0);
    let ends = |r: &Rig| {
        let Mate::Slot { slot, .. } = r.mate("S") else { panic!("a slot") };
        let Some(StoredSupport::ArcSlot { centre, ends, radius, .. }) = slot.support else {
            panic!("an arc slot: {:?}", slot.support)
        };
        (DVec3::from_array(centre), ends.map(DVec3::from_array), radius)
    };
    // Where the pin's axis meets the plate's top plane, and its angle from the first end.
    let angle = |r: &Rig| {
        let (c, e, radius) = ends(r);
        let p = r.placement(1).translation;
        let p = DVec3::new(p.x, p.y, c.z);
        assert!(((p - c).length() - radius).abs() < 1e-7, "on the centre line: {p}");
        (e[0] - c).angle_between(p - c)
    };
    let quarter = std::f64::consts::FRAC_PI_2;
    for (mode, expect, dof) in [
        (SlotInput::Center, quarter / 2.0, 2),
        (SlotInput::Distance { distance: "10 mm".into(), flipped: false }, 10.0 / 25.0, 2),
        (SlotInput::Distance { distance: "10 mm".into(), flipped: true }, quarter - 10.0 / 25.0, 2),
        (SlotInput::Percent { percent: 25.0, flipped: false }, quarter / 4.0, 2),
        (SlotInput::Free, f64::NAN, 3),
    ] {
        let mut r = rig(
            "arc-slot",
            &[(&arc_slotted, Placement::IDENTITY), (&pin, Placement::translation(DVec3::new(20.0, 22.0, 30.0)))],
        );
        let end = r.nearest(0, SurfaceKind::Cylinder, DVec3::new(25.0, -4.0, 5.0));
        let side = r.nearest(1, SurfaceKind::Cylinder, DVec3::ZERO);
        r.doc.mate_slot("S", &end, &side, Alignment::Aligned, &mode, false, false).unwrap();
        certified(&mut r.doc, "S");
        let a = angle(&r);
        if expect.is_nan() {
            assert!((0.0..=quarter).contains(&a), "{a}");
        } else {
            assert!((a - expect).abs() < 1e-7, "{mode:?}: {a} against {expect}");
        }
        assert_eq!(r.doc.solver().remaining_dof, dof, "{mode:?}");
    }

    // Free: a pin started beyond an end comes into the slot; a drag along it keeps it on the arc.
    let mut r = rig(
        "arc-slot-free",
        &[(&arc_slotted, Placement::IDENTITY), (&pin, Placement::translation(DVec3::new(30.0, -20.0, 30.0)))],
    );
    let end = r.nearest(0, SurfaceKind::Cylinder, DVec3::new(25.0, -4.0, 5.0));
    let side = r.nearest(1, SurfaceKind::Cylinder, DVec3::ZERO);
    r.doc.mate_slot("S", &end, &side, Alignment::Aligned, &SlotInput::Free, false, false).unwrap();
    certified(&mut r.doc, "S");
    let a = angle(&r);
    assert!((-1e-9..=quarter + 1e-9).contains(&a), "between the ends: {a}");
    let path = OccurrencePath::root().child(Step::component(r.ids[1]));
    r.doc.begin_drag(path).unwrap();
    r.doc.preview_drag(Placement::translation(DVec3::new(14.0, 22.0, 30.0))).unwrap();
    r.doc.commit_drag().unwrap();
    certified(&mut r.doc, "S");
    let a = angle(&r);
    assert!(a > 0.1 && a < quarter - 0.1, "{a}");

    // Edited to a distance, then refused past the slot's length; nothing changes.
    let mut r = rig(
        "arc-slot-edit",
        &[(&arc_slotted, Placement::IDENTITY), (&pin, Placement::translation(DVec3::new(20.0, 22.0, 30.0)))],
    );
    let end = r.nearest(0, SurfaceKind::Cylinder, DVec3::new(25.0, -4.0, 5.0));
    let side = r.nearest(1, SurfaceKind::Cylinder, DVec3::ZERO);
    let mode = SlotInput::Distance { distance: "10 mm".into(), flipped: false };
    r.doc.mate_slot("S", &end, &side, Alignment::Aligned, &mode, false, false).unwrap();
    r.doc.edit_mates(&[("S".into(), MateEdit::Distance("30 mm".into()))]).unwrap();
    assert!((angle(&r) - 30.0 / 25.0).abs() < 1e-7);
    let before = r.doc.assembly().clone();
    assert!(r.doc.edit_mates(&[("S".into(), MateEdit::Distance("50 mm".into()))]).is_err());
    assert_eq!(r.doc.assembly(), &before);

    // Saved and reopened it holds; a later Distance on top takes the pin's height.
    r.reopen();
    certified(&mut r.doc, "S");
    assert!((angle(&r) - 30.0 / 25.0).abs() < 1e-7);
    let dof = r.doc.solver().remaining_dof;
    r.doc
        .mate_distance(
            "D",
            &r.generator(0, GeneratorChoice::EndCap),
            &r.generator(1, GeneratorChoice::StartCap),
            "2 mm",
            Alignment::Opposed,
            false,
        )
        .unwrap();
    assert_eq!(r.doc.solver().remaining_dof, dof - 1);
    assert!((r.placement(1).translation.z - 12.0).abs() < 1e-7, "{}", r.placement(1).translation);
    std::fs::remove_dir_all(r.dir).unwrap();
}

#[test]
fn the_mate_controller_saves_applies_and_animates_positions() {
    let pin = shaft(8.0, 20.0);
    let mut r = rig(
        "controller",
        &[(&arc_slotted, Placement::IDENTITY), (&pin, Placement::translation(DVec3::new(20.0, 22.0, 30.0)))],
    );
    let end = r.nearest(0, SurfaceKind::Cylinder, DVec3::new(25.0, -4.0, 5.0));
    let side = r.nearest(1, SurfaceKind::Cylinder, DVec3::ZERO);
    let mode = SlotInput::Distance { distance: "10 mm".into(), flipped: false };
    r.doc.mate_slot("S", &end, &side, Alignment::Aligned, &mode, false, false).unwrap();
    let (top, bottom) = (r.generator(0, GeneratorChoice::EndCap), r.generator(1, GeneratorChoice::StartCap));
    r.doc.mate_distance("D", &top, &bottom, "2 mm", Alignment::Opposed, false).unwrap();
    // Where the pin is: its angle round the slot from (25, 0) and its height.
    let pose = |r: &Rig| {
        let p = r.placement(1).translation;
        (p.y.atan2(p.x), p.z)
    };
    let near = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-7 && (a.1 - b.1).abs() < 1e-7;

    // Origin is no driven value; nothing changes.
    let before = r.doc.assembly().clone();
    assert!(r.doc.set_controller_mates(&["O".into()]).is_err());
    assert_eq!(r.doc.assembly(), &before);

    r.doc.set_controller_mates(&["S".into(), "D".into()]).unwrap();
    assert_eq!(r.doc.controller_mates(), ["S", "D"]);
    r.doc.save_controller_position("Low").unwrap();
    r.doc.apply_controller_values(&[30.0, 8.0]).unwrap();
    assert!(near(pose(&r), (30.0 / 25.0, 18.0)), "{:?}", pose(&r));
    r.doc.save_controller_position("High").unwrap();
    let positions = &r.doc.mate_controller().unwrap().positions;
    assert_eq!(positions.iter().map(|p| p.values.clone()).collect::<Vec<_>>(), [vec![10.0, 2.0], vec![30.0, 8.0]]);

    // Applying a position is one undo step.
    let depth = r.doc.undo_depth();
    r.doc.apply_controller_position("Low").unwrap();
    assert!(near(pose(&r), (10.0 / 25.0, 12.0)), "{:?}", pose(&r));
    assert_eq!(r.doc.undo_depth(), depth + 1);
    assert!(r.doc.undo());
    assert!(near(pose(&r), (30.0 / 25.0, 18.0)), "{:?}", pose(&r));
    r.doc.apply_controller_position("Low").unwrap();

    // Frames between two positions solve each step and change nothing.
    let before = r.doc.assembly().clone();
    let frames = r.doc.controller_frames("Low", "High", 4).unwrap();
    assert_eq!(frames.len(), 5);
    let at = |k: usize| frames[k].iter().find(|(c, _)| *c == r.ids[1]).unwrap().1.translation;
    let middle = at(2);
    assert!(near((middle.y.atan2(middle.x), middle.z), (20.0 / 25.0, 15.0)), "{middle}");
    assert_eq!(r.doc.assembly(), &before);
    // A preview shows a frame and leaves the truth alone.
    r.doc.preview_controller_values(&r.doc.controller_values_between("Low", "High", 0.5).unwrap()).unwrap();
    assert_eq!(r.doc.assembly(), &before);
    r.doc.cancel_drag();

    // A position past the slot's end is refused by name and changes nothing.
    assert!(r.doc.apply_controller_values(&[50.0, 2.0]).is_err());
    assert_eq!(r.doc.assembly(), &before);

    // Saved and reopened, the controller and its positions come back and still apply.
    r.reopen();
    assert_eq!(r.doc.controller_mates(), ["S", "D"]);
    r.doc.apply_controller_position("High").unwrap();
    assert!(near(pose(&r), (30.0 / 25.0, 18.0)), "{:?}", pose(&r));

    // A later mate goes on top: a third part's Distance joins the controller, and the saved
    // positions take its current value. Deleting D takes it out of every position.
    let third = r.doc.insert(reference(&r.file, &r.parts[1]), Placement::translation(DVec3::new(-30.0, -30.0, 40.0)));
    r.ids.push(third.unwrap());
    let (top, bottom) = (r.generator(0, GeneratorChoice::EndCap), r.generator(2, GeneratorChoice::StartCap));
    r.doc.mate_distance("E", &top, &bottom, "6 mm", Alignment::Opposed, false).unwrap();
    r.doc.set_controller_mates(&["S".into(), "D".into(), "E".into()]).unwrap();
    assert_eq!(r.doc.mate_controller().unwrap().positions[0].values, [10.0, 2.0, 6.0]);
    r.doc.delete_mate("D").unwrap();
    assert_eq!(r.doc.controller_mates(), ["S", "E"]);
    assert_eq!(r.doc.mate_controller().unwrap().positions[1].values, [30.0, 6.0]);
    r.doc.apply_controller_position("Low").unwrap();
    assert!((pose(&r).0 - 10.0 / 25.0).abs() < 1e-7);
    std::fs::remove_dir_all(r.dir).unwrap();
}

fn wait_motion(doc: &mut AssemblyDocument) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while doc.drag_pending() {
        doc.poll_drag();
        assert!(std::time::Instant::now() < deadline, "collision worker timed out");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    if let Some(report) = doc.drag_report() {
        assert!(report.error.is_none(), "{report:?}");
    }
}

/// Two 6 mm blocks sit on a plate, free to slide on it; a third is locked to it further on.
#[test]
fn physical_dynamics_pushes_what_the_dragged_part_meets() {
    let plate = |p: &Path| prism(p, 5.0, |s| drop(s.rectangle(P2::new(0.0, 0.0), 120.0, 40.0).unwrap()));
    let at = |x: f64| Placement::translation(DVec3::new(x, 0.0, 5.0));
    let mut r =
        rig("dynamics", &[(&plate, Placement::IDENTITY), (&cube, at(-20.0)), (&cube, at(0.0)), (&cube, at(40.0))]);
    let top = r.generator(0, GeneratorChoice::EndCap);
    // A millimetre above the plate, so sliding over it is no contact that changes.
    for (name, i) in [("A", 1), ("B", 2)] {
        let bottom = r.generator(i, GeneratorChoice::StartCap);
        r.doc.mate_distance(name, &top, &bottom, "1 mm", Alignment::Opposed, false).unwrap();
    }
    r.doc.mate_lock("C", r.ids[0], r.ids[3], false).unwrap();
    let occurrence = |id: ComponentId| OccurrencePath::root().child(Step::component(id));
    let x = |r: &Rig, i: usize| r.doc.table().world(r.doc.table().find(&occurrence(r.ids[i])).unwrap()).translation.x;

    // A guarded drag stops at the block in the way.
    r.doc.begin_guarded_drag(occurrence(r.ids[1]), None).unwrap();
    r.doc.preview_drag(at(20.0)).unwrap();
    wait_motion(&mut r.doc);
    let report = r.doc.drag_report().unwrap().clone();
    assert!(report.stopped, "{report:?}");
    let ats: Vec<Placement> = (0..4).map(|i| r.placement(i)).collect();
    assert!(x(&r, 1) > -6.03 && x(&r, 1) <= -6.0, "{}: {report:?} {ats:?}", x(&r, 1));
    r.doc.cancel_drag();

    // Physical Dynamics pushes it along instead, and the release is one undo step.
    let depth = r.doc.undo_depth();
    r.doc.begin_dynamic_drag(occurrence(r.ids[1]), None).unwrap();
    r.doc.preview_drag(at(20.0)).unwrap();
    wait_motion(&mut r.doc);
    let report = r.doc.drag_report().unwrap().clone();
    assert!(!report.stopped, "{report:?}");
    assert_eq!(report.pushed, [occurrence(r.ids[2])]);
    assert!((x(&r, 1) - 20.0).abs() < 1e-9, "{}", x(&r, 1));
    let gap = x(&r, 2) - x(&r, 1) - 6.0;
    assert!((0.0..0.05).contains(&gap), "the pushed block stays just ahead: {gap}");
    r.doc.commit_drag().unwrap();
    wait_motion(&mut r.doc);
    assert_eq!(r.doc.undo_depth(), depth + 1);
    let pushed_to = r.placement(2).translation.x;
    assert!((pushed_to - x(&r, 2)).abs() < 1e-12);

    // The locked block holds: the chain stops against it.
    r.doc.begin_dynamic_drag(occurrence(r.ids[1]), None).unwrap();
    r.doc.preview_drag(at(60.0)).unwrap();
    wait_motion(&mut r.doc);
    let report = r.doc.drag_report().unwrap().clone();
    assert!(report.stopped, "{report:?}");
    assert!(x(&r, 2) <= 34.0 && x(&r, 2) > 33.97, "{}", x(&r, 2));
    assert!((x(&r, 3) - 40.0).abs() < 1e-12);
    r.doc.commit_drag().unwrap();
    wait_motion(&mut r.doc);

    // Saved and reopened, the pushed places hold; a later mate on top keeps them.
    let (a, b) = (r.placement(1), r.placement(2));
    r.reopen();
    assert_eq!((r.placement(1), r.placement(2)), (a, b));
    r.doc.mate_lock("L", r.ids[1], r.ids[2], false).unwrap();
    assert_eq!((r.placement(1), r.placement(2)), (a, b));

    // Only a root component is dragged this way.
    assert!(r.doc.begin_dynamic_drag(OccurrencePath::root(), None).is_err());
    std::fs::remove_dir_all(r.dir).unwrap();
}

/// The nearest distance from `p` to the cubic Bezier through `c`, in the plane z = 0.
fn bezier_distance(c: [DVec3; 4], p: DVec3) -> f64 {
    let at = |t: f64| {
        let s = 1.0 - t;
        c[0] * (s * s * s) + c[1] * (3.0 * s * s * t) + c[2] * (3.0 * s * t * t) + c[3] * (t * t * t)
    };
    let d = |t: f64| (at(t) - p).length();
    let mut best = (0..=4000).map(|i| i as f64 / 4000.0).min_by(|a, b| d(*a).total_cmp(&d(*b))).unwrap();
    let (mut lo, mut hi) = ((best - 1.0 / 4000.0).max(0.0), (best + 1.0 / 4000.0).min(1.0));
    for _ in 0..200 {
        let (m1, m2) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
        if d(m1) < d(m2) {
            hi = m2
        } else {
            lo = m1
        }
        best = (lo + hi) / 2.0;
    }
    d(best)
}

/// A cam whose top is a spline arch: a clamped cubic through (-20, 0), (-20, 30), (20, 30)
/// and (20, 0), closed by a line along the bottom.
fn arch(p: &Path) {
    prism(p, 10.0, |s| {
        let points = vec![P2::new(-20.0, 0.0), P2::new(-20.0, 30.0), P2::new(20.0, 30.0), P2::new(20.0, 0.0)];
        s.add(EntityKind::Spline { mode: SplineMode::Control, points }, false);
        s.add(EntityKind::Line { a: P2::new(20.0, 0.0), b: P2::new(-20.0, 0.0) }, false);
    })
}

#[test]
fn a_roller_rides_a_spline_cam_and_a_corner_runs_a_spline_path() {
    let curve = [
        DVec3::new(-20.0, 0.0, 0.0),
        DVec3::new(-20.0, 30.0, 0.0),
        DVec3::new(20.0, 30.0, 0.0),
        DVec3::new(20.0, 0.0, 0.0),
    ];
    let roller = shaft(10.0, 10.0);
    let mut r = rig(
        "spline-cam",
        &[(&arch, Placement::IDENTITY), (&roller, Placement::translation(DVec3::new(3.0, 40.0, 0.0)))],
    );
    r.doc
        .mate_cam("C", &r.generator(0, GeneratorChoice::EndCap), &r.generator(1, GeneratorChoice::Side), false)
        .unwrap();
    certified(&mut r.doc, "C");
    let Mate::Cam { cam, outside, .. } = r.mate("C") else { panic!("a cam") };
    assert!(outside);
    let Some(StoredSupport::Outline { pieces, .. }) = &cam.support else { panic!("an outline") };
    assert!(pieces.iter().any(|p| p.spline.is_some()), "{pieces:?}");
    let centre = |r: &Rig| {
        let t = r.placement(1).translation;
        DVec3::new(t.x, t.y, 0.0)
    };
    let reach = bezier_distance(curve, centre(&r));
    assert!((reach - 5.0).abs() < 1e-6, "the roller touches the arch: {reach} at {}", centre(&r));

    // A drag over the top keeps it touching; saved and reopened it holds.
    let path = OccurrencePath::root().child(Step::component(r.ids[1]));
    r.doc.begin_drag(path).unwrap();
    r.doc.preview_drag(Placement::translation(DVec3::new(-12.0, 36.0, 0.0))).unwrap();
    r.doc.commit_drag().unwrap();
    certified(&mut r.doc, "C");
    let reach = bezier_distance(curve, centre(&r));
    assert!((reach - 5.0).abs() < 1e-6, "{reach} at {}", centre(&r));
    r.reopen();
    certified(&mut r.doc, "C");
    // A later mate on top: the roller's bottom on the cam's.
    let dof = r.doc.solver().remaining_dof;
    let (a, b) = (r.generator(0, GeneratorChoice::StartCap), r.generator(1, GeneratorChoice::StartCap));
    r.doc.mate_coincident("Z", &a, &b, Alignment::Aligned, false).unwrap();
    assert_eq!(r.doc.solver().remaining_dof, dof - 1);
    std::fs::remove_dir_all(r.dir).unwrap();

    // A block's corner a quarter of the way round the arch's outline.
    let start = Placement::translation(DVec3::new(5.0, 40.0, 12.0));
    let mut r = rig("spline-path", &[(&arch, Placement::IDENTITY), (&cube, start)]);
    let quarter = PathOptions { position: SlotInput::Percent { percent: 25.0, flipped: false }, ..Default::default() };
    r.doc.mate_path_with("P", &r.generator(0, GeneratorChoice::EndCap), &corner(&r, 1), &quarter, false).unwrap();
    certified(&mut r.doc, "P");
    let p = r.placement(1).apply(DVec3::new(3.0, 3.0, 0.0));
    let on = bezier_distance(curve, DVec3::new(p.x, p.y, 0.0)).min(if p.y.abs() < 1e-7 { 0.0 } else { f64::MAX });
    assert!(on < 1e-7 && (p.z - 10.0).abs() < 1e-7, "on the arch's outline: {p}");
    std::fs::remove_dir_all(r.dir).unwrap();
}
