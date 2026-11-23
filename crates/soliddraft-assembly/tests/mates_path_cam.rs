//! Path and Cam mates on real parts: a stadium track and a block's corner run along its
//! outline, and rollers ride an eccentric disc cam and a stadium cam. Solves are bounded;
//! every accepted pose carries a residual certificate.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{DQuat, DVec3};
use soliddraft_assembly::{
    AssemblyDocument, ComponentId, EndpointSelector, FaceSelector, GeneratorChoice, KernelSource, Mate, MateEntry,
    OccurrencePath, OpenOptions, Placement, RefusalCode, Step,
};
use soliddraft_document::{CacheOnlyStore, DocRef, identify_file};
use soliddraft_kernel_api::Kernel;
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{P2, Sketch, solve::solve};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, ProfileSource, SketchFeature,
    SketchPlane,
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
    let dir = std::env::temp_dir().join(format!("soliddraft-path-cam-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A part extruded `depth` from a sketch on the Top plane.
fn part(path: &Path, depth: f64, draw: impl FnOnce(&mut Sketch)) {
    let mut doc = Document::new("Part", kernel());
    let mut sketch = Sketch::new();
    draw(&mut sketch);
    solve(&mut sketch, None, true);
    let kind = FeatureKind::Sketch(SketchFeature {
        plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 },
        sketch,
    });
    let s = doc
        .execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false })
        .unwrap()
        .created
        .unwrap();
    doc.execute(Command::AddFeature {
        kind: FeatureKind::Extrude(ExtrudeFeature {
            profile: ProfileSource::Sketch { feature: s },
            operation: ExtrudeOp::Boss,
            end: EndCondition::Blind { depth },
            reverse: false,
        }),
        name: None,
        position: None,
        via_assistant: false,
    })
    .unwrap();
    doc.save(path).unwrap();
}

fn reference(from: &Path, to: &Path) -> DocRef {
    let i = identify_file(to).unwrap();
    let mut r = DocRef::new(i.id, i.kind, "", i.revision);
    r.set_path(from, to);
    r
}

fn generator(component: ComponentId, generator: GeneratorChoice) -> EndpointSelector {
    EndpointSelector {
        component,
        face: FaceSelector::Generator { feature: "Boss-Extrude1".into(), generator, normal: None },
    }
}

fn corner(component: ComponentId) -> EndpointSelector {
    EndpointSelector {
        component,
        face: FaceSelector::Vertex(vec![
            "Boss-Extrude1 · Start cap".into(),
            "Boss-Extrude1 · Side of line 2".into(),
            "Boss-Extrude1 · Side of line 3".into(),
        ]),
    }
}

struct Rig {
    dir: PathBuf,
    file: PathBuf,
    doc: AssemblyDocument,
    base: ComponentId,
    mover: ComponentId,
}

/// `base` at the origin with an Origin mate, `mover` at `at`.
fn rig(name: &str, base: impl FnOnce(&Path), mover: impl FnOnce(&Path), at: Placement) -> Rig {
    let dir = folder(name);
    let (a, b, file) = (dir.join("base.dwprt"), dir.join("mover.dwprt"), dir.join("rig.dwasm"));
    base(&a);
    mover(&b);
    let mut doc = AssemblyDocument::new_with("Rig", options());
    doc.save(Some(&file)).unwrap();
    let base = doc.insert(reference(&file, &a), Placement::IDENTITY).unwrap();
    let mover = doc.insert(reference(&file, &b), at).unwrap();
    doc.finish_loading();
    doc.mate_origin("O", base, true, false).unwrap();
    Rig { dir, file, doc, base, mover }
}

fn certified(doc: &mut AssemblyDocument, name: &str) {
    let status = doc.solver();
    assert!(status.diagnostics.is_empty(), "{status:?}");
    let p = status.bounded.iter().find(|p| p.mate == name).unwrap_or_else(|| panic!("{name}: {status:?}"));
    assert_eq!(p.certainty, "bounded");
    assert!(p.residual_mm <= 1e-9 && p.residual_angle <= 1e-9, "{p:?}");
}

fn placement(doc: &AssemblyDocument, c: ComponentId) -> Placement {
    doc.assembly().component(c).unwrap().placement
}

/// The distance from `p` (in the plane z = `z`) to a stadium of half-length `h` and
/// radius `r` centred on the origin along X, signed positive outside.
fn stadium(p: DVec3, h: f64, r: f64) -> f64 {
    let x = (p.x.abs() - h).max(0.0);
    (x * x + p.y * p.y).sqrt() - r
}

#[test]
fn a_corner_runs_along_a_stadium_track() {
    let mut r = rig(
        "path",
        |p| {
            part(p, 5.0, |s| {
                let _ = s.slot(P2::new(0.0, 0.0), 60.0, 20.0).unwrap();
            })
        },
        |p| {
            part(p, 6.0, |s| {
                let _ = s.rectangle(P2::new(0.0, 0.0), 6.0, 6.0).unwrap();
            })
        },
        Placement::new(DQuat::from_rotation_z(0.3), DVec3::new(5.0, 18.0, 9.0)),
    );
    r.doc.mate_path("P", &generator(r.base, GeneratorChoice::EndCap), &corner(r.mover), false).unwrap();
    certified(&mut r.doc, "P");
    // Some corner of the block sits on the track's top outline: in its plane, on the curve.
    let on_track = |doc: &AssemblyDocument| {
        let at = placement(doc, r.mover);
        [(-3.0, -3.0), (3.0, -3.0), (3.0, 3.0), (-3.0, 3.0)]
            .iter()
            .map(|(x, y)| at.apply(DVec3::new(*x, *y, 0.0)))
            .any(|p| (p.z - 5.0).abs() < 1e-7 && stadium(p, 20.0, 10.0).abs() < 1e-7)
    };
    assert!(on_track(&r.doc), "{:?}", placement(&r.doc, r.mover));
    assert_eq!(r.doc.solver().remaining_dof, 4, "the corner slides along the path and the block turns about it");

    // A drag round the end of the track keeps the corner on it.
    let path = OccurrencePath::root().child(Step::component(r.mover));
    r.doc.begin_drag(path).unwrap();
    r.doc.preview_drag(Placement::new(DQuat::from_rotation_z(0.3), DVec3::new(38.0, 4.0, 7.0))).unwrap();
    r.doc.commit_drag().unwrap();
    certified(&mut r.doc, "P");
    assert!(on_track(&r.doc), "{:?}", placement(&r.doc, r.mover));

    // Saved and reopened, the mate and its stored outline come back and still hold.
    r.doc.save(None).unwrap();
    let mut back = AssemblyDocument::open(&r.file, options()).unwrap();
    back.finish_loading();
    let kinds: Vec<&str> = back.assembly().mates.iter().filter_map(MateEntry::known).map(Mate::kind).collect();
    assert!(kinds.contains(&"path"), "{kinds:?}");
    certified(&mut back, "P");

    // A cylinder is not a point: refused, nothing changes.
    let before = back.assembly().clone();
    let refused = back.mate_path(
        "P2",
        &generator(r.base, GeneratorChoice::EndCap),
        &generator(r.mover, GeneratorChoice::Side),
        false,
    );
    assert!(refused.is_err());
    assert_eq!(back.assembly(), &before);
    std::fs::remove_dir_all(r.dir).unwrap();
}

#[test]
fn a_roller_rides_an_eccentric_disc_and_a_stadium_cam() {
    // An eccentric disc: a 30 mm circle centred 5 mm off the origin.
    let mut r = rig(
        "cam-disc",
        |p| {
            part(p, 10.0, |s| {
                let _ = s.circle_at(P2::new(5.0, 0.0), 30.0).unwrap();
            })
        },
        |p| {
            part(p, 10.0, |s| {
                let _ = s.circle_at(P2::new(0.0, 0.0), 10.0).unwrap();
            })
        },
        Placement::translation(DVec3::new(40.0, 3.0, 0.0)),
    );
    r.doc
        .mate_cam("C", &generator(r.base, GeneratorChoice::EndCap), &generator(r.mover, GeneratorChoice::Side), false)
        .unwrap();
    certified(&mut r.doc, "C");
    let centre = placement(&r.doc, r.mover).translation;
    let reach = (DVec3::new(centre.x, centre.y, 0.0) - DVec3::new(5.0, 0.0, 0.0)).length();
    assert!((reach - 20.0).abs() < 1e-7, "the roller touches the disc from outside: {reach}");
    let Some(Mate::Cam { outside, .. }) =
        r.doc.assembly().mates.iter().filter_map(MateEntry::known).find(|m| m.name() == "C")
    else {
        panic!("a cam")
    };
    assert!(*outside);
    // A flat follower face is no follower.
    let before = r.doc.assembly().clone();
    let refused = r
        .doc
        .mate_cam(
            "C2",
            &generator(r.base, GeneratorChoice::EndCap),
            &generator(r.mover, GeneratorChoice::EndCap),
            false,
        )
        .unwrap_err();
    assert_eq!(refused.code, RefusalCode::UnsupportedReference);
    assert_eq!(r.doc.assembly(), &before);
    std::fs::remove_dir_all(r.dir).unwrap();

    // A stadium cam: lines and arcs. The roller starts above a flat side and stays on it.
    let mut r = rig(
        "cam-stadium",
        |p| {
            part(p, 10.0, |s| {
                let _ = s.slot(P2::new(0.0, 0.0), 60.0, 20.0).unwrap();
            })
        },
        |p| {
            part(p, 10.0, |s| {
                let _ = s.circle_at(P2::new(0.0, 0.0), 10.0).unwrap();
            })
        },
        Placement::translation(DVec3::new(4.0, 22.0, 0.0)),
    );
    r.doc
        .mate_cam("C", &generator(r.base, GeneratorChoice::EndCap), &generator(r.mover, GeneratorChoice::Side), false)
        .unwrap();
    certified(&mut r.doc, "C");
    let c = placement(&r.doc, r.mover).translation;
    assert!((stadium(DVec3::new(c.x, c.y, 0.0), 20.0, 10.0) - 5.0).abs() < 1e-7, "{c}");
    r.doc.save(None).unwrap();
    let mut back = AssemblyDocument::open(&r.file, options()).unwrap();
    back.finish_loading();
    certified(&mut back, "C");
    std::fs::remove_dir_all(r.dir).unwrap();
}
