//! The policy, the worker and the cache on the Catallax kernel.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use soliddraft_kernel_api::contract;
use soliddraft_kernel_api::{
    BodyId, FaceId, FaceLevel, FaceLevelPoints, Kernel, KernelError, LoftShape, Loop, Mesh, OpTag, Plane, Point2,
    Profile, Revolve, Segment, Vec3,
};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_tessellation::budget::POINT_BYTES;
use soliddraft_tessellation::{
    DeviceBudget, DisplayCache, FaceLevels, FaceMesh, Level, Levels, Planned, Quality, Tessellator, Thresholds, Usage,
    View, Weigh, report,
};

/// The hang guard every test shares: a wait this long means stuck, not slow.
const WAIT: Duration = Duration::from_secs(600);

/// A cylinder exercises chord refinement without waiting for kernel fillets.
fn curved_body(k: &dyn Kernel) -> soliddraft_kernel_api::BodyId {
    contract::cylinder(k, Vec3::ZERO, 10.0, 20.0)
}

/// The stations of [`spindle`] along x: each a circle's radius and how far its centre
/// sits off the axis.
const STATIONS: [(f64, f64, f64); 8] = [
    (-70.0, 10.0, 0.0),
    (-50.0, 7.0, 2.0),
    (-30.0, 7.0, 2.0),
    (-10.0, 10.0, 0.0),
    (10.0, 10.0, 0.0),
    (30.0, 7.0, 2.0),
    (50.0, 7.0, 2.0),
    (70.0, 10.0, 0.0),
];

/// A spindle 140 mm long: a ruled loft through eight circles along x. Between a wide
/// circle and a narrow one off the axis the wall is a tapered spline face, which the
/// kernel refines in rounds and so gives distinct levels. The four tapers are 20 mm long
/// and 20 mm apart, with a straight wall between them, far enough for a camera to hold
/// one and lose the rest.
fn spindle(k: &dyn Kernel) -> BodyId {
    let sections: Vec<BodyId> = STATIONS
        .iter()
        .map(|(x, radius, offset)| {
            let plane = Plane::YZ.with_origin(Vec3::new(*x, 0.0, 0.0));
            let circle = Profile::circle(plane, Point2::new(*offset, 0.0), *radius);
            k.create_body_from_profile(&OpTag::new(format!("section_{x}")), &circle).expect("section").body
        })
        .collect();
    k.loft(&OpTag::new("spindle"), &sections, LoftShape::Ruled).expect("spindle").body
}

/// A ball of radius 10: one face, which the kernel meshes in one pass, so its three face
/// levels are one mesh and only the part's own level changes its points.
fn ball(k: &dyn Kernel) -> BodyId {
    let (south, north) = (Point2::new(-10.0, 0.0), Point2::new(10.0, 0.0));
    let half_disc = Profile {
        plane: Plane::ZX,
        outer: Loop(vec![
            Segment::Line { from: south, to: north },
            Segment::Arc { center: Point2::new(0.0, 0.0), from: north, to: south, clockwise: false },
        ]),
        holes: Vec::new(),
    };
    let sheet = k.create_body_from_profile(&OpTag::new("ball_sketch"), &half_disc).expect("half disc").body;
    let turn = Revolve { axis_origin: Vec3::ZERO, axis_direction: Vec3::Z, angle_deg: 360.0 };
    k.revolve(&OpTag::new("ball"), sheet, &turn).expect("ball").body
}

/// How many vertices each face of `mesh` has. A display mesh keeps the vertices of two
/// faces apart, so the counts add up to the mesh's.
fn points_by_face(mesh: &Mesh) -> BTreeMap<FaceId, usize> {
    let mut seen: BTreeMap<FaceId, BTreeSet<u32>> = BTreeMap::new();
    for (corners, face) in mesh.indices.chunks(3).zip(&mesh.triangle_faces) {
        seen.entry(*face).or_default().extend(corners);
    }
    seen.into_iter().map(|(face, vertices)| (face, vertices.len())).collect()
}

/// One face of `mesh` as it is drawn: the bits of its triangles' corners, in order.
fn face_bits(mesh: &Mesh, face: FaceId) -> Vec<u32> {
    mesh.indices
        .chunks(3)
        .zip(&mesh.triangle_faces)
        .filter(|(_, of)| **of == face)
        .flat_map(|(corners, _)| corners.iter().flat_map(|i| mesh.positions[*i as usize].map(f32::to_bits)))
        .collect()
}

/// The display data of `mesh` on top of `held`.
fn with_mesh(held: Usage, mesh: &Mesh) -> Usage {
    Usage { triangles: held.triangles + mesh.triangles() as u64, bytes: held.bytes + mesh.bytes() as u64 }
}

/// The mesh a plan was expected to give.
fn made(planned: Planned) -> FaceMesh {
    match planned {
        Planned::Mesh(mesh) => *mesh,
        other => panic!("no mesh: {other:?}"),
    }
}

/// Two recorded adapters on Linux Vulkan: an RTX 2070 Max-Q reports a buffer limit of
/// 1 TiB, and wgpu clamps a UHD 630 to i32::MAX.
fn adapters() -> (DeviceBudget, DeviceBudget) {
    (DeviceBudget::from_buffer_limit(1 << 40), DeviceBudget::from_buffer_limit(i32::MAX as u64))
}

/// Other display data held on an adapter that leaves it room for `points` more points.
fn leaving(budget: DeviceBudget, points: u64) -> Usage {
    let held = Usage { triangles: budget.triangles - 2 * points, bytes: budget.bytes - POINT_BYTES * points };
    assert_eq!(budget.points_left(held), points);
    held
}

#[test]
fn zooming_in_refines_the_faces_in_view_and_not_the_rest() {
    let k = CatallaxKernel::new();
    let body = spindle(&k);
    let topology = k.topology(body).unwrap();
    let levels = FaceLevels::new(&Quality::Auto, topology.bbox.diagonal());
    let boxes: Vec<_> = topology.faces.iter().map(|face| face.bbox).collect();
    let ids: Vec<FaceId> = topology.faces.iter().map(|face| face.id).collect();
    let counts = k.face_level_points(body, levels.params()).unwrap().expect("a solid has face levels");
    assert_eq!(counts.iter().map(|count| count.face).collect::<Vec<_>>(), ids, "counts come in topology order");
    // The four tapers have fewer points below fine. The straight walls and the two ends
    // are one mesh at every level.
    let tapers: Vec<bool> =
        counts.iter().map(|count| count.at(FaceLevel::Coarse) < count.at(FaceLevel::Fine)).collect();
    assert_eq!((tapers.iter().filter(|taper| **taper).count(), ids.len()), (4, 9));
    for (count, taper) in counts.iter().zip(&tapers) {
        let [coarse, standard, fine] = count.points;
        assert!(coarse <= standard && standard <= fine && (*taper || coarse == fine), "{count:?}");
    }
    let thresholds = Thresholds::default();

    // The whole spindle from two metres: every face is in view and coarse is sharp enough.
    let far = View::perspective(Vec3::new(0.0, 0.0, 2000.0), Vec3::ZERO, Vec3::Y, 45f64.to_radians(), 1500.0, 1000.0);
    let mut choices = thresholds.initial_faces(&levels, &boxes, &far, Level::Fine);
    assert!(boxes.iter().all(|face| far.shows_box(face)));
    assert!(choices.iter().all(|choice| choice.level == Level::Coarse));

    // Zoom in on the last taper with a 10 degree lens. It and the end beside it stay in
    // view; the other seven faces leave it.
    let near = View::perspective(
        Vec3::new(66.0, 0.0, 100.0),
        Vec3::new(66.0, 0.0, 0.0),
        Vec3::Y,
        10f64.to_radians(),
        1500.0,
        1000.0,
    );
    let in_view: Vec<bool> = boxes.iter().map(|face| near.shows_box(face)).collect();
    let at_the_end: Vec<bool> = boxes.iter().map(|face| face.min.x > 49.0).collect();
    assert_eq!(in_view, at_the_end);
    assert_eq!(in_view.iter().filter(|shown| **shown).count(), 2);
    let raised = thresholds.update_faces(&mut choices, &levels, &boxes, &near, 0.016, Level::Fine);
    assert_eq!(raised, 2, "the two faces in view went up and the seven others did not");
    for (choice, shown) in choices.iter().zip(&in_view) {
        assert_eq!(choice.level, if *shown { Level::Fine } else { Level::Coarse });
    }

    // The mesh at those levels has, face by face, the points counted before it was made:
    // the fine count on the taper in view and the coarse count on the other three.
    let wanted: BTreeMap<FaceId, Level> = ids.iter().copied().zip(choices.iter().map(|choice| choice.level)).collect();
    let mesh = made(
        DeviceBudget::UNLIMITED.mesh_faces(Usage::default(), &k, body, &levels, |face| (wanted[&face], 1.0)).unwrap(),
    );
    assert_eq!(mesh.levels, ids.iter().map(|id| (*id, wanted[id])).collect::<Vec<_>>());
    let points = points_by_face(&mesh.mesh);
    let (mut refined, mut left_coarse) = (0, 0);
    for ((count, shown), taper) in counts.iter().zip(&in_view).zip(&tapers) {
        let level = if *shown { FaceLevel::Fine } else { FaceLevel::Coarse };
        assert_eq!(points[&count.face], count.at(level) as usize, "{count:?}");
        if *taper && *shown {
            refined += 1;
            assert!(points[&count.face] > count.at(FaceLevel::Coarse) as usize);
        } else if *taper {
            left_coarse += 1;
            assert!(points[&count.face] < count.at(FaceLevel::Fine) as usize);
        }
    }
    assert_eq!((refined, left_coarse), (1, 3));
    assert_eq!(mesh.mesh.positions.len() as u64, mesh.points);
    assert_eq!(mesh.points, points.values().sum::<usize>() as u64);
}

#[test]
fn a_stronger_adapter_gets_finer_faces_and_both_stay_in_budget() {
    let k = CatallaxKernel::new();
    let body = spindle(&k);
    let levels = FaceLevels::new(&Quality::Auto, k.topology(body).unwrap().bbox.diagonal());
    let counts = k.face_level_points(body, levels.params()).unwrap().expect("a solid has face levels");
    let total = |level: FaceLevel| counts.iter().map(|count| u64::from(count.at(level))).sum::<u64>();
    let (coarse, fine) = (total(FaceLevel::Coarse), total(FaceLevel::Fine));
    assert!(coarse < fine, "{coarse} coarse points, {fine} fine");

    // Both adapters hold the same other display data. It leaves the weak one room for
    // the coarse levels and half of what the fine ones add.
    let (strong, weak) = adapters();
    let room = (coarse + fine) / 2;
    let held = leaving(weak, room);
    assert!(strong.points_left(held) > fine);

    // The camera is close on every face, so every face asks for fine on both adapters.
    let on = |budget: DeviceBudget| -> FaceMesh {
        let mesh = made(budget.mesh_faces(held, &k, body, &levels, |_| (Level::Fine, 10.0)).unwrap());
        // The total was settled on the counts, before the mesh, and the mesh has it.
        assert!(mesh.fits && mesh.points <= budget.points_left(held));
        assert_eq!(mesh.mesh.positions.len() as u64, mesh.points);
        let points = points_by_face(&mesh.mesh);
        for (count, (face, level)) in counts.iter().zip(&mesh.levels) {
            assert_eq!(count.face, *face);
            assert_eq!(points[face], count.at(FaceLevel::from(*level)) as usize, "{count:?} at {level}");
        }
        // With its triangles too, the mesh is within what the count allowed for.
        assert!(budget.fits(with_mesh(held, &mesh.mesh)), "{:?} over {budget:?}", with_mesh(held, &mesh.mesh));
        mesh
    };
    let (on_strong, on_weak) = (on(strong), on(weak));
    let at_fine = |mesh: &FaceMesh| mesh.levels.iter().filter(|(_, level)| *level == Level::Fine).count();
    assert_eq!(
        (at_fine(&on_strong), on_strong.points),
        (counts.len(), fine),
        "the strong adapter shows every face fine"
    );
    assert!(at_fine(&on_weak) < at_fine(&on_strong), "the weak adapter shows fewer faces fine");
    assert!(on_weak.levels.iter().zip(&on_strong.levels).all(|(weak, strong)| weak.1 <= strong.1));
    assert!(coarse < on_weak.points && on_weak.points <= room && room < on_strong.points);
}

#[test]
fn a_part_over_budget_is_planned_coarser_before_it_is_meshed() {
    let k = CatallaxKernel::new();
    let body = ball(&k);
    let whole = Levels::new(&Quality::Auto, k.topology(body).unwrap().bbox.diagonal());
    // The ball's face is one mesh at all three face levels, so only planning the part
    // at a coarser level brings its points down.
    let planned = |level: Level| -> u64 {
        let counts = k.face_level_points(body, whole.params(level)).unwrap().expect("a solid has face levels");
        assert!(counts.iter().all(|count| count.points[0] == count.points[2]), "{counts:?}");
        counts.iter().map(|count| u64::from(count.at(FaceLevel::Fine))).sum()
    };
    let [coarse, standard, fine] = Level::ALL.map(planned);
    assert!(coarse < standard && standard < fine, "{coarse}, {standard}, {fine}");

    // The weak adapter has room for the standard plan and not for the fine one.
    let (strong, weak) = adapters();
    let held = leaving(weak, (standard + fine) / 2);
    let close = |_: FaceId| (Level::Fine, 10.0);
    // The fine plan is counted, found over, and no mesh is made of it.
    let over = weak.mesh_faces(held, &k, body, &whole.faces(), close).unwrap();
    assert!(matches!(&over, Planned::Over(granted) if !granted.fits && granted.points == fine), "{over:?}");

    let on = |budget: DeviceBudget| {
        let (level, mesh) = budget.mesh_part(held, &k, body, &whole, Level::Fine, close).unwrap().expect("face levels");
        assert!(mesh.fits && mesh.points <= budget.points_left(held));
        assert_eq!(mesh.mesh.positions.len() as u64, mesh.points);
        assert!(budget.fits(with_mesh(held, &mesh.mesh)));
        // The part's mesh at the level it was planned at, byte for byte.
        let direct = k.tessellate(body, whole.params(level)).unwrap();
        assert_eq!(serde_json::to_vec(&mesh.mesh).unwrap(), serde_json::to_vec(&direct).unwrap());
        (level, mesh.points)
    };
    assert_eq!(on(strong), (Level::Fine, fine));
    assert_eq!(on(weak), (Level::Standard, standard));

    // With no room at all the part is still drawn, at its coarse level, and says it is over.
    let none = DeviceBudget { triangles: 0, bytes: 0 };
    let (level, mesh) = none.mesh_part(Usage::default(), &k, body, &whole, Level::Fine, close).unwrap().unwrap();
    assert_eq!((level, mesh.points, mesh.fits), (Level::Coarse, coarse, false));
}

#[test]
fn mesh_bytes_per_level_do_not_change() {
    let k = CatallaxKernel::new();
    let body = spindle(&k);
    let whole = Levels::new(&Quality::Auto, k.topology(body).unwrap().bbox.diagonal());
    let levels = whole.faces();
    let bytes = |mesh: &Mesh| serde_json::to_vec(mesh).unwrap();
    // The three whole-part levels as the screen and the exports get them today.
    let whole_part = || -> Vec<Vec<u8>> {
        let calls = [
            k.preview_levels(body, whole.all()).unwrap(),
            k.checked_rendering_levels(body, whole.all()).unwrap(),
            k.tessellate_levels(body, whole.all()).unwrap(),
        ];
        calls.iter().flatten().map(bytes).collect()
    };
    let before = whole_part();

    let at = |planned: &FaceLevels, choose: &dyn Fn(usize) -> FaceLevel| -> Mesh {
        k.face_level_mesh(body, planned.params(), &mut |counts: &[FaceLevelPoints]| {
            Some((0..counts.len()).map(choose).collect())
        })
        .unwrap()
        .expect("a solid has face levels")
    };
    // Every face fine is the part's mesh at the level it was planned at, byte for byte.
    for level in Level::ALL {
        let direct = k.tessellate(body, whole.params(level)).unwrap();
        assert_eq!(bytes(&at(&whole.faces_at(level), &|_| FaceLevel::Fine)), bytes(&direct), "{level}");
    }
    // A face at a level is the same triangles whatever level its neighbours are at.
    let uniform = FaceLevel::ALL.map(|level| at(&levels, &|_| level));
    let mixed = at(&levels, &|i| FaceLevel::ALL[i % 3]);
    let counts = k.face_level_points(body, levels.params()).unwrap().expect("a solid has face levels");
    for (i, count) in counts.iter().enumerate() {
        let level = FaceLevel::ALL[i % 3];
        assert!(!face_bits(&mixed, count.face).is_empty());
        assert_eq!(face_bits(&mixed, count.face), face_bits(&uniform[level.index()], count.face), "{count:?}");
    }
    // And whatever budget picked the levels: the same choice is the same bytes.
    let coarsest: u64 = counts.iter().map(|count| u64::from(count.at(FaceLevel::Coarse))).sum();
    let tight = DeviceBudget { triangles: 2 * coarsest, bytes: POINT_BYTES * coarsest };
    let granted = made(tight.mesh_faces(Usage::default(), &k, body, &levels, |_| (Level::Fine, 10.0)).unwrap());
    assert!(granted.points == coarsest && granted.levels.iter().any(|(_, level)| *level < Level::Fine));
    assert_eq!(bytes(&granted.mesh), bytes(&at(&levels, &|i| FaceLevel::from(granted.levels[i].1))));

    // Planning and choosing face levels left the whole-part levels as they were.
    assert_eq!(whole_part(), before);
}

#[test]
fn the_report_gives_each_levels_deviation_and_triangles() {
    let k = CatallaxKernel::new();
    let body = curved_body(&k);
    let r = report(&k, body, &Quality::Auto).unwrap();
    assert!((34.6..34.8).contains(&r.diagonal_mm), "{}", r.diagonal_mm);
    let deviations: Vec<f64> = r.levels.iter().map(|l| l.deviation_mm.unwrap()).collect();
    assert!((deviations[1] - 0.002 * r.diagonal_mm).abs() < 1e-12);
    assert_eq!(deviations[0], deviations[1] * 4.0);
    assert_eq!(deviations[2], deviations[1] / 4.0);
    // Chord refinement adds triangles on the curved wall.
    let triangles: Vec<usize> = r.levels.iter().map(|l| l.triangles).collect();
    assert!(triangles.windows(2).all(|w| w[0] <= w[1]), "finer levels never have fewer triangles: {triangles:?}");
    assert!(triangles[2] > triangles[0], "{triangles:?}");
    for l in &r.levels {
        assert_eq!(l.triangles, k.tessellate(body, &l.params).unwrap().triangle_count());
    }

    // The slider moves the levels; a finer position has more triangles.
    let fine = report(&k, body, &Quality::Slider { value: 9 }).unwrap();
    assert!(fine.level(Level::Standard).triangles > r.level(Level::Standard).triangles);
    assert!(fine.level(Level::Standard).deviation_mm < r.level(Level::Standard).deviation_mm);

    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["levels"][1]["level"], "standard");
    assert_eq!(json["quality"]["mode"], "auto");
}

#[test]
fn the_worker_answers_off_the_interface_thread() {
    let k: Arc<dyn Kernel> = Arc::new(CatallaxKernel::new());
    let body = curved_body(k.as_ref());
    let levels = Levels::new(&Quality::Auto, k.topology(body).unwrap().bbox.diagonal());
    let mut t: Tessellator<&str> = Tessellator::new(k.clone());
    let generation = t.request("block", body, levels.all());
    assert_eq!(t.pending(), 1);
    let done = t.wait(WAIT).expect("an answer");
    assert_eq!((done.key, done.generation), ("block", generation));
    let meshes = done.result.unwrap();
    // Compare bytes, including the sign bit of zero in positions and normals.
    let direct = k.preview_levels(body, levels.all()).unwrap();
    assert_eq!(serde_json::to_vec(&meshes).unwrap(), serde_json::to_vec(&direct).unwrap());
    assert_eq!(t.pending(), 0);
    assert!(t.poll().is_empty());
}

#[test]
fn a_new_request_supersedes_the_old_one() {
    let k: Arc<dyn Kernel> = Arc::new(CatallaxKernel::new());
    let body = curved_body(k.as_ref());
    let coarse = Levels::new(&Quality::Slider { value: 1 }, 50.0);
    let fine = Levels::new(&Quality::Slider { value: 10 }, 50.0);
    let mut t: Tessellator<u32> = Tessellator::new(k.clone());
    for _ in 0..5 {
        t.request(7, body, coarse.all());
    }
    let last = t.request(7, body, fine.all());
    let done = t.wait(WAIT).expect("an answer");
    assert_eq!(done.generation, last, "only the latest request is answered");
    assert_eq!(done.result.unwrap()[0], k.preview(body, fine.params(Level::Coarse)).unwrap());
    std::thread::sleep(Duration::from_millis(50));
    assert!(t.poll().is_empty());

    // A cancelled request is never answered.
    t.request(8, body, fine.all());
    t.cancel(&8);
    assert_eq!(t.pending(), 0);
    assert!(t.wait(Duration::from_millis(500)).is_none());

    // A body released before its turn answers with an error, not a panic.
    let gone = contract::cylinder(k.as_ref(), Vec3::new(0.0, 0.0, 0.0), 4.0, 4.0);
    k.release(gone).unwrap();
    t.request(9, gone, fine.all());
    let done = t.wait(WAIT).expect("an answer");
    assert!(matches!(done.result, Err(KernelError::UnknownBody(_))));
}

#[test]
fn many_placements_share_one_definitions_meshes() {
    let k: Arc<dyn Kernel> = Arc::new(CatallaxKernel::new());
    let bolt = contract::cylinder(k.as_ref(), Vec3::new(0.0, 0.0, 0.0), 3.0, 20.0);
    let levels = Levels::new(&Quality::Auto, k.topology(bolt).unwrap().bbox.diagonal());
    let mut t: Tessellator<&str> = Tessellator::new(k.clone());
    let mut cache: DisplayCache<&str, Mesh> = DisplayCache::new(DeviceBudget::UNLIMITED);
    // Ten thousand placements ask for the definition; one request goes out.
    for _ in 0..10_000 {
        if cache.missing(&"bolt", &levels).len() == 3 && t.pending() == 0 {
            t.request("bolt", bolt, levels.all());
        }
    }
    let done = t.wait(WAIT).expect("an answer");
    cache.insert_levels(done.key, &levels, done.result.unwrap());
    assert!(cache.missing(&"bolt", &levels).is_empty());
    let per_placement: Vec<_> = (0..10_000).map(|_| cache.best(&"bolt", &levels, Level::Standard).unwrap().1).collect();
    assert!(per_placement.windows(2).all(|w| Arc::ptr_eq(&w[0], &w[1])), "every placement draws the same mesh");
    let standard = cache.get(&"bolt", levels.key(Level::Standard)).unwrap();
    let coarse = cache.get(&"bolt", levels.key(Level::Coarse)).unwrap();
    assert!(Arc::ptr_eq(&standard, &coarse) || standard.triangle_count() > coarse.triangle_count());
    assert!(cache.mesh_count() <= 3);
    assert!(
        cache.usage().triangles <= 3 * cache.get(&"bolt", levels.key(Level::Fine)).unwrap().triangle_count() as u64
    );
}
