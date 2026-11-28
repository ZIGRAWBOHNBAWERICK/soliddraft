//! The Mac GPU preview gate: each part opens on the app's own Metal device, and its GPU
//! preview is read back from the mesher's buffers and checked against the checked mesh.
//!
//! The app picks and installs the GPU preview itself, as it does at startup. Each part
//! then opens as the app opens a file. Its preview is drawn by the viewport renderer and
//! read back before the checked mesh replaces it. Each part writes `record.json`, the
//! preview frame and the checked frame. This module records facts and problems; the
//! gate script decides.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use egui_wgpu::wgpu;
use serde_json::{Value, json};
use soliddraft_app::document::{DisplayMesh, Document};
use soliddraft_app::viewport::gpu_preview::{self, GpuFaces};
use soliddraft_app::viewport::offscreen::{Offscreen, Picture};
use soliddraft_kernel_api::{BodyId, BodyKind, EdgeId, FaceId, Kernel, Mesh, SurfaceKind, Topology};

use super::trimmed_carrier::{Point, Triangles, TrimmedCarrier};
use soliddraft_kernel_catallax::gpu_preview::PreviewVertex;

/// The frame size of the pictures, the native P1 gate's window size.
const SIZE: [u32; 2] = [1200, 780];
/// The preview tolerance where the display asks for no chord height: the Catallax
/// description's default chord target.
const DEFAULT_TOLERANCE_MM: f64 = 0.01;

/// The device the app installed its GPU preview on.
pub struct Device {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter: Value,
    pub tier: soliddraft_app::viewport::look::Tier,
}

/// Opens each part of `manifest` on `device` and writes its record under `out`.
/// `manifest` is a JSON list of `{"name", "path"}`.
pub fn read_back_parts(
    device: &Device,
    kernel: std::sync::Arc<dyn soliddraft_kernel_api::Kernel>,
    manifest: &Path,
    out: &Path,
) {
    let parts: Vec<Value> = serde_json::from_slice(&std::fs::read(manifest).expect("parts manifest")).unwrap();
    let mut offscreen = Offscreen::new(&device.device, &device.queue, device.tier);
    // Compile the pipelines and draw once before any part is timed.
    let warm = Document::new("empty", kernel.clone());
    offscreen.capture(&device.device, &device.queue, &warm, SIZE).expect("an empty frame on Metal");
    let mut summary = Vec::new();
    for part in &parts {
        let name = part["name"].as_str().expect("part name");
        let path = PathBuf::from(part["path"].as_str().expect("part path"));
        let dir = out.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let started = Instant::now();
        let record = read_back(device, &kernel, &mut offscreen, &path, &dir);
        let mut record = record.unwrap_or_else(|problem| json!({"problems": [problem]}));
        record["name"] = json!(name);
        record["path"] = json!(path);
        record["wall_ms"] = json!(ms(started));
        std::fs::write(dir.join("record.json"), serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        eprintln!("{name}: {}", record["problems"]);
        summary.push(record);
    }
    let result = json!({"adapter": device.adapter, "status": gpu_preview::status(), "parts": summary});
    std::fs::write(out.join("readback.json"), serde_json::to_vec_pretty(&result).unwrap()).unwrap();
}

fn ms(since: Instant) -> f64 {
    (since.elapsed().as_secs_f64() * 1e4).round() / 10.0
}

fn read_back(
    device: &Device,
    kernel: &std::sync::Arc<dyn soliddraft_kernel_api::Kernel>,
    offscreen: &mut Offscreen,
    path: &Path,
    dir: &Path,
) -> Result<Value, String> {
    let before = gpu_preview::counts();
    let opened = Instant::now();
    let mut doc = soliddraft_app::file::open(path, kernel.clone(), None)?;
    let open_ms = ms(opened);
    let after = gpu_preview::counts();
    let mut record = json!({
        "open_ms": open_ms,
        "status": gpu_preview::status(),
        "counts": delta(&before, &after),
        "live_bodies": doc.model().live_bodies().len(),
        "chord_height_mm": doc.tess_params.chord_height,
    });
    let mut problems = Vec::new();
    if let Some(failure) = doc.display_failure() {
        problems.push(format!("display failure: {failure}"));
    }
    let shown: Vec<(BodyId, std::sync::Arc<DisplayMesh>)> = doc.meshes.iter().map(|(b, m)| (*b, m.clone())).collect();
    let gpu_meshes: Vec<_> = shown.iter().filter(|(_, mesh)| mesh.gpu.is_some()).collect();
    record["route"] = json!(if !gpu_meshes.is_empty() {
        "gpu"
    } else if shown.iter().any(|(_, mesh)| !mesh.positions.is_empty()) {
        "cpu"
    } else {
        "none"
    });

    // The preview frame, timed twice: the first draw converts and uploads the meshes,
    // the second draws what is already on the device.
    let first = Instant::now();
    let preview = offscreen.capture(&device.device, &device.queue, &doc, SIZE).map_err(|e| e.to_string())?;
    let first_ms = ms(first);
    let again = Instant::now();
    offscreen.capture(&device.device, &device.queue, &doc, SIZE).map_err(|e| e.to_string())?;
    let frame_ms = ms(again);
    std::fs::write(dir.join("preview.png"), preview.png().map_err(|e| e.to_string())?).unwrap();
    record["timings"] = json!({
        "open_ms": open_ms,
        "first_frame_ms": first_ms,
        "frame_ms": frame_ms,
        "upload_ms": ((first_ms - frame_ms).max(0.0) * 10.0).round() / 10.0,
    });

    let tolerance =
        if doc.tess_params.chord_height > 0.0 { doc.tess_params.chord_height } else { DEFAULT_TOLERANCE_MM };
    record["tolerance_mm"] = json!(tolerance);
    let mut bodies = Vec::new();
    for (body, mesh) in &shown {
        let topology = kernel.topology(*body).map_err(|e| format!("topology of {body:?}: {e}"))?;
        let Some(gpu) = &mesh.gpu else {
            let why = match kernel.gpu_preview(*body, &doc.tess_params) {
                // The mesher ran; the preview is dropped when the CPU preview of the faces
                // it refused fails too.
                Ok(Some(_)) => match kernel.preview(*body, &doc.tess_params) {
                    Err(error) => format!("the CPU preview of the faces the GPU mesher refused failed: {error}"),
                    Ok(_) => "the kernel describes the body, but the viewport did not show a GPU preview".to_owned(),
                },
                Ok(None) => "the kernel has no GPU description of the body".to_owned(),
                Err(error) => error.to_string(),
            };
            // A lattice or gyroid is drawn by the field display, not by the B-rep mesher.
            let field = doc.features.iter().rev().find(|f| f.body == Some(*body) && !f.suppressed).is_some_and(|f| {
                matches!(
                    f.kind,
                    soliddraft_app::document::FeatureKind::Lattice(_)
                        | soliddraft_app::document::FeatureKind::Gyroid(_)
                )
            });
            bodies.push(json!({"body": format!("{body:?}"), "route": "cpu", "why": why, "field": field}));
            continue;
        };
        if gpu.device != device.device {
            problems.push(format!("{body:?}: the GPU preview is on another device"));
        }
        let checked =
            kernel.checked_rendering(*body, &doc.tess_params).map_err(|e| format!("checked mesh of {body:?}: {e}"))?;
        // The face boundaries the trimmed carrier is measured against when a point is
        // off its face: the kernel's checked edge samples.
        let edges: HashMap<EdgeId, Vec<Point>> = kernel
            .checked_rendering_edges(*body, &doc.tess_params)
            .map_err(|e| format!("checked edges of {body:?}: {e}"))?
            .into_iter()
            .map(|edge| (edge.edge, edge.points.iter().map(|p| [p.x, p.y, p.z]).collect()))
            .collect();
        let reference = Reference { kernel: kernel.as_ref(), body: *body, checked: &checked, edges: &edges };
        let mut checks = check_body(device, gpu, mesh, &topology, &reference, tolerance);
        checks["body"] = json!(format!("{body:?}"));
        for problem in checks["problems"].as_array().into_iter().flatten() {
            problems.push(format!("{body:?}: {}", problem.as_str().unwrap_or_default()));
        }
        bodies.push(checks);
    }
    record["bodies"] = json!(bodies);

    // The checked frame that replaces the preview, kept beside it.
    let replaced = Instant::now();
    doc.finish_display();
    record["timings"]["checked_ms"] = json!(ms(replaced));
    let checked = offscreen.capture(&device.device, &device.queue, &doc, SIZE).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("checked.png"), checked.png().map_err(|e| e.to_string())?).unwrap();
    record["checked_route"] = json!(doc.meshes.values().all(|mesh| mesh.gpu.is_none()));
    if let (false, Some(body)) = (doc.meshes.values().all(|mesh| mesh.gpu.is_none()), doc.display_body()) {
        // Why the preview stayed: the level the display wanted and the kernel's answer there.
        let quality = doc.display_quality();
        let wanted = quality.wanted();
        let params = *quality.levels().params(wanted);
        record["checked_missing"] = json!({
            "wanted": format!("{wanted:?}"),
            "shown": format!("{:?}", quality.shown()),
            "pending": quality.pending(),
            "chord_height_mm": params.chord_height,
            "kernel": match kernel.checked_rendering(body, &params) {
                Ok(mesh) => format!("a checked mesh of {} triangles", mesh.indices.len() / 3),
                Err(error) => error.to_string(),
            },
            // The display worker asks for every level at once; one refused level refuses all.
            "levels": soliddraft_tessellation::Level::ALL
                .iter()
                .map(|level| {
                    let answer = match kernel.checked_rendering(body, quality.levels().params(*level)) {
                        Ok(mesh) => format!("{} triangles", mesh.indices.len() / 3),
                        Err(error) => error.to_string(),
                    };
                    (format!("{level:?}"), json!(answer))
                })
                .collect::<serde_json::Map<_, _>>(),
        });
    }
    record["picture_difference"] = json!(difference(&preview, &checked));
    record["problems"] = json!(problems);
    Ok(record)
}

/// The change in the GPU preview counts across one open.
fn delta(before: &Value, after: &Value) -> Value {
    let number = |v: &Value, key: &str| v[key].as_u64().unwrap_or(0);
    let mut refusals = serde_json::Map::new();
    if let Some(now) = after["refusals"].as_object() {
        for (reason, count) in now {
            let was = before["refusals"][reason].as_u64().unwrap_or(0);
            let count = count.as_u64().unwrap_or(0);
            if count > was {
                refusals.insert(reason.clone(), json!(count - was));
            }
        }
    }
    json!({
        "gpu_previews": number(after, "gpu_previews") - number(before, "gpu_previews"),
        "cpu_previews": number(after, "cpu_previews") - number(before, "cpu_previews"),
        "refused_faces": number(after, "refused_faces") - number(before, "refused_faces"),
        "refusals": refusals,
    })
}

/// The share of pixels that differ visibly between two frames of the same size.
fn difference(a: &Picture, b: &Picture) -> f64 {
    if a.size != b.size {
        return 1.0;
    }
    let pixels = a.pixels.as_chunks::<4>().0.iter().zip(b.pixels.as_chunks::<4>().0);
    let changed = pixels.clone().filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > 32)).count();
    let total = pixels.count().max(1);
    (changed as f64 / total as f64 * 1e5).round() / 1e5
}

type Key = [u32; 3];

/// Reads one body's GPU preview back and checks it: every face drawn by the GPU or, when
/// refused, by the CPU preview; a closed, consistently wound GPU mesh whose open edges
/// meet only CPU-drawn faces; and every GPU vertex within `tolerance` of its face in the
/// checked mesh.
/// What a body's GPU preview is measured against: its checked mesh, and the kernel's
/// binary64 carriers restricted to each face's trim.
struct Reference<'a> {
    kernel: &'a dyn Kernel,
    body: BodyId,
    checked: &'a Mesh,
    edges: &'a HashMap<EdgeId, Vec<Point>>,
}

fn check_body(
    device: &Device,
    gpu: &GpuFaces,
    mesh: &DisplayMesh,
    topology: &Topology,
    reference: &Reference<'_>,
    tolerance: f64,
) -> Value {
    let checked = reference.checked;
    let mut problems = Vec::new();
    let origin = gpu.meshed.origin;
    let (vertices, indices) = match read_buffers(device, gpu) {
        Ok(read) => read,
        Err(error) => {
            problems.push(error);
            (Vec::new(), Vec::new())
        }
    };
    let cpu = Triangles::new(
        mesh.indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| t.map(|i| mesh.positions[i as usize].as_dvec3().to_array()))
            .collect(),
        tolerance,
    );
    let cpu_picks: std::collections::HashSet<u32> = mesh.face_picks.iter().copied().collect();
    let described: std::collections::HashSet<FaceId> = gpu.meshed.faces.iter().map(|face| face.face).collect();
    let missing: Vec<String> = topology
        .faces
        .iter()
        .filter(|face| !described.contains(&face.id))
        .map(|face| format!("{:?}", face.id))
        .collect();
    if !missing.is_empty() {
        problems.push(format!("faces missing from the GPU preview description: {}", missing.join(", ")));
    }

    let mut faces = Vec::new();
    // What the gate reports without failing on.
    let mut findings = Vec::new();
    let mut refused = Vec::new();
    let mut worst: f64 = 0.0;
    // Per edge: triangles that use it, its winding sum, and the faces those triangles lie on.
    let mut edges: HashMap<(Key, Key), (u32, i32, Vec<FaceId>)> = HashMap::new();
    let world = |index: u32| -> [f64; 3] {
        let p = vertices[index as usize].position;
        std::array::from_fn(|i| f64::from(p[i]) + origin[i])
    };
    let key = |index: u32| -> Key {
        let p = vertices[index as usize].position;
        [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()]
    };
    for (meshed, shown) in gpu.meshed.faces.iter().zip(&gpu.faces) {
        let range = match &meshed.draw {
            Ok(range) => range.clone(),
            Err(reason) => {
                let drawn_by_cpu = shown.pick != 0 && cpu_picks.contains(&shown.pick);
                if !drawn_by_cpu {
                    problems
                        .push(format!("face {:?} refused ({reason}) and the CPU preview did not draw it", meshed.face));
                }
                refused
                    .push(json!({"face": format!("{:?}", meshed.face), "reason": reason, "cpu_drawn": drawn_by_cpu}));
                continue;
            }
        };
        if vertices.is_empty() {
            continue;
        }
        if range.is_empty() || range.len() % 3 != 0 || range.end as usize > indices.len() {
            problems.push(format!("face {:?}: draw range {range:?} is empty or out of the index buffer", meshed.face));
            continue;
        }
        let own = &indices[range.start as usize..range.end as usize];
        let own_checked: Vec<[Point; 3]> = checked
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&checked.triangle_faces)
            .filter(|(_, face)| **face == meshed.face)
            .map(|(t, _)| t.map(|i| checked.positions[i as usize].map(f64::from)))
            .collect();
        let info = topology.face(meshed.face);
        let boundary: Vec<Vec<Point>> = info
            .map(|info| info.edges.iter().filter_map(|edge| reference.edges.get(edge).cloned()).collect())
            .unwrap_or_default();
        let carrier = TrimmedCarrier::new(
            reference.kernel,
            reference.body,
            meshed.face,
            own_checked.clone(),
            boundary,
            tolerance,
        );
        let checked_face = Triangles::new(own_checked, tolerance);
        // A spline face's checked mesh, or any checked face coarser than the tolerance,
        // cannot judge a preview to the tolerance: the trimmed carrier does instead.
        let spline = info.is_some_and(|info| info.surface == SurfaceKind::Spline);
        let by_carrier = spline || carrier.sag > tolerance;
        let (mut from_checked, mut from_carrier): (f64, f64) = (0.0, 0.0);
        let (mut off_face, mut no_hit) = (0, 0);
        let mut deviation: f64 = 0.0;
        let mut bad_normals = 0;
        let mut bad_normal = None;
        let mut used = std::collections::BTreeSet::new();
        for &index in own {
            if index as usize >= vertices.len() {
                problems.push(format!("face {:?}: index {index} past the vertex buffer", meshed.face));
                break;
            }
            if !used.insert(index) {
                continue;
            }
            let normal = vertices[index as usize].normal;
            if ((normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]) - 1.0).abs() > 1e-3 {
                bad_normals += 1;
                bad_normal.get_or_insert((world(index), normal));
            }
            let point = world(index);
            from_checked = from_checked.max(checked_face.distance(point));
            let measured = carrier.measure(point, std::array::from_fn(|i| f64::from(normal[i])), tolerance);
            from_carrier = from_carrier.max(measured.distance);
            off_face += usize::from(!measured.on_face);
            no_hit += usize::from(!measured.hit);
            deviation = if by_carrier { from_carrier } else { from_checked };
        }
        // Each triangle by its corners in winding order, rotated to start at the least, so a
        // copy drawn the same way and a copy drawn the other way can both be recognised.
        let (mut slivers, mut repeated, mut folded) = (0, 0, 0);
        let mut kept: Vec<[Key; 3]> = Vec::new();
        let mut at: HashMap<[Key; 3], usize> = HashMap::new();
        let rotated = |c: [Key; 3]| {
            let first = (0..3).min_by_key(|&i| c[i]).unwrap_or(0);
            [c[first], c[(first + 1) % 3], c[(first + 2) % 3]]
        };
        for t in own.as_chunks::<3>().0 {
            if t.iter().any(|&i| i as usize >= vertices.len()) {
                continue;
            }
            let [a, b, c] = t.map(world);
            let (u, v): ([f64; 3], [f64; 3]) =
                (std::array::from_fn(|i| b[i] - a[i]), std::array::from_fn(|i| c[i] - a[i]));
            let cross = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            // Twice the area against the longest edge squared: zero for collinear corners.
            let longest = [u, v, std::array::from_fn(|i| c[i] - b[i])]
                .map(|e| e.iter().map(|x| x * x).sum::<f64>())
                .into_iter()
                .fold(0.0, f64::max);
            if cross.iter().map(|x| x * x).sum::<f64>().sqrt() <= 1e-6 * longest {
                slivers += 1;
            }
            let corners = rotated(t.map(key));
            let reverse = rotated([corners[0], corners[2], corners[1]]);
            if at.contains_key(&corners) {
                // Drawn again the same way: it covers its first copy and opens no gap.
                repeated += 1;
            } else if let Some(&twin) = at.get(&reverse) {
                // Drawn back to back with a reversed copy: the pair covers nothing, so both
                // leave the edge counts and any gap they hid shows as a crack.
                folded += 1;
                at.remove(&reverse);
                kept[twin] = [[u32::MAX; 3]; 3];
            } else {
                at.insert(corners, kept.len());
                kept.push(corners);
            }
        }
        for t in kept.iter().filter(|t| t[0] != [u32::MAX; 3]) {
            for i in 0..3 {
                let (a, b) = (t[i], t[(i + 1) % 3]);
                if a == b {
                    continue;
                }
                let usage = edges.entry((a.min(b), a.max(b))).or_default();
                usage.0 += 1;
                usage.1 += if a < b { 1 } else { -1 };
                usage.2.push(meshed.face);
            }
        }
        let against = if by_carrier { "its trimmed carrier" } else { "the checked mesh" };
        if checked_face.is_empty() {
            problems.push(format!("face {:?}: the checked mesh has no triangles for it", meshed.face));
        } else if deviation > tolerance {
            problems.push(format!("face {:?}: {deviation:.5} mm from {against}, over {tolerance} mm", meshed.face));
        }
        if slivers > 0 {
            findings.push(format!("face {:?}: {slivers} zero-area triangles", meshed.face));
        }
        if repeated > 0 {
            findings.push(format!("face {:?}: {repeated} triangles drawn twice the same way", meshed.face));
        }
        if folded > 0 {
            findings
                .push(format!("face {:?}: {folded} triangles drawn back to back with a reversed copy", meshed.face));
        }
        if bad_normals > 0 {
            findings.push(format!(
                "face {:?}: {bad_normals} vertices without a unit normal, first {:?}",
                meshed.face,
                bad_normal.unwrap_or_default()
            ));
        }
        worst = worst.max(if checked_face.is_empty() { 0.0 } else { deviation });
        faces.push(json!({
            "face": format!("{:?}", meshed.face),
            "triangles": own.len() / 3,
            "sampled": meshed.sampled,
            "deviation_mm": deviation,
            "reference": if by_carrier { "trimmed carrier" } else { "checked mesh" },
            "surface": info.map(|info| format!("{:?}", info.surface)),
            "checked_deviation_mm": from_checked,
            "carrier_deviation_mm": from_carrier,
            "checked_sag_mm": carrier.sag,
            "sag_misses": carrier.sag_misses,
            "vertices_off_face": off_face,
            "vertices_without_carrier_hit": no_hit,
            "zero_area_triangles": slivers,
            "repeated_triangles": repeated,
            "folded_pairs": folded,
        }));
    }

    // Cracks: a GPU edge used once must meet a face the CPU drew, within the tolerance,
    // unless the body is a sheet; an edge used twice must run both ways.
    let unkey = |k: Key| -> [f64; 3] { std::array::from_fn(|i| f64::from(f32::from_bits(k[i])) + origin[i]) };
    let (mut open, mut seams, mut cracks, mut flipped, mut over) = (0, 0, 0, 0, 0);
    let mut worst_crack: Option<[f64; 3]> = None;
    let mut examples = Vec::new();
    for (&(a, b), (count, winding, users)) in &edges {
        let (count, winding) = (*count, *winding);
        match count {
            1 => {
                open += 1;
                let (a, b) = (unkey(a), unkey(b));
                if topology.kind == BodyKind::Sheet {
                    continue;
                }
                if !cpu.is_empty() && cpu.distance(a) <= tolerance && cpu.distance(b) <= tolerance {
                    seams += 1;
                } else {
                    cracks += 1;
                    worst_crack.get_or_insert(a);
                    if examples.len() < 8 {
                        examples.push(json!({"kind": "crack", "at": a, "to": b, "faces": format!("{users:?}")}));
                    }
                }
            }
            2 if winding != 0 => {
                flipped += 1;
                if examples.len() < 8 {
                    examples.push(
                        json!({"kind": "flipped", "at": unkey(a), "to": unkey(b), "faces": format!("{users:?}")}),
                    );
                }
            }
            2 => {}
            _ => {
                over += 1;
                if examples.len() < 8 {
                    examples.push(json!({"kind": "non-manifold", "uses": count, "winding": winding, "at": unkey(a),
                        "to": unkey(b), "faces": format!("{users:?}")}));
                }
            }
        }
    }
    if cracks > 0 {
        problems.push(format!("{cracks} open GPU edges meet no drawn face (first at {worst_crack:?})"));
    }
    if flipped > 0 {
        findings.push(format!("{flipped} GPU edges shared by faces wound the same way"));
    }
    if over > 0 {
        findings.push(format!("{over} GPU edges used by more than two triangles"));
    }
    json!({
        "route": "gpu",
        "kind": format!("{:?}", topology.kind),
        "faces": topology.faces.len(),
        "gpu_faces": faces.len(),
        "gpu_triangles": gpu.triangle_count(),
        "vertices_read": vertices.len(),
        "indices_read": indices.len(),
        "deviation_mm": worst,
        "edges": {"open": open, "cpu_seams": seams, "cracks": cracks, "flipped": flipped, "non_manifold": over},
        "edge_examples": examples,
        "findings": findings,
        "refused": refused,
        "face_detail": faces,
        "problems": problems,
    })
}

/// The mesher's vertex and index buffers, copied back from the device.
fn read_buffers(device: &Device, gpu: &GpuFaces) -> Result<(Vec<PreviewVertex>, Vec<u32>), String> {
    let Some(buffers) = gpu.meshed.buffers.as_ref() else {
        return Ok((Vec::new(), Vec::new()));
    };
    let (vertex_bytes, index_bytes) = (buffers.vertex.size(), buffers.index.size());
    let staging = device.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("GPU preview acceptance readback"),
        size: vertex_bytes + index_bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(&buffers.vertex, 0, &staging, 0, vertex_bytes);
    encoder.copy_buffer_to_buffer(&buffers.index, 0, &staging, vertex_bytes, index_bytes);
    device.queue.submit([encoder.finish()]);
    let slice = staging.slice(..);
    let (send, receive) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = send.send(result);
    });
    device
        .device
        .poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(std::time::Duration::from_secs(60)) })
        .map_err(|e| format!("readback poll: {e}"))?;
    receive.recv().map_err(|e| e.to_string())?.map_err(|e| format!("readback map: {e}"))?;
    let mapped = slice.get_mapped_range().map_err(|e| format!("readback range: {e}"))?;
    let (vertices, indices) = mapped.split_at(vertex_bytes as usize);
    let mut vertices = bytemuck::pod_collect_to_vec::<u8, PreviewVertex>(vertices);
    vertices.truncate(buffers.vertex_count as usize);
    let indices = bytemuck::pod_collect_to_vec::<u8, u32>(indices);
    if vertices.len() != buffers.vertex_count as usize {
        return Err(format!("read {} vertices of {}", vertices.len(), buffers.vertex_count));
    }
    Ok((vertices, indices))
}
