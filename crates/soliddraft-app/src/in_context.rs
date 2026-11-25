//! Editing a part in context: a component of the open assembly opened for editing in
//! place, as SolidWorks' Edit Part does.
//!
//! The part's own tab shows the part where the assembly places it, with the rest of the
//! assembly drawn dimmed around it. The dimmed components are never the part's own
//! geometry and are never edited; a click on one picks its face as a face of another
//! component, and a sketch started then lies on that face's plane as an in-context
//! reference ([`soliddraft_model::ContextFace`]). The tree shows the part's features
//! while the editing lasts, and the assembly's again once it ends.
//!
//! The references follow the assembly's design: where it places each component, never
//! the exploded view or a drag being previewed. Whenever the assembly reaches a revision
//! the open part has not been read against, the references are read afresh and, where a
//! face moved, updated as one undoable step; a face that cannot be found any more marks
//! its reference broken and leaves its plane as it was. Reading against the same
//! revision again is not done, so an undone update stays undone. Saving reads them once
//! more before the part is written. A part opened on its own keeps the planes it last
//! read.

use std::hash::{Hash, Hasher};
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use egui::{Align2, Stroke, pos2};
use glam::DVec3;
use soliddraft_assembly::{Flags, KernelSource, OccurrenceIndex, OccurrencePath, Placement};
use soliddraft_kernel_api::{Kernel, Plane, Vec3};
use soliddraft_model::ContextFace;
use soliddraft_tessellation::mesh::{DisplayMesh, MeshFace};

use crate::assembly::{AssemblyAction, AssemblySession};
use crate::document::{Command, SketchPlane};
use crate::state::{AppAction, AppState, NoteKind};
use crate::theme;

/// A component of the open assembly opened for editing in place.
pub struct InContext {
    /// The component, by path.
    pub path: OccurrencePath,
    /// Its name in the assembly tree.
    pub name: String,
    /// The assembly's name.
    pub assembly: String,
    /// The other components in the part's frame, drawn dimmed.
    context: Option<ContextMesh>,
    /// What the dimmed mesh was made from (see [`context_key`]).
    built: Option<u64>,
    /// A face of another component the last click picked.
    pub picked: Option<ContextPick>,
}

/// The other components as one mesh, with the component each of its faces belongs to,
/// and the same mesh as the part's renderer draws it.
struct ContextMesh {
    mesh: Arc<DisplayMesh>,
    owners: Vec<(OccurrencePath, String)>,
    drawn: Arc<crate::document::DisplayMesh>,
}

impl ContextMesh {
    fn new(mesh: DisplayMesh, owners: Vec<(OccurrencePath, String)>) -> ContextMesh {
        let n = mesh.positions.len();
        let point = |p: &[f32; 3]| glam::Vec3::from_array(*p);
        let drawn = crate::document::DisplayMesh {
            positions: mesh.positions.iter().map(point).collect(),
            normals: mesh.normals.iter().map(point).collect(),
            // Pick id 0 is no entity: the dimmed faces are never hovered or selected as
            // the part's.
            face_picks: vec![0; n],
            face_features: vec![0; n],
            face_appearances: vec![0; n],
            appearances: Vec::new(),
            indices: mesh.indices.clone(),
            edges: mesh
                .edges
                .iter()
                .map(|line| crate::document::DisplayEdge {
                    pick: 0,
                    segments: line.windows(2).flat_map(|w| [point(&w[0]), point(&w[1])]).collect(),
                })
                .collect(),
            corners: Vec::new(),
        };
        ContextMesh { mesh: Arc::new(mesh), owners, drawn: Arc::new(drawn) }
    }
}

/// A face of another component, picked in the part's tab.
#[derive(Clone, Debug)]
pub struct ContextPick {
    /// The component.
    pub path: OccurrencePath,
    /// Its name in the assembly tree.
    pub component: String,
    /// The face's persistent name on its part.
    pub face: String,
    /// The face's label, such as "Boss-Extrude1 · End cap".
    pub face_label: String,
    /// The face's triangles in the part's frame, for drawing it picked.
    triangles: Vec<[glam::Vec3; 3]>,
}

fn live_row(s: &AssemblySession, path: &OccurrencePath) -> Option<OccurrenceIndex> {
    let table = s.doc.table();
    table.find(path).filter(|r| !table.flags(*r).has(Flags::REMOVED))
}

/// Where the assembly's design places the occurrence at `row`: the exploded view and a
/// drag being previewed move only what is shown, never what a part is built from.
fn design(s: &AssemblySession, row: OccurrenceIndex) -> Placement {
    s.doc.accepted_worlds()[row.0 as usize]
}

/// Makes the open part show the configuration the occurrence at `path` places, as the
/// assembly shows it.
fn show_configuration(state: &mut AppState, path: &OccurrencePath) -> Result<(), String> {
    let s = state.assembly.session.as_ref().ok_or("Open an assembly first.")?;
    let row = live_row(s, path).ok_or("The component is no longer in the assembly.")?;
    let wanted = s.doc.reference(s.doc.table().definition(row)).and_then(|r| r.configuration.clone());
    let model = state.doc.model();
    if model.active_configuration() == wanted.as_ref() {
        return Ok(());
    }
    let name = match &wanted {
        None => "Default".to_owned(),
        Some(id) => model
            .named_configurations()
            .find(|(i, _)| *i == id)
            .map(|(_, n)| n.to_owned())
            .ok_or_else(|| format!("{} has no configuration {}", state.doc.name, id.0))?,
    };
    state.doc.apply(Command::ActivateConfiguration { name }).map(|_| ())
}

/// Starts editing the component at `path`, whose part is the open part, in the
/// configuration the assembly places.
pub fn enter(state: &mut AppState, path: &OccurrencePath) -> Result<(), String> {
    show_configuration(state, path)?;
    let s = state.assembly.session.as_ref().ok_or("Open an assembly first.")?;
    let row = live_row(s, path).ok_or("The component is no longer in the assembly.")?;
    let name = s.doc.label(row);
    let assembly = s.doc.assembly().name.clone();
    // The part opens seen as the assembly was, when the component is not turned.
    let world = design(s, row);
    if world.rotation.abs_diff_eq(glam::DQuat::IDENTITY, 1e-12) {
        let mut camera = s.viewport.camera.clone();
        camera.target = world.inverse().apply(camera.target.as_dvec3()).as_vec3();
        state.viewport.camera = camera;
    }
    state.assembly.in_context = Some(InContext {
        path: path.clone(),
        name: name.clone(),
        assembly: assembly.clone(),
        context: None,
        built: None,
        picked: None,
    });
    state.assembly.context_check = true;
    state
        .note(format!("Editing {name} in the context of {assembly} · the other components are dimmed"), NoteKind::Info);
    Ok(())
}

/// Ends the editing. Returns the component that was edited.
pub fn leave(state: &mut AppState) -> Option<OccurrencePath> {
    let was = state.assembly.in_context.take()?;
    crate::assembly::refresh(state);
    if state.doc.dirty {
        state.note(
            format!("Stopped editing {} · save the part to bring its edits into {}", was.name, was.assembly),
            NoteKind::Info,
        );
    } else {
        state.note(format!("Stopped editing {} in context", was.name), NoteKind::Info);
    }
    Some(was.path)
}

/// The dimmed assembly to draw with the part, while a part is edited in context.
pub fn mesh(state: &AppState) -> Option<Arc<crate::document::DisplayMesh>> {
    let ic = state.assembly.in_context.as_ref()?;
    ic.context.as_ref().map(|c| c.drawn.clone()).filter(|m| !m.indices.is_empty())
}

/// Runs once a frame after the assembly took in what changed: ends the editing when the
/// assembly became the active document or lost the component, or another part opened,
/// and remakes the dimmed mesh when what it shows changed.
pub(crate) fn frame(state: &mut AppState) {
    // The browser opens the component's part a frame or more after Edit part asked.
    if let Some(path) = state.assembly.entering.clone() {
        if state.assembly.is_active() || state.assembly.session.is_none() {
            state.assembly.entering = None;
        } else if matches!(crate::assembly::component_part(state, &path), Ok(crate::assembly::ComponentPart::Current)) {
            state.assembly.entering = None;
            if let Err(e) = enter(state, &path) {
                state.note(e, NoteKind::Error);
            }
        }
    }
    let Some(ic) = state.assembly.in_context.as_ref() else { return };
    let gone = state.assembly.session.as_ref().is_none_or(|s| live_row(s, &ic.path).is_none());
    if gone {
        state.assembly.in_context = None;
        state.note("Stopped editing in context: the component is no longer in the assembly", NoteKind::Info);
        return;
    }
    let path = ic.path.clone();
    let same_part =
        matches!(crate::assembly::component_part(state, &path), Ok(crate::assembly::ComponentPart::Current));
    if !same_part || state.assembly.is_active() {
        leave(state);
        return;
    }
    let assembly = &mut state.assembly;
    let (Some(s), Some(ic)) = (assembly.session.as_ref(), assembly.in_context.as_mut()) else { return };
    if s.doc.is_loading() {
        return;
    }
    let key = context_key(s, &ic.path);
    if ic.built != Some(key) {
        ic.context = Some(context_mesh(s, &ic.path));
        ic.built = Some(key);
        ic.picked = None;
    }
}

/// A fingerprint of what the dimmed mesh shows: the scene's structure, and each
/// occurrence's design placement, flags and display mesh. Moving, hiding, suppressing or
/// reloading a component changes it, though the scene's structure stays as it was.
fn context_key(s: &AssemblySession, own: &OccurrencePath) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.scene.lock().expect("the scene lock").generation().hash(&mut h);
    own.to_string().hash(&mut h);
    let table = s.doc.table();
    for (row, world) in s.doc.accepted_worlds().iter().enumerate() {
        let r = OccurrenceIndex(row as u32);
        let flags = table.flags(r);
        [Flags::REMOVED, Flags::HIDDEN, Flags::SUPPRESSED, Flags::ASSEMBLY].map(|f| flags.has(f)).hash(&mut h);
        for v in world.translation.to_array().into_iter().chain(world.rotation.to_array()) {
            v.to_bits().hash(&mut h);
        }
        s.doc.store().get(table.definition(r)).mesh.as_ref().map(|m| Arc::as_ptr(m) as usize).hash(&mut h);
    }
    h.finish()
}

/// Every component but `own`, shown and not suppressed, in `own`'s frame, where the
/// assembly's design places them.
fn context_mesh(s: &AssemblySession, own: &OccurrencePath) -> ContextMesh {
    let mut out = DisplayMesh::default();
    let mut owners = Vec::new();
    let table = s.doc.table();
    let Some(own_row) = live_row(s, own) else {
        return ContextMesh::new(out, owners);
    };
    let worlds = s.doc.accepted_worlds();
    let into_part = worlds[own_row.0 as usize].inverse();
    let mut stack = vec![OccurrenceIndex::ROOT];
    while let Some(r) = stack.pop() {
        for c in table.children(r) {
            let flags = table.flags(c);
            if c == own_row || flags.has(Flags::REMOVED) || flags.has(Flags::HIDDEN) || flags.has(Flags::SUPPRESSED) {
                continue;
            }
            if flags.has(Flags::ASSEMBLY) {
                stack.push(c);
                continue;
            }
            let Some(mesh) = s.doc.store().get(table.definition(c)).mesh.as_ref() else { continue };
            append(&mut out, mesh, &into_part.compose(&worlds[c.0 as usize]));
            let owner = (table.path(c), s.doc.label(c));
            owners.extend(mesh.faces.iter().map(|_| owner.clone()));
        }
    }
    ContextMesh::new(out, owners)
}

fn append(out: &mut DisplayMesh, mesh: &DisplayMesh, place: &soliddraft_assembly::Placement) {
    let base = out.positions.len() as u32;
    let first = (out.indices.len() / 3) as u32;
    let point = |p: &[f32; 3]| {
        let q = place.apply(DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64));
        [q.x as f32, q.y as f32, q.z as f32]
    };
    out.positions.extend(mesh.positions.iter().map(point));
    out.normals.extend(mesh.normals.iter().map(|n| {
        let q = place.rotate(DVec3::new(n[0] as f64, n[1] as f64, n[2] as f64));
        [q.x as f32, q.y as f32, q.z as f32]
    }));
    out.indices.extend(mesh.indices.iter().map(|i| i + base));
    out.faces.extend(mesh.faces.iter().map(|f| MeshFace {
        first: f.first + first,
        count: f.count,
        name: f.name.clone(),
        style: None,
    }));
    out.edges.extend(mesh.edges.iter().map(|e| e.iter().map(point).collect()));
}

/// The nearest triangle the ray from `origin` along `dir` meets, by its distance.
fn hit(mesh: &DisplayMesh, origin: glam::Vec3, dir: glam::Vec3) -> Option<u32> {
    let at = |i: u32| glam::Vec3::from_array(mesh.positions[i as usize]);
    let mut best: Option<(f32, u32)> = None;
    for (t, tri) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
        let (a, b, c) = (at(tri[0]), at(tri[1]), at(tri[2]));
        let (e1, e2) = (b - a, c - a);
        let p = dir.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-12 {
            continue;
        }
        let s = origin - a;
        let u = s.dot(p) / det;
        let q = s.cross(e1);
        let v = dir.dot(q) / det;
        let d = e2.dot(q) / det;
        if u < 0.0 || v < 0.0 || u + v > 1.0 || d <= 0.0 {
            continue;
        }
        if best.is_none_or(|(near, _)| d < near) {
            best = Some((d, t as u32));
        }
    }
    best.map(|(_, t)| t)
}

/// The face of another component the ray meets first, if any.
fn pick(context: &ContextMesh, origin: glam::Vec3, dir: glam::Vec3) -> Option<ContextPick> {
    let mesh = &context.mesh;
    let face = mesh.face_of_triangle(hit(mesh, origin, dir)?)?;
    let f = &mesh.faces[face];
    // Only a face with a persistent name can be followed.
    persistent_face(&f.name).ok()?;
    let (path, component) = context.owners.get(face)?.clone();
    let at = |i: u32| glam::Vec3::from_array(mesh.positions[i as usize]);
    let triangles = (f.first..f.first + f.count)
        .map(|t| {
            let i = &mesh.indices[t as usize * 3..t as usize * 3 + 3];
            [at(i[0]), at(i[1]), at(i[2])]
        })
        .collect();
    Some(ContextPick { path, component, face: f.name.clone(), face_label: face_label(&f.name), triangles })
}

/// The label a face's persistent name carries, or the name itself.
fn face_label(name: &str) -> String {
    serde_json::from_str::<soliddraft_model::FaceRef>(name)
        .ok()
        .map(|r| r.label.to_string())
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| name.to_owned())
}

/// A face's persistent name, read from the name its display mesh gives it. A name that
/// is not one is refused: matching it by its label could bind another face that took
/// the label since.
fn persistent_face(name: &str) -> Result<soliddraft_model::FaceRef, String> {
    serde_json::from_str(name).map_err(|_| "the face has no persistent name to follow".to_owned())
}

/// The unit normal and a point of the planar face named `face`, on whichever solid body
/// of `part` holds it.
fn face_in_part(part: &soliddraft_model::Document, face: &str) -> Result<(DVec3, DVec3), String> {
    let r = persistent_face(face)?;
    let mut why = "the part has no solid body".to_owned();
    for (_, body) in part.live_bodies() {
        match part.resolve_face(&r, body) {
            Ok(id) => {
                let topology = part.kernel().topology(body).map_err(|e| e.to_string())?;
                let info = topology.face(id).ok_or("the face is not on its body")?;
                let normal = info.normal.ok_or("the face is not flat")?;
                let point = DVec3::new(info.centroid.x, info.centroid.y, info.centroid.z);
                return Ok((DVec3::new(normal.x, normal.y, normal.z), point));
            }
            Err(e) => why = e.message,
        }
    }
    Err(why)
}

/// The plane of `face` of the component at `row`, in the frame of the occurrence at
/// `own`, read from the part's exact body where the assembly's design places both.
fn face_plane(
    s: &mut AssemblySession,
    kernel: &Arc<dyn Kernel>,
    own: OccurrenceIndex,
    row: OccurrenceIndex,
    face: &str,
) -> Result<Plane, String> {
    let into_own = design(s, own).inverse().compose(&design(s, row));
    let def = s.doc.table().definition(row);
    let name = s.doc.label(row);
    let part = s.doc.exact(def, &KernelSource::new(kernel.clone()))?;
    let part = part.lock().map_err(|_| format!("{name} is busy"))?;
    let (normal, point) = face_in_part(&part, face).map_err(|why| format!("{} of {name}: {why}", face_label(face)))?;
    let n = into_own.rotate(normal).normalize();
    let p = into_own.apply(point);
    Ok(soliddraft_model::plane_on_face(Vec3::new(n.x, n.y, n.z), Vec3::new(p.x, p.y, p.z)))
}

/// The sketch plane on the picked face of another component, taking the pick. `None`
/// when nothing is picked.
pub(crate) fn picked_plane(state: &mut AppState) -> Option<Result<SketchPlane, String>> {
    let kernel = state.doc.kernel().clone();
    let assembly = &mut state.assembly;
    let ic = assembly.in_context.as_mut()?;
    let pick = ic.picked.take()?;
    let own = ic.path.clone();
    let s = assembly.session.as_mut()?;
    let own_row = live_row(s, &own);
    let row = live_row(s, &pick.path);
    let (Some(own_row), Some(row)) = (own_row, row) else {
        return Some(Err(format!("{} is no longer in the assembly", pick.component)));
    };
    Some(face_plane(s, &kernel, own_row, row, &pick.face).map(|plane| {
        let context = ContextFace::new(
            (s.doc.id().to_string(), s.doc.assembly().name.clone()),
            own.to_string(),
            (pick.path.to_string(), pick.component.clone()),
            (pick.face.clone(), pick.face_label.clone()),
            plane,
        );
        SketchPlane::Context { context: Box::new(context), offset: 0.0 }
    }))
}

/// `c` read afresh from the open assembly.
fn read(s: &mut AssemblySession, kernel: &Arc<dyn Kernel>, c: &ContextFace) -> Result<ContextFace, String> {
    let own = OccurrencePath::from_str(&c.own).ok().and_then(|p| live_row(s, &p));
    let own = own.ok_or("the part is no longer placed where the reference was made")?;
    let row = OccurrencePath::from_str(&c.component).ok().and_then(|p| live_row(s, &p));
    let row = row.ok_or_else(|| format!("{} is no longer in the assembly", c.component_name))?;
    let plane = face_plane(s, kernel, own, row, &c.face)?;
    Ok(c.followed(&s.doc.assembly().name, &s.doc.label(row), plane))
}

/// Within this many millimetres two readings of a plane are the same.
const SAME_PLANE: f64 = 1e-9;

fn same(a: &ContextFace, b: &ContextFace) -> bool {
    let close = |x: &[f64; 3], y: &[f64; 3]| x.iter().zip(y).all(|(p, q)| (p - q).abs() <= SAME_PLANE);
    a.broken == b.broken
        && a.assembly_name == b.assembly_name
        && a.component_name == b.component_name
        && close(&a.origin, &b.origin)
        && close(&a.normal, &b.normal)
        && close(&a.x_dir, &b.x_dir)
}

/// What the references were last read against: the part, the assembly, and the
/// assembly's revision.
pub type Seen = (soliddraft_document::DocumentId, soliddraft_document::DocumentId, soliddraft_document::ContentHash);

fn seen(state: &AppState) -> Option<Seen> {
    let s = state.assembly.session.as_ref()?;
    Some((state.doc.model().id(), s.doc.id(), s.doc.revision()))
}

/// What reading the open part's references to the open assembly afresh found: each
/// changed feature's definition as read and as it now is, and why any broke.
struct Reading {
    edits: Vec<(crate::document::FeatureId, soliddraft_model::FeatureKind, soliddraft_model::FeatureKind)>,
    broken: Vec<String>,
}

fn reading(state: &mut AppState) -> Reading {
    let kernel = state.doc.kernel().clone();
    let mut out = Reading { edits: Vec::new(), broken: Vec::new() };
    let Some(s) = state.assembly.session.as_mut() else { return out };
    let id = s.doc.id().to_string();
    let model = state.doc.model();
    for (feature, c) in soliddraft_model::context::references(model.features()) {
        if c.assembly != id {
            continue;
        }
        let now = read(s, &kernel, c).unwrap_or_else(|why| c.broken(why));
        if same(&now, c) {
            continue;
        }
        if let Some(why) = &now.broken {
            out.broken.push(format!("{}: {why}", c.label()));
        }
        if let Some(f) = model.feature(feature)
            && let Some(kind) = soliddraft_model::context::with_reference(&f.kind, now)
        {
            out.edits.push((crate::document::app_id(feature), f.kind.clone(), kind));
        }
    }
    out
}

/// Reads the open part's references to the open assembly afresh, once for each revision
/// of the assembly, and updates those that changed as one undoable step. The update is
/// queued for this part only: another part opened before it runs refuses it.
pub(crate) fn follow(state: &mut AppState) {
    // An open sketch or form edits a draft of the part; the check waits for it to close.
    if state.sketch.is_some() || state.editor.is_some() {
        return;
    }
    state.assembly.context_check = false;
    let Some(s) = state.assembly.session.as_ref() else { return };
    if s.doc.is_loading() {
        state.assembly.context_check = true;
        return;
    }
    let key = seen(state);
    if state.assembly.context_seen == key {
        return;
    }
    state.assembly.context_seen = key;
    let Reading { edits, broken } = reading(state);
    if edits.is_empty() {
        return;
    }
    let n = edits.len();
    let part = state.doc.model().id();
    state.push(AppAction::Command(Command::FollowContext { part, edits }));
    // Said after the rebuild's own note, at the next frame.
    state.assembly.context_note = Some(if broken.is_empty() {
        (format!("Updated {n} in-context reference{}", if n == 1 { "" } else { "s" }), NoteKind::Ok)
    } else {
        (format!("In-context reference broken · {}", broken.join(" · ")), NoteKind::Error)
    });
}

/// Reads the open part's references afresh and applies what changed at once, as one
/// undoable step. Returns how many changed and why any broke.
fn follow_now(state: &mut AppState) -> Result<(usize, Vec<String>), String> {
    if let Some(s) = state.assembly.session.as_mut() {
        s.doc.finish_loading();
    }
    let Reading { edits, broken } = reading(state);
    state.assembly.context_seen = seen(state);
    let n = edits.len();
    if n > 0 {
        let part = state.doc.model().id();
        state.doc.apply(Command::FollowContext { part, edits })?;
    }
    Ok((n, broken))
}

/// "a", "a and b", "a, b and c".
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Saves the part edited in context to `path`, then each assembly from the one that holds
/// it up to the open one, each taking only the new revision of the document below it;
/// the other components keep the revisions they show. The references are read afresh
/// first, so the part is written as the assembly places it; should the assemblies'
/// update move them, the part and the assemblies are saved once more. Returns the note
/// that says what was written; an error says what was not.
pub(crate) fn save(state: &mut AppState, path: &Path, thumbnail: Option<&[u8]>) -> Result<(String, NoteKind), String> {
    let own = state.assembly.in_context.as_ref().map(|ic| ic.path.clone()).ok_or("No part is edited in context.")?;
    let (mut warnings, mut flagged, mut written) = (Vec::new(), Vec::new(), Vec::new());
    let mut rounds = 0;
    loop {
        let (moved, broken) = follow_now(state)?;
        flagged.extend(broken);
        if rounds > 0 && moved == 0 {
            break;
        }
        if rounds == 2 {
            return Err(format!(
                "Saved {} and its assemblies, but their update moved {moved} in-context reference{} again; save again",
                path.display(),
                if moved == 1 { "" } else { "s" }
            ));
        }
        let extras = crate::file::Extras { thumbnail, ..Default::default() };
        warnings.extend(crate::file::save_with(&mut state.doc, path, &extras)?);
        state.remember_revision();
        state.recent = crate::file::recent();
        written =
            crate::assembly::save_chain(state, &own).map_err(|why| format!("Saved {} · {why}", path.display()))?;
        rounds += 1;
    }
    let mut names = vec![path.display().to_string()];
    names.extend(written.iter().map(|p| p.display().to_string()));
    let mut text = format!("Saved {}", and_list(&names));
    let mut kind = NoteKind::Ok;
    for w in warnings {
        text.push_str(&format!(" · {w}"));
        kind = NoteKind::Error;
    }
    flagged.sort();
    flagged.dedup();
    if !flagged.is_empty() {
        text.push_str(&format!(" · in-context reference broken: {}", flagged.join(" · ")));
        kind = NoteKind::Error;
    }
    Ok((text, kind))
}

/// Whether a picture of the screen shows the part alone, for the recent list: not while
/// the part is edited in context, when the screen also holds the dimmed assembly and the
/// bar over the view.
pub fn screen_shows_part_alone(state: &AppState) -> bool {
    state.assembly.in_context.is_none()
}

/// The in-context reference a feature holds, for the tree.
pub fn reference_of(kind: &crate::document::FeatureKind) -> Option<&ContextFace> {
    match kind {
        crate::document::FeatureKind::Sketch(s) => match &s.plane {
            SketchPlane::Context { context, .. } => Some(context.as_ref()),
            _ => None,
        },
        _ => None,
    }
}

/// The warning a feature's broken in-context reference puts on its tree row.
pub fn broken(kind: &crate::document::FeatureKind) -> Option<String> {
    let c = reference_of(kind)?;
    let why = c.broken.as_ref()?;
    Some(format!("In-context reference to {} broken: {why}. The sketch keeps the plane it last read.", c.label()))
}

/// What the tree adds to a feature's line for its in-context reference.
pub fn tree_note(kind: &crate::document::FeatureKind) -> Option<String> {
    let c = reference_of(kind)?;
    Some(match &c.broken {
        None => format!("in context: {}", c.label()),
        Some(why) => format!("in-context reference broken: {why}"),
    })
}

/// The line above the part's tree while it is edited in context.
pub fn tree_banner(ui: &mut egui::Ui, state: &AppState) {
    let Some(ic) = state.assembly.in_context.as_ref() else { return };
    egui::Frame::new()
        .fill(theme::accent().gamma_multiply(0.14))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(8, 4))
        .outer_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(format!("Editing {} in {}", ic.name, ic.assembly)).color(theme::ink()));
        });
}

/// What a click in the part's view does to the picked face of another component:
/// `Some(true)` picks the face under the pointer, `Some(false)` drops the pick, `None`
/// keeps it. A click that met the part, or a reference such as a datum plane, selects
/// that instead, so Sketch never draws on another plane than the one highlighted.
fn click_picks<T>(clicked: Option<Option<T>>, reference: bool, sketching: bool) -> Option<bool> {
    match clicked {
        None => None,
        Some(Some(_)) => Some(false),
        Some(None) if reference => Some(false),
        Some(None) if sketching => None,
        Some(None) => Some(true),
    }
}

/// After the part's viewport: picks a face of another component on a click that met no
/// part geometry, and draws the picked face and the bar that ends the editing.
pub(crate) fn after_viewport(ui: &mut egui::Ui, state: &mut AppState, events: &crate::viewport::ViewportEvents) {
    let rect = state.viewport.rect;
    let camera = state.viewport.camera.clone();
    let sketching = state.sketch.is_some() || state.editor.is_some();
    let Some(ic) = state.assembly.in_context.as_mut() else { return };
    match click_picks(events.clicked, events.clicked_reference.is_some(), sketching) {
        Some(false) => ic.picked = None,
        Some(true) => {
            let pos = ui.input(|i| i.pointer.interact_pos());
            ic.picked = pos.zip(ic.context.as_ref()).and_then(|(pos, context)| {
                let (origin, dir) = camera.ray(pos, rect);
                pick(context, origin, dir)
            });
        }
        None => {}
    }
    let painter = ui.painter().with_clip_rect(rect);
    if let Some(p) = &ic.picked {
        let fill = theme::accent().gamma_multiply(0.45);
        for t in &p.triangles {
            let corners: Option<Vec<_>> = t.iter().map(|v| camera.project(*v, rect)).collect();
            if let Some(corners) = corners {
                painter.add(egui::Shape::convex_polygon(corners, fill, Stroke::NONE));
            }
        }
    }
    let text = match &ic.picked {
        Some(p) => format!(
            "Editing {} in {} · picked {} · {} · Sketch draws on it",
            ic.name, ic.assembly, p.component, p.face_label
        ),
        None => format!("Editing {} in {}", ic.name, ic.assembly),
    };
    let mut exit = false;
    egui::Area::new(egui::Id::new("in_context_bar"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(rect.center().x, rect.top() + 52.0))
        .pivot(Align2::CENTER_TOP)
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(theme::hud_bg())
                .stroke(Stroke::new(1.0, theme::line()))
                .corner_radius(egui::CornerRadius::same(99))
                .inner_margin(egui::Margin::symmetric(12, 4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(text).color(theme::ink()));
                        let button = ui.button("Exit in context");
                        #[cfg(target_arch = "wasm32")]
                        crate::web::show_control("Exit in context", button.rect);
                        exit = button.clicked();
                    });
                });
        });
    if exit {
        state.push(AppAction::Assembly(AssemblyAction::ExitInContext));
    }
}

/// The editing as JSON for browser checks: `null` when no part is edited in context,
/// else the `component` path and `name`, the `assembly`, how many triangles the
/// dimmed `context` has, the `picked` face, and the open part's `references`.
pub fn shown(state: &AppState) -> serde_json::Value {
    let Some(ic) = state.assembly.in_context.as_ref() else { return serde_json::Value::Null };
    let references: Vec<serde_json::Value> = soliddraft_model::context::references(state.doc.model().features())
        .into_iter()
        .map(|(f, c)| serde_json::json!({ "feature": f.0, "label": c.label(), "broken": c.broken }))
        .collect();
    serde_json::json!({
        "component": ic.path.to_string(),
        "name": ic.name,
        "assembly": ic.assembly,
        "context": ic.context.as_ref().map_or(0, |c| c.mesh.triangle_count()),
        "picked": ic.picked.as_ref().map(|p| serde_json::json!({ "component": p.path.to_string(), "face": p.face_label })),
        "references": references,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use soliddraft_model::sketch::{EntityKind, P2, Sketch, solve::solve};

    /// Two separate solid bodies: round blocks 10 mm tall, one on the Top plane and one
    /// 50 mm above it.
    fn two_blocks() -> soliddraft_model::Document {
        use soliddraft_model::{
            Command, DatumPlane, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, ProfileSource, SketchFeature,
        };
        let kernel = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new());
        let mut d = soliddraft_model::Document::new("Blocks", kernel);
        for (body, z) in [(0, 0.0), (1, 50.0)] {
            let mut s = Sketch::new();
            s.add(EntityKind::Circle { center: P2::ZERO, radius: 5.0 }, false);
            solve(&mut s, None, true);
            let plane = soliddraft_model::SketchPlane::Datum { datum: DatumPlane::Top, offset: z };
            let kind = FeatureKind::Sketch(SketchFeature { plane, sketch: s });
            let add = Command::AddFeature { kind, name: None, position: None, via_assistant: false };
            let sketch = d.execute(add).unwrap().created.unwrap();
            let boss = FeatureKind::Extrude(ExtrudeFeature {
                profile: ProfileSource::Sketch { feature: sketch },
                operation: ExtrudeOp::Boss,
                end: EndCondition::Blind { depth: 10.0 },
                reverse: false,
                options: Default::default(),
            });
            d.execute(Command::AddFeatureInBody { body, kind: boss, via_assistant: false, position: None }).unwrap();
        }
        assert_eq!(d.live_bodies().len(), 2);
        d
    }

    /// The persistent name of the face of body `index` whose normal points along `z`.
    fn face_name(d: &soliddraft_model::Document, index: usize, z: f64) -> String {
        let body = d.live_bodies()[index].1;
        let topology = d.kernel().topology(body).unwrap();
        let id = d
            .face_labels(body)
            .into_iter()
            .map(|(f, _)| f)
            .find(|f| topology.face(*f).and_then(|i| i.normal).is_some_and(|n| n.z * z > 0.5))
            .unwrap();
        serde_json::to_string(&d.face_ref(id).unwrap()).unwrap()
    }

    fn square(z: f32, name: &str) -> DisplayMesh {
        DisplayMesh {
            positions: vec![[0.0, 0.0, z], [10.0, 0.0, z], [10.0, 10.0, z], [0.0, 10.0, z]],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            indices: vec![0, 1, 2, 0, 2, 3],
            faces: vec![MeshFace { first: 0, count: 2, name: name.into(), style: None }],
            edges: Vec::new(),
        }
    }

    #[test]
    fn a_ray_picks_the_nearest_face_of_another_component() {
        let blocks = two_blocks();
        let (low, high) = (face_name(&blocks, 0, 1.0), face_name(&blocks, 1, 1.0));
        let mut mesh = DisplayMesh::default();
        let place = soliddraft_assembly::Placement::translation(DVec3::ZERO);
        append(&mut mesh, &square(5.0, &low), &place);
        append(&mut mesh, &square(9.0, &high), &place);
        let owners = vec![("1".parse().unwrap(), "Low<1>".into()), ("2".parse().unwrap(), "High<1>".into())];
        let context = ContextMesh::new(mesh, owners);
        let down = glam::Vec3::new(0.0, 0.0, -1.0);
        let p = pick(&context, glam::Vec3::new(5.0, 4.0, 50.0), down).unwrap();
        assert_eq!((p.component.as_str(), p.face.as_str(), p.triangles.len()), ("High<1>", high.as_str(), 2));
        assert!(pick(&context, glam::Vec3::new(50.0, 4.0, 50.0), down).is_none());
        let up = pick(&context, glam::Vec3::new(5.0, 4.0, 7.0), down).unwrap();
        assert_eq!(up.component, "Low<1>");
    }

    #[test]
    fn a_face_without_a_persistent_name_is_never_picked_or_matched_by_label() {
        let mut mesh = DisplayMesh::default();
        append(&mut mesh, &square(5.0, "Boss-Extrude1 · End cap"), &Placement::translation(DVec3::ZERO));
        let context = ContextMesh::new(mesh, vec![("1".parse().unwrap(), "Low<1>".into())]);
        assert!(pick(&context, glam::Vec3::new(5.0, 4.0, 50.0), glam::Vec3::new(0.0, 0.0, -1.0)).is_none());
        let blocks = two_blocks();
        let label = blocks.face_labels(blocks.live_bodies()[0].1).into_iter().find_map(|(_, l)| l).unwrap();
        assert!(face_in_part(&blocks, &label.text).is_err(), "a label alone binds no face");
    }

    #[test]
    fn a_face_on_a_second_solid_body_gives_its_plane() {
        let blocks = two_blocks();
        let (normal, point) = face_in_part(&blocks, &face_name(&blocks, 1, -1.0)).unwrap();
        assert!((normal - DVec3::NEG_Z).length() < 1e-12, "{normal}");
        assert!((point.z - 50.0).abs() < 1e-9, "{point}");
    }

    #[test]
    fn appending_moves_each_component_into_the_part_frame() {
        let mut mesh = DisplayMesh::default();
        let place = soliddraft_assembly::Placement::translation(DVec3::new(0.0, 0.0, -5.0));
        append(&mut mesh, &square(5.0, "a"), &place);
        append(&mut mesh, &square(5.0, "b"), &place);
        assert_eq!(mesh.positions[0], [0.0, 0.0, 0.0]);
        assert_eq!(mesh.indices[6..], [4, 5, 6, 4, 6, 7]);
        assert_eq!((mesh.faces[1].first, mesh.faces[1].count), (2, 2));
    }

    #[test]
    fn a_click_on_a_datum_plane_or_the_part_drops_the_picked_face() {
        assert_eq!(click_picks(Some(None::<()>), true, false), Some(false), "a datum plane takes the click");
        assert_eq!(click_picks(Some(Some(())), false, false), Some(false), "the part takes the click");
        assert_eq!(click_picks(Some(None::<()>), false, false), Some(true));
        assert_eq!(click_picks(Some(None::<()>), false, true), None, "a sketch keeps the pick");
        assert_eq!(click_picks(None::<Option<()>>, false, false), None);
    }

    /// A part with one sketch on the base's top face, `z` millimetres from the part.
    fn plate(z: f64) -> crate::document::Document {
        let mut doc = crate::startup::new_document();
        let context = ContextFace::new(
            ("asm", "Rig"),
            "2",
            ("1", "base<1>"),
            ("{}", "Extrude1 · End cap"),
            Plane { origin: Vec3::new(0.0, 0.0, z), normal: Vec3::Z, x_dir: Vec3::X },
        );
        let plane = SketchPlane::Context { context: Box::new(context), offset: 0.0 };
        doc.apply(Command::AddSketch { plane }).unwrap();
        doc
    }

    fn reference_z(doc: &crate::document::Document) -> f64 {
        soliddraft_model::context::references(doc.model().features())[0].1.origin[2]
    }

    /// The update of `doc`'s reference to the base moved to `z`.
    fn update(doc: &crate::document::Document, z: f64) -> Command {
        let (feature, c) = soliddraft_model::context::references(doc.model().features())[0];
        let before = doc.model().feature(feature).unwrap().kind.clone();
        let moved =
            c.followed("Rig", "base<1>", Plane { origin: Vec3::new(0.0, 0.0, z), normal: Vec3::Z, x_dir: Vec3::X });
        let after = soliddraft_model::context::with_reference(&before, moved).unwrap();
        Command::FollowContext {
            part: doc.model().id(),
            edits: vec![(crate::document::app_id(feature), before, after)],
        }
    }

    #[test]
    fn an_update_read_for_one_part_is_refused_by_another() {
        // Both parts hold the same feature ids; an update queued for the first must not
        // land on the second opened before it ran.
        let (mut first, mut second) = (plate(-20.0), plate(-20.0));
        let for_first = update(&first, -15.0);
        assert!(second.apply(for_first.clone()).is_err(), "the second part takes no update read for the first");
        assert_eq!(reference_z(&second), -20.0);
        first.apply(for_first.clone()).unwrap();
        assert_eq!(reference_z(&first), -15.0);
        // Read before a later change of the same sketch, it is refused too.
        assert!(first.apply(for_first).is_err());
        assert_eq!(reference_z(&first), -15.0);
    }
}
