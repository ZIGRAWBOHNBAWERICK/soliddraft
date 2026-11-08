//! The scene, colours, picking and the hierarchy, on an assembly made of display copies
//! alone (no kernel).

use glam::DVec3;
use soliddraft_assembly::bvh::Ray;
use soliddraft_assembly::{
    Aabb, Assembly, AssemblyDocument, AssemblyPackage, DisplayMesh, Instance, OccurrencePath, OpenOptions, Placement,
    RefCopy, Scene, Step,
};
use soliddraft_document::{ContentHash, DocKind, DocRef, DocumentId};
use soliddraft_materials::{Appearance, Srgb};

/// A root with a grid of 4 x 5 bricks (components 3 to 22, sources 1 and 2) and two
/// copies of a sub-assembly of two plates (components 23 and 24; inside, 2 and 3).
fn sample() -> AssemblyDocument {
    let mut doc = sample_loading();
    doc.finish_loading();
    doc
}

/// The sample, its loading still under way.
fn sample_loading() -> AssemblyDocument {
    let brick = (DocumentId::new(), ContentHash::of_bytes(b"brick"));
    let plate = (DocumentId::new(), ContentHash::of_bytes(b"plate"));
    let mut sub = Assembly::new("Sub");
    let s = sub.add_source(DocRef::new(plate.0, DocKind::Part, "plate.dwprt", plate.1.clone()));
    sub.add_component(s, Placement::IDENTITY);
    sub.add_component(s, Placement::translation(DVec3::new(0.0, 0.0, 5.0)));
    let sub_id = DocumentId::new();
    let sub_rev = sub.semantic_hash();

    let mut root = Assembly::new("Root");
    let b = root.add_source(DocRef::new(brick.0, DocKind::Part, "brick.dwprt", brick.1.clone()));
    let a = root.add_source(DocRef::new(sub_id, DocKind::Assembly, "sub.dwasm", sub_rev.clone()));
    for i in 0..4 {
        for j in 0..5 {
            root.add_component(b, Placement::translation(DVec3::new(i as f64 * 20.0, j as f64 * 20.0, 0.0)));
        }
    }
    root.add_component(a, Placement::translation(DVec3::new(200.0, 0.0, 0.0)));
    root.add_component(a, Placement::translation(DVec3::new(300.0, 0.0, 0.0)));
    let copies = [
        RefCopy::part(brick.0, brick.1, &DisplayMesh::cuboid(DVec3::ZERO, DVec3::new(10.0, 10.0, 10.0)), "test"),
        RefCopy::part(plate.0, plate.1, &DisplayMesh::cuboid(DVec3::ZERO, DVec3::new(50.0, 50.0, 2.0)), "test"),
        RefCopy::assembly(sub_id, sub_rev, &sub.truth_entries()),
    ];
    let bytes = root.to_bytes(DocumentId::new(), None, &copies).unwrap();
    let package = AssemblyPackage::from_bytes(bytes).unwrap();
    AssemblyDocument::open_package(package, None, OpenOptions::default()).unwrap()
}

#[test]
fn the_scene_groups_instances_by_definition_and_follows_edits() {
    let mut doc = sample();
    assert_eq!(doc.store().counts()[5], 0, "nothing failed");
    let mut scene = Scene::new(&mut doc);
    assert_eq!(scene.instances().len(), 24, "20 bricks and 4 plates");
    let draws = scene.draws(|_| true);
    assert_eq!(draws.iter().map(|d| d.count).collect::<Vec<_>>(), vec![20, 4]);
    assert_eq!(scene.take_dirty(), std::iter::once(0..24).collect::<Vec<_>>(), "everything after a rebuild");
    assert_eq!(scene.take_new_meshes(usize::MAX).len(), 2);

    // Hide one brick: only its instance changes.
    let first: OccurrencePath = "3".parse().unwrap();
    doc.set_hidden(std::slice::from_ref(&first), true).unwrap();
    scene.update(&mut doc);
    let slot = scene.slot(doc.table().find(&first).unwrap()).unwrap();
    assert_ne!(scene.instances()[slot as usize].flags & Instance::HIDDEN, 0);
    assert_eq!(scene.take_dirty(), vec![slot..slot + 1]);

    // Colour one copy of the sub-assembly: both its plates, and only its plates.
    let red = Appearance::new("red", "Red", Srgb::new(200, 20, 20));
    let right: OccurrencePath = "24".parse().unwrap();
    doc.set_colour(std::slice::from_ref(&right), Some(red.clone())).unwrap();
    scene.update(&mut doc);
    let coloured: Vec<&Instance> = scene.instances().iter().filter(|i| i.flags & Instance::COLOURED != 0).collect();
    assert_eq!(coloured.len(), 2);
    assert_eq!(coloured[0].colour.to_le_bytes()[..3], [200, 20, 20]);
    // A part below with a colour of its own keeps it when the outer one changes.
    let inner: OccurrencePath = "24/3".parse().unwrap();
    let blue = Appearance::new("blue", "Blue", Srgb::new(20, 20, 200));
    doc.set_colour(std::slice::from_ref(&inner), Some(blue)).unwrap();
    doc.set_colour(std::slice::from_ref(&right), Some(Appearance::new("green", "Green", Srgb::new(20, 200, 20))))
        .unwrap();
    let t = doc.table();
    assert_eq!(t.colour(t.find(&inner).unwrap()).unwrap().id.as_str(), "blue");
    assert_eq!(t.colour(t.find(&"24/2".parse().unwrap()).unwrap()).unwrap().id.as_str(), "green");
    assert!(doc.undo() && doc.undo());
    let t = doc.table();
    assert_eq!(t.colour(t.find(&inner).unwrap()).unwrap().id.as_str(), "red");
    assert!(doc.undo());
    assert!(doc.table().colour(doc.table().find(&inner).unwrap()).is_none());
    assert!(doc.assembly().appearances.is_empty() && doc.assembly().snapshots.is_empty());

    // Selecting the sub-assembly marks its parts.
    let row = doc.table().find(&right).unwrap();
    scene.set_selection(doc.table(), &[row]);
    let selected = scene.instances().iter().filter(|i| i.flags & Instance::SELECTED != 0).count();
    assert_eq!(selected, 2);
}

#[test]
fn colours_survive_saving_and_do_not_change_the_geometry() {
    let mut doc = sample();
    let before = doc.assembly().geometry_hash();
    let p: OccurrencePath = "23".parse().unwrap();
    doc.set_colour(std::slice::from_ref(&p), Some(Appearance::new("red", "Red", Srgb::new(200, 20, 20)))).unwrap();
    assert_eq!(doc.assembly().geometry_hash(), before);
    let dir = std::env::temp_dir().join(format!("soliddraft-scene-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = doc.save(Some(&dir.join("coloured.dwasm"))).unwrap();
    let mut back = AssemblyDocument::open(&path, OpenOptions::default()).unwrap();
    back.finish_loading();
    let t = back.table();
    assert_eq!(t.colour(t.find(&"23/2".parse().unwrap()).unwrap()).unwrap().id.as_str(), "red");
    assert!(t.colour(t.find(&"24/2".parse().unwrap()).unwrap()).is_none());
}

#[test]
fn picks_name_occurrences_and_faces() {
    let mut doc = sample();
    let scene = Scene::new(&mut doc);
    let row = doc.table().find(&"7".parse().unwrap()).unwrap();
    assert!(scene.slot(row).is_some());
    let picked = scene.decode([row.0 + 1, (1 << 30) | 6]).unwrap();
    assert_eq!(picked.occurrence, row);
    let sel = doc.selection(&picked);
    assert_eq!(sel.path.to_string(), "7");
    assert_eq!(sel.face.as_deref(), Some("+z"), "face 5 of the cuboid is its top");
    assert!(scene.decode([0, 0]).is_none(), "nothing there");
}

#[test]
fn the_hierarchy_answers_rays_and_regions_and_follows_moves() {
    let mut doc = sample();
    let target: OccurrencePath = "6".parse().unwrap(); // the fourth brick: (0, 60)
    let ray = Ray { origin: DVec3::new(5.0, 65.0, 100.0), direction: DVec3::NEG_Z };
    let table = doc.table().clone();
    let hits = doc.bvh().along_ray(&table, &ray);
    let row = doc.table().find(&target).unwrap();
    assert_eq!(hits.first().map(|h| h.0), Some(row));
    assert!((hits[0].1 - 90.0).abs() < 1e-3);
    let region = Aabb { min: DVec3::new(-1.0, -1.0, -1.0), max: DVec3::new(31.0, 31.0, 11.0) };
    assert_eq!(doc.bvh().in_box(&table, &region).len(), 4);

    doc.move_to(&[(target.clone(), Placement::translation(DVec3::new(500.0, 500.0, 0.0)))]).unwrap();
    let table = doc.table().clone();
    assert!(doc.bvh().along_ray(&table, &ray).iter().all(|h| h.0 != row), "moved away");
    let there = Ray { origin: DVec3::new(505.0, 505.0, 100.0), direction: DVec3::NEG_Z };
    assert_eq!(doc.bvh().along_ray(&table, &there).first().map(|h| h.0), Some(row));
}

/// An empty root, read from a package as the sample is.
fn empty() -> AssemblyDocument {
    read(&Assembly::new("Root"), &[])
}

/// A root placing sub-assembly S (component 2) whose own file colours its plate 3 red,
/// and a brick beside it; with the display copies that read it again.
fn coloured_sub() -> (AssemblyDocument, Vec<RefCopy>) {
    let plate = (DocumentId::new(), ContentHash::of_bytes(b"plate"));
    let mut sub = Assembly::new("Sub");
    let s = sub.add_source(DocRef::new(plate.0, DocKind::Part, "plate.dwprt", plate.1.clone()));
    sub.add_component(s, Placement::IDENTITY);
    let three = sub.add_component(s, Placement::translation(DVec3::new(0.0, 0.0, 5.0)));
    sub.set_colour(&OccurrencePath::root().child(Step::component(three)), Some(red()));
    let sub_id = DocumentId::new();
    let sub_rev = sub.semantic_hash();
    let mut root = Assembly::new("Root");
    let a = root.add_source(DocRef::new(sub_id, DocKind::Assembly, "sub.dwasm", sub_rev.clone()));
    root.add_component(a, Placement::IDENTITY);
    let copies = vec![
        RefCopy::part(plate.0, plate.1, &DisplayMesh::cuboid(DVec3::ZERO, DVec3::new(50.0, 50.0, 2.0)), "test"),
        RefCopy::assembly(sub_id, sub_rev, &sub.truth_entries()),
    ];
    (read(&root, &copies), copies)
}

fn read(root: &Assembly, copies: &[RefCopy]) -> AssemblyDocument {
    let bytes = root.to_bytes(DocumentId::new(), None, copies).unwrap();
    let package = AssemblyPackage::from_bytes(bytes).unwrap();
    let mut doc = AssemblyDocument::open_package(package, None, OpenOptions::default()).unwrap();
    doc.finish_loading();
    doc
}

fn marked(scene: &Scene, flag: u32) -> Vec<u32> {
    scene.instances().iter().filter(|i| i.flags & flag != 0).map(|i| i.row).collect()
}

/// A part is inserted, shown, selected and hovered, and the insert undone: the table
/// shrinks under the scene's rows. The next update forgets what is gone instead of
/// reading old rows in the new table, and Redo and fresh selections work again.
#[test]
fn undoing_an_insert_the_scene_selects_forgets_it() {
    for (name, mut doc) in [("empty", empty()), ("sample", sample())] {
        let mut scene = Scene::new(&mut doc);
        let brick = DocRef::new(DocumentId::new(), DocKind::Part, "new.dwprt", ContentHash::of_bytes(b"new"));
        let id = doc.insert(brick, Placement::translation(DVec3::new(0.0, -50.0, 0.0))).unwrap();
        doc.finish_loading();
        scene.update(&mut doc);
        let path = OccurrencePath::root().child(Step::component(id));
        let row = doc.table().find(&path).unwrap();
        scene.set_selection(doc.table(), &[row]);
        scene.set_hover(doc.table(), Some(row));

        assert!(doc.undo(), "{name}");
        scene.update(&mut doc);
        assert!(marked(&scene, Instance::SELECTED | Instance::HOVERED).is_empty(), "{name}: nothing is marked");
        scene.set_selection(doc.table(), &[]);
        scene.set_hover(doc.table(), None);

        assert!(doc.redo(), "{name}");
        doc.finish_loading();
        scene.update(&mut doc);
        let row = doc.table().find(&path).unwrap();
        scene.set_selection(doc.table(), &[row]);
        scene.set_hover(doc.table(), Some(row));
        assert_eq!(marked(&scene, Instance::SELECTED), [row.0], "{name}");
        assert_eq!(marked(&scene, Instance::HOVERED), [row.0], "{name}");
    }
}

/// Rereading the structure renumbers rows: what is selected and hovered stays on the
/// same occurrence, found again by its path.
#[test]
fn a_reread_keeps_the_selection_on_the_same_occurrence() {
    let mut doc = sample();
    let mut scene = Scene::new(&mut doc);
    let plate: OccurrencePath = "24/3".parse().unwrap();
    let brick: OccurrencePath = "22".parse().unwrap();
    let (p, b) = (doc.table().find(&plate).unwrap(), doc.table().find(&brick).unwrap());
    scene.set_selection(doc.table(), &[p]);
    scene.set_hover(doc.table(), Some(b));
    // Deleting the first brick leaves its row; changing the quality rereads the table
    // without it, so every later row moves down one.
    doc.delete(&[soliddraft_assembly::ComponentId(3)]).unwrap();
    scene.update(&mut doc);
    doc.set_viewport_quality(soliddraft_tessellation::Quality::Slider { value: 10 }).unwrap();
    doc.finish_loading();
    scene.update(&mut doc);
    let (p2, b2) = (doc.table().find(&plate).unwrap(), doc.table().find(&brick).unwrap());
    assert_eq!((p2.0, b2.0), (p.0 - 1, b.0 - 1), "the rows moved");
    assert_eq!(marked(&scene, Instance::SELECTED), [p2.0]);
    assert_eq!(marked(&scene, Instance::HOVERED), [b2.0]);
}

/// Whether the hierarchy finds brick 6 by a ray down through it and by a small box
/// around it, against the table as it is now.
fn finds_brick_6(doc: &mut AssemblyDocument) -> (bool, bool) {
    let row = doc.table().find(&"6".parse().unwrap());
    let ray = Ray { origin: DVec3::new(5.0, 65.0, 100.0), direction: DVec3::NEG_Z };
    let region = Aabb { min: DVec3::new(4.0, 64.0, 4.0), max: DVec3::new(6.0, 66.0, 6.0) };
    let table = doc.table().clone();
    let bvh = doc.bvh();
    let by_ray = bvh.along_ray(&table, &ray).iter().any(|h| Some(h.0) == row);
    let by_box = bvh.in_box(&table, &region).iter().any(|r| Some(*r) == row);
    (by_ray, by_box)
}

/// The hierarchy is built while brick 6 is suppressed. Showing the brick again, by
/// Unsuppress or by Undo, makes rays and boxes find it; suppressing or deleting it
/// again keeps them from returning it, though the hierarchy is not rebuilt.
#[test]
fn the_hierarchy_follows_suppression_and_deletion() {
    let mut doc = sample();
    let brick: OccurrencePath = "6".parse().unwrap();
    assert_eq!(finds_brick_6(&mut doc), (true, true));
    doc.set_suppressed(std::slice::from_ref(&brick), true).unwrap();
    let mut fresh = sample();
    fresh.set_suppressed(std::slice::from_ref(&brick), true).unwrap();
    for (name, doc) in [("built before", &mut doc), ("built while suppressed", &mut fresh)] {
        assert_eq!(finds_brick_6(doc), (false, false), "{name}: suppressed");
        doc.set_suppressed(std::slice::from_ref(&brick), false).unwrap();
        assert_eq!(finds_brick_6(doc), (true, true), "{name}: unsuppressed");
        assert!(doc.undo());
        assert_eq!(finds_brick_6(doc), (false, false), "{name}: unsuppress undone");
        assert!(doc.undo());
        assert_eq!(finds_brick_6(doc), (true, true), "{name}: suppress undone");
        assert!(doc.redo());
        assert_eq!(finds_brick_6(doc), (false, false), "{name}: suppress redone");
        assert!(doc.undo());
        doc.delete(&[soliddraft_assembly::ComponentId(6)]).unwrap();
        assert_eq!(finds_brick_6(doc), (false, false), "{name}: deleted");
    }

    // The background build starts when loading ends, here with the brick suppressed,
    // and is taken after the brick is shown again: it finds the brick too.
    let mut late = sample_loading();
    late.set_suppressed(std::slice::from_ref(&brick), true).unwrap();
    while late.poll(std::time::Duration::from_millis(10)) {}
    late.set_suppressed(std::slice::from_ref(&brick), false).unwrap();
    let started = std::time::Instant::now();
    while late.bvh_ready().is_none() && started.elapsed() < std::time::Duration::from_secs(10) {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let row = late.table().find(&brick).unwrap();
    let ray = Ray { origin: DVec3::new(5.0, 65.0, 100.0), direction: DVec3::NEG_Z };
    let table = late.table().clone();
    let ready = late.bvh_ready().expect("the background build is done");
    assert!(ready.along_ray(&table, &ray).iter().any(|h| h.0 == row), "the ready hierarchy finds it");
}

fn red() -> Appearance {
    Appearance::new("red", "Red", Srgb::new(200, 20, 20))
}

/// The colour every occurrence shows, and whether it is its own, by path.
fn colours(doc: &AssemblyDocument) -> Vec<(String, Option<String>, bool)> {
    let t = doc.table();
    (1..t.len() as u32)
        .map(soliddraft_assembly::OccurrenceIndex)
        .map(|r| {
            let own = t.flags(r).has(soliddraft_assembly::Flags::COLOURED_SELF);
            (t.path(r).to_string(), t.colour(r).map(|a| a.id.0.clone()), own)
        })
        .collect()
}

/// The root colours S's red plate blue, then takes that away, by Remove colour or Undo:
/// the plate shows S's own red again, as the same truth read afresh does. A colour on S
/// itself then leaves the red plate alone, and the scene shows what the table holds.
#[test]
fn taking_away_an_outer_colour_shows_the_inner_one_again() {
    let (mut doc, copies) = coloured_sub();
    let mut scene = Scene::new(&mut doc);
    let plate: OccurrencePath = "2/3".parse().unwrap();
    let s: OccurrencePath = "2".parse().unwrap();
    let fresh = |doc: &AssemblyDocument| colours(&read(doc.assembly(), &copies));
    let plate_colour =
        |doc: &AssemblyDocument| doc.table().colour(doc.table().find(&plate).unwrap()).map(|a| a.id.0.clone());
    assert_eq!(plate_colour(&doc).as_deref(), Some("red"), "S's own colour");
    let start = colours(&doc);

    let blue = Appearance::new("blue", "Blue", Srgb::new(20, 20, 200));
    doc.set_colour(std::slice::from_ref(&plate), Some(blue)).unwrap();
    scene.update(&mut doc);
    assert_eq!(plate_colour(&doc).as_deref(), Some("blue"), "the outer assembly wins");
    assert_eq!(colours(&doc), fresh(&doc));
    assert!(doc.undo());
    scene.update(&mut doc);
    assert_eq!(colours(&doc), start, "undo shows red again");
    assert_eq!(colours(&doc), fresh(&doc));
    assert!(doc.redo());
    scene.update(&mut doc);
    assert_eq!(plate_colour(&doc).as_deref(), Some("blue"));
    doc.set_colour(std::slice::from_ref(&plate), None).unwrap();
    scene.update(&mut doc);
    assert_eq!(colours(&doc), start, "Remove colour shows red again");
    // Removing a colour the root never set changes nothing.
    doc.set_colour(std::slice::from_ref(&plate), None).unwrap();
    assert_eq!(colours(&doc), start);

    // A colour on S reaches its uncoloured plate, not the red one, now or after a recolour.
    let green = Appearance::new("green", "Green", Srgb::new(20, 200, 20));
    doc.set_colour(std::slice::from_ref(&s), Some(green)).unwrap();
    doc.set_colour(std::slice::from_ref(&s), Some(Appearance::new("grey", "Grey", Srgb::new(90, 90, 90)))).unwrap();
    scene.update(&mut doc);
    assert_eq!(plate_colour(&doc).as_deref(), Some("red"));
    assert_eq!(colours(&doc), fresh(&doc));
    let slot = scene.slot(doc.table().find(&plate).unwrap()).unwrap();
    assert_eq!(scene.instances()[slot as usize].colour.to_le_bytes()[..3], [200, 20, 20], "the scene shows red");
}

/// The root colours brick 3's first pattern copy, the sub-assembly copy 23 and a plate
/// inside it, and copy 24 with the same red. Deleting 3 and 23 takes their colours along,
/// so the document still saves; Undo brings the colours back and Redo drops them again.
/// 24 keeps its red throughout.
#[test]
fn deleting_a_coloured_component_takes_its_colours_along() {
    let mut doc = sample();
    let pattern = doc
        .add_pattern(
            vec![soliddraft_assembly::ComponentId(3)],
            soliddraft_assembly::PatternRule::Linear {
                direction: [0.0, 0.0, 1.0],
                spacing: 30.0,
                count: 2,
                second: None,
            },
        )
        .unwrap();
    doc.finish_loading();
    let paths: Vec<OccurrencePath> = [format!("3:{pattern}.1"), "23".into(), "23/2".into(), "24".into()]
        .iter()
        .map(|p| p.parse().unwrap())
        .collect();
    for p in &paths {
        doc.set_colour(std::slice::from_ref(p), Some(red())).unwrap();
    }
    let saves = |doc: &AssemblyDocument| {
        doc.assembly().to_bytes(DocumentId::new(), None, &[]).map(|_| ()).map_err(|e| e.to_string())
    };
    assert_eq!(saves(&doc), Ok(()));
    let assigned =
        |doc: &AssemblyDocument| doc.assembly().appearances.iter().map(|c| c.path.to_string()).collect::<Vec<_>>();
    let before = assigned(&doc);

    doc.delete(&[soliddraft_assembly::ComponentId(3), soliddraft_assembly::ComponentId(23)]).unwrap();
    assert_eq!(saves(&doc), Ok(()), "the deleted components' colours went with them");
    assert_eq!(assigned(&doc), ["24"]);
    let row = doc.table().find(&paths[3]).unwrap();
    assert_eq!(doc.table().colour(row).map(|a| a.id.0.as_str()), Some("red"), "24 keeps the shared red");
    assert!(doc.undo());
    doc.finish_loading();
    assert_eq!(assigned(&doc), before, "undo brings every colour back");
    assert_eq!(saves(&doc), Ok(()));
    for p in &paths {
        let row = doc.table().find(p).unwrap();
        assert_eq!(doc.table().colour(row).map(|a| a.id.0.as_str()), Some("red"), "{p}");
    }
    assert!(doc.redo());
    doc.finish_loading();
    assert_eq!(assigned(&doc), ["24"]);
    assert_eq!(saves(&doc), Ok(()));
}
