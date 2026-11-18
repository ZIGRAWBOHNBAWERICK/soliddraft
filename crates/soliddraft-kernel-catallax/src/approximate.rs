//! Bodies Catallax built within a stated tolerance, with no exact proof.
//!
//! Catallax returns such a body as a `NumericalApplicationBody`. It has no exact model,
//! so none of the exact queries apply to it. Catallax does not yet display or measure a
//! numerical body directly; it writes one to STEP. [`sample_through_step`] previews that
//! file, and the display, picking, measurements and mass properties here all read that
//! sample. They are estimates and say so. When Catallax displays numerical bodies
//! itself, that function is the one to replace.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

use catallax_app::{NumericalApplicationBody, NumericalBody};
use catallax_mesh::{PreviewMesh, PreviewOptions};
use catallax_numbers::{Decider, Rational, Refusal};
use catallax_shapes::CurveKind as CatCurve;
use soliddraft_kernel_api::*;

use crate::convert;

/// The display sample of a numerical body, keyed by its definition's face and edge keys.
pub(crate) type Sample = PreviewMesh<String, String>;

/// A numerical body, its interface ids and what has been read from its sample.
pub(crate) struct Approximate {
    pub outcome: Arc<NumericalApplicationBody>,
    pub faces: BTreeMap<String, FaceId>,
    pub edges: BTreeMap<String, EdgeId>,
    pub vertices: BTreeMap<String, VertexId>,
    /// Each face's origins in the interface's terms.
    pub face_origins: BTreeMap<FaceId, Vec<Origin>>,
    samples: crate::MeshCache<Sample>,
    topology: OnceLock<KernelResult<Topology>>,
    mass: OnceLock<KernelResult<MassProperties>>,
}

/// What an approximate body cannot do yet: anything that needs its exact model.
pub(crate) fn not_yet() -> KernelError {
    // The app shows such a message as Coming soon, and nothing more.
    crate::gap("Exact work on an approximate body")
}

impl Approximate {
    /// Gives each face, edge and vertex of `outcome` a new interface id and names its
    /// faces through `parents` and `tags`, as an exact body's faces are named. A blend
    /// names the picked edge among `selected_edges` when two edges share its name.
    pub(crate) fn new(
        outcome: Arc<NumericalApplicationBody>,
        parents: &[&convert::Ids],
        tags: &BTreeMap<String, OpTag>,
        selected_edges: &[EdgeId],
        next: &mut u64,
    ) -> Self {
        let mut id = || {
            *next += 1;
            *next
        };
        let definition = outcome.body().definition();
        let faces: BTreeMap<_, _> =
            definition.faces.iter().map(|face| (face.key.clone(), FaceId::from_raw(id()))).collect();
        let edges = definition.edges.iter().map(|edge| (edge.key.clone(), EdgeId::from_raw(id()))).collect();
        let vertices =
            definition.vertices.iter().map(|vertex| (vertex.key.clone(), VertexId::from_raw(id()))).collect();
        let face_origins = faces
            .iter()
            .map(|(key, face)| {
                let mut named: Vec<Origin> = outcome
                    .face_origins()
                    .get(key)
                    .into_iter()
                    .flatten()
                    .filter_map(|origin| convert::named_origin_for_edges(origin, parents, tags, selected_edges))
                    .collect();
                named.sort();
                named.dedup();
                (*face, named)
            })
            .collect();
        Approximate {
            outcome,
            faces,
            edges,
            vertices,
            face_origins,
            samples: crate::MeshCache::new(),
            topology: OnceLock::new(),
            mass: OnceLock::new(),
        }
    }

    pub(crate) fn body(&self) -> &NumericalBody {
        self.outcome.body()
    }

    /// The tolerance stated before the build, as Catallax holds it.
    pub(crate) fn tolerance(&self) -> &Rational {
        &self.body().options().tolerance_mm
    }

    pub(crate) fn accuracy(&self) -> BodyAccuracy {
        BodyAccuracy::Approximate { tolerance_mm: decimal(self.tolerance()) }
    }

    pub(crate) fn kind(&self) -> BodyKind {
        match self.body().kind() {
            catallax_topology::BodyKind::Surface => BodyKind::Sheet,
            _ => BodyKind::Solid,
        }
    }

    /// The record of the operation that built this body. No id carries over from the
    /// exact parents, so their entities are deleted and every entity here is created.
    pub(crate) fn record(&self, seq: u64, tag: &OpTag, op: OpKind, parents: &[&convert::Ids]) -> HistoryRecord {
        fn changes<I>(created: Vec<I>, deleted: Vec<I>) -> EntityChanges<I> {
            EntityChanges { created, deleted, split: Vec::new(), modified: Vec::new() }
        }
        let faces = changes(
            self.faces.values().copied().collect(),
            parents.iter().flat_map(|p| p.faces.values().copied()).collect(),
        );
        let edges = changes(
            self.edges.values().copied().collect(),
            parents.iter().flat_map(|p| p.edges.values().copied()).collect(),
        );
        let vertices = changes(
            self.vertices.values().copied().collect(),
            parents.iter().flat_map(|p| p.vertices.values().copied()).collect(),
        );
        // The roles this operation gave its faces, as named when the body was made.
        let face_roles = self
            .faces
            .values()
            .flat_map(|face| {
                let origins = self.face_origins.get(face).into_iter().flatten();
                let mut roles: Vec<_> =
                    origins.filter(|origin| &origin.tag == tag).map(|origin| origin.role.clone()).collect();
                roles.dedup();
                if roles.is_empty() {
                    roles.push(FaceRole::Other);
                }
                roles.into_iter().map(|role| FaceOrigin { face: *face, role })
            })
            .collect();
        HistoryRecord { seq, tag: tag.clone(), op, faces, edges, vertices, face_roles }
    }

    /// The display sample for these controls. Catallax's STEP preview reads solids only,
    /// so an approximate surface body has none yet.
    pub(crate) fn sample(&self, options: &catallax_mesh::MeshOptions) -> KernelResult<Arc<Sample>> {
        if self.kind() == BodyKind::Sheet {
            return Err(crate::gap("Display of an approximate surface body"));
        }
        self.samples.get(crate::display_key(options), || {
            on_request_allowance("approximate_display", |dec| {
                sample_through_step(self.body(), &PreviewOptions::from(options), dec)
            })
            .map_err(|refusal| match refusal.code {
                INCOMPLETE => KernelError::Unsupported("The display of this approximate body is incomplete.".into()),
                _ => crate::query_error("approximate_display", refusal),
            })
        })
    }

    /// The screen mesh, marked unproved.
    pub(crate) fn mesh(&self, options: &catallax_mesh::MeshOptions) -> KernelResult<Mesh> {
        let sample = self.sample(options)?;
        if sample.positions.iter().flatten().any(|v| !(*v as f32).is_finite()) {
            return Err(KernelError::Unsupported("The display coordinates cannot be represented on screen".into()));
        }
        let triangles = sample.triangles.iter().map(|t| (t.vertices, self.faces[&t.face]));
        Ok(Mesh { unproved: true, ..convert::triangles(&sample.positions, triangles) })
    }

    pub(crate) fn edges(&self, options: &catallax_mesh::MeshOptions) -> KernelResult<Vec<EdgePolyline>> {
        let sample = self.sample(options)?;
        Ok(sample
            .edges
            .iter()
            .map(|edge| EdgePolyline {
                edge: self.edges[&edge.edge],
                points: edge.vertices.iter().map(|v| sample.positions[*v].into()).collect(),
            })
            .collect())
    }

    /// Faces, edges and vertices, measured on the default display sample.
    pub(crate) fn topology(&self) -> KernelResult<Topology> {
        self.topology.get_or_init(|| self.compute_topology()).clone()
    }

    fn compute_topology(&self) -> KernelResult<Topology> {
        let options = catallax_mesh::MeshOptions::default();
        let chord = options.chord_tolerance.to_f64_nearest();
        let sample = self.sample(&options)?;
        let definition = self.body().definition();
        let tolerance = self.tolerance().to_f64_nearest();
        let point = |i: usize| Vec3::from(sample.positions[i]);
        let mut adjacency: BTreeMap<usize, Vec<FaceId>> = BTreeMap::new();
        let mut faces = Vec::new();
        for face in &definition.faces {
            let id = self.faces[&face.key];
            let triangles: Vec<_> = sample
                .faces
                .iter()
                .filter(|f| f.face == face.key)
                .flat_map(|f| sample.triangles[f.triangles.clone()].iter())
                .map(|t| t.vertices.map(point))
                .collect();
            let (area, centroid) = area_centroid(&triangles);
            if area <= 0.0 {
                return Err(KernelError::QueryFailed {
                    query: "topology".into(),
                    code: 0,
                    message: format!("the approximate display of face {} has no area", face.key),
                });
            }
            let mut edges = Vec::new();
            for trim in face.loops.iter().flatten() {
                edges.push(self.edges[&definition.edges[trim.edge].key]);
                adjacency.entry(trim.edge).or_default().push(id);
            }
            let (_, normal) =
                convert::outward_plane(&face.surface, face.sense).map_err(|e| crate::query_error("face_plane", e))?;
            faces.push(FaceInfo {
                id,
                surface: convert::surface_kind(&face.surface),
                accuracy: FaceAccuracy::Approximate { tolerance_mm: tolerance },
                area,
                centroid,
                bbox: bounds(triangles.iter().flatten().copied(), chord),
                edges,
                normal,
                origins: self.face_origins[&id].clone(),
            });
        }
        let polylines: BTreeMap<&str, Vec<Vec3>> = sample
            .edges
            .iter()
            .map(|edge| (edge.edge.as_str(), edge.vertices.iter().map(|v| point(*v)).collect()))
            .collect();
        let mut edges = Vec::new();
        for (index, edge) in definition.edges.iter().enumerate() {
            let points = polylines
                .get(edge.key.as_str())
                .cloned()
                .unwrap_or_else(|| edge.vertices.iter().map(|v| position(&definition.vertices[*v].position)).collect());
            let length = points.windows(2).map(|p| p[0].distance(p[1])).sum();
            edges.push(EdgeInfo {
                id: self.edges[&edge.key],
                curve: match edge.curve.kind() {
                    CatCurve::Line => CurveKind::Line,
                    CatCurve::Circle => CurveKind::Circle,
                    CatCurve::Ellipse => CurveKind::Ellipse,
                    _ => CurveKind::Other,
                },
                length,
                midpoint: along(&points, length / 2.0),
                bbox: bounds(points.iter().copied(), chord),
                faces: adjacency.remove(&index).unwrap_or_default(),
                construction_origins: None,
                vertices: edge
                    .vertices
                    .iter()
                    .map(|v| self.vertices[&definition.vertices[*v].key])
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            });
        }
        let vertices: Vec<_> = definition
            .vertices
            .iter()
            .map(|vertex| {
                let id = self.vertices[&vertex.key];
                VertexInfo {
                    id,
                    position: position(&vertex.position),
                    edges: edges.iter().filter(|e| e.vertices.contains(&id)).map(|e| e.id).collect(),
                }
            })
            .collect();
        Ok(Topology {
            kind: self.kind(),
            faces,
            edges,
            vertex_count: vertices.len(),
            vertices,
            bbox: bounds(sample.positions.iter().copied().map(Vec3::from), chord),
            display_chord_height: Some(chord),
        })
    }

    /// Volume, area, centroid and inertia of the closed display sample.
    pub(crate) fn mass_properties(&self) -> KernelResult<MassProperties> {
        self.mass.get_or_init(|| self.compute_mass()).clone()
    }

    fn compute_mass(&self) -> KernelResult<MassProperties> {
        if self.kind() == BodyKind::Sheet {
            return Err(KernelError::Unsupported(
                "A surface body encloses no volume, so it has no mass properties.".into(),
            ));
        }
        let sample = self.sample(&catallax_mesh::MeshOptions::default())?;
        let positions: Vec<Vec3> = sample.positions.iter().copied().map(Vec3::from).collect();
        let triangles: Vec<[usize; 3]> = sample.triangles.iter().map(|t| t.vertices).collect();
        mass_of_closed(&positions, &triangles).map_err(|why| KernelError::Unsupported(why.into()))
    }

    /// Mass properties already estimated, without starting.
    pub(crate) fn cached_mass(&self) -> Option<MassProperties> {
        self.mass.get()?.as_ref().ok().copied()
    }

    /// Catallax admitted the body under its stated tolerance; an exact check has no
    /// model to run on.
    pub(crate) fn validation(&self) -> Validation {
        let kind = self.kind();
        Validation { valid: true, kind, manifold_solid: kind == BodyKind::Solid, issues: Vec::new() }
    }

    /// The length of one edge, estimated on the display sample.
    pub(crate) fn edge_length(&self, edge: EdgeId) -> KernelResult<Measurement> {
        let topology = self.topology()?;
        let edge = topology.edge(edge).ok_or_else(|| KernelError::InvalidInput(format!("no edge {edge:?}")))?;
        Ok(estimate(edge.length))
    }

    /// The area of one face, estimated on the display sample.
    pub(crate) fn face_area(&self, face: FaceId) -> KernelResult<Measurement> {
        let topology = self.topology()?;
        let face = topology.face(face).ok_or_else(|| KernelError::InvalidInput(format!("no face {face:?}")))?;
        Ok(estimate(face.area))
    }

    /// The body as a numerical STEP file: the file states that it is approximate. Catallax
    /// bounds the rounding of each written length, not the file's distance from the body.
    pub(crate) fn export_step(&self, options: &StepOptions) -> KernelResult<StepExport> {
        let limit = Rational::from_f64_exact(options.tolerance)
            .filter(|limit| limit.sign() == catallax_numbers::Sign::Positive)
            .ok_or_else(|| KernelError::InvalidInput("the STEP tolerance must be a positive number".into()))?;
        let product = if options.product_name.trim().is_empty() { "part" } else { options.product_name.as_str() };
        let step_options = catallax_step::numerical::NumericalStepOptions {
            product_name: product.to_owned(),
            max_scalar_rounding_error_mm: limit.clone(),
            ..Default::default()
        };
        let step =
            on_request_allowance("export_step", |dec| catallax_step::numerical::build(self.body(), &step_options, dec))
                .map_err(|e| crate::query_error("export_step", e))?;
        let report = step.report();
        let rounding = &report.max_scalar_rounding_error_mm;
        let [faces, edges, vertices] = report.counts;
        Ok(StepExport {
            text: String::from_utf8(step.bytes().to_vec()).map_err(|e| KernelError::QueryFailed {
                query: "export_step".into(),
                code: 0,
                message: e.to_string(),
            })?,
            report: StepReport {
                max_deviation: NOT_ESTABLISHED.into(),
                max_deviation_mm: 0.0,
                fit_mm: 0.0,
                // The file's distance from the body is not established, so it is never
                // shown to meet the requested tolerance, however small the rounding.
                within_tolerance: false,
                solids: usize::from(self.kind() == BodyKind::Solid),
                faces,
                edges,
                vertices,
                fitted_curves: 0,
                added_vertices: 0,
                added_edges: 0,
                removed_faces: 0,
                removed_edges: 0,
                warnings: Vec::new(),
                approximate: Some(ApproximateStep {
                    tolerance_mm: self.tolerance().to_f64_nearest(),
                    max_length_rounding_mm: crate::step::f64_up(rounding),
                }),
            },
        })
    }

    /// About how many bytes this body keeps: its definition and its samples.
    pub(crate) fn retained_bytes(&self) -> usize {
        const FACE: usize = 2048;
        const EDGE: usize = 512;
        let mut bytes = self.faces.len() * FACE + self.edges.len() * EDGE;
        self.samples.each(|m| {
            bytes += m.positions.len() * 24
                + m.triangles.len() * 32
                + m.faces.len() * 64
                + m.edges.iter().map(|e| 64 + e.vertices.len() * 8).sum::<usize>()
        });
        bytes
    }
}

/// The display sample of a numerical body: Catallax writes the body's numerical STEP file
/// and previews that file. The preview names faces and edges by STEP entity; the writer
/// labels each entity with its definition key, which names them here. Nothing about the
/// sample is proved.
/// Writes or shows an approximate body on the caller's allowance, or on the app's request
/// allowance when none is active, as an exact body's display and export run. Left to
/// itself, Catallax's numerical STEP writer stops at its smaller standalone default,
/// which the STEP of a rounded pocket rim passes.
fn on_request_allowance<T>(
    operation: &str,
    run: impl FnOnce(&mut Decider) -> Result<T, Refusal>,
) -> Result<T, Refusal> {
    let observed = || crate::numerical(operation, |dec| run(dec).map(catallax_numbers::Answer::exact));
    if catallax_numbers::operation_budget_active() {
        return observed().map(|answer| answer.value);
    }
    let (result, work) = catallax_numbers::with_operation_budget(catallax_app::DEFAULT_BUILD_WORK, observed);
    if work.exhausted {
        return Err(Refusal::new(
            "numbers.operation-budget",
            format!("{operation} exhausted the application's work allowance"),
        ));
    }
    result.map(|answer| answer.value)
}

fn sample_through_step(body: &NumericalBody, options: &PreviewOptions, dec: &mut Decider) -> Result<Sample, Refusal> {
    let step = catallax_step::numerical::build(body, &Default::default(), dec)?;
    let step = step.bytes();
    let preview = catallax_mesh::preview_step(step, options, dec)?;
    let definition = body.definition();
    let labels = entity_labels(step, &["ADVANCED_FACE", "EDGE_CURVE"]);
    let keys: BTreeMap<String, &str> = definition
        .faces
        .iter()
        .map(|face| face.key.as_str())
        .chain(definition.edges.iter().map(|edge| edge.key.as_str()))
        .map(|key| (step_string(key), key))
        .collect();
    let key = |entity: &str| -> Result<String, Refusal> {
        labels.get(entity).and_then(|label| keys.get(label)).map(|key| (*key).to_owned()).ok_or_else(|| {
            Refusal::new("approximate.display-names", format!("STEP entity {entity} names no body entity"))
        })
    };
    let mesh = preview.mesh;
    // A preview that leaves a face out would display, pick and measure less than the body.
    for face in &definition.faces {
        let shown = mesh.faces.iter().any(|f| {
            !f.triangles.is_empty() && labels.get(&f.face).and_then(|l| keys.get(l)) == Some(&face.key.as_str())
        });
        if !shown {
            return Err(Refusal::new(INCOMPLETE, format!("face {} has no display triangles", face.key)));
        }
    }
    Ok(PreviewMesh {
        positions: mesh.positions,
        triangles: mesh
            .triangles
            .into_iter()
            .map(|t| Ok(catallax_mesh::Triangle { vertices: t.vertices, face: key(&t.face)? }))
            .collect::<Result<_, Refusal>>()?,
        faces: mesh
            .faces
            .into_iter()
            .map(|f| Ok(catallax_mesh::PreviewFace { face: key(&f.face)?, triangles: f.triangles }))
            .collect::<Result<_, Refusal>>()?,
        edges: mesh
            .edges
            .into_iter()
            .map(|e| Ok(catallax_mesh::PreviewEdge { edge: key(&e.edge)?, vertices: e.vertices }))
            .collect::<Result<_, Refusal>>()?,
    })
}

/// The refusal of a display sample that leaves out a face of the body.
const INCOMPLETE: &str = "approximate.display-incomplete";

/// The quoted name, as STEP text writes it, of each entity of these types, by entity
/// name such as `#12`.
fn entity_labels(step: &[u8], types: &[&str]) -> BTreeMap<String, String> {
    let text = String::from_utf8_lossy(step);
    let mut labels = BTreeMap::new();
    for statement in statements(&text) {
        let statement = statement.trim();
        let Some((entity, rest)) = statement.split_once('=') else { continue };
        let (entity, rest) = (entity.trim(), rest.trim_start());
        if !entity.starts_with('#') {
            continue;
        }
        let Some(kind) = types.iter().find(|kind| rest.starts_with(&format!("{kind}("))) else { continue };
        let argument = rest[kind.len() + 1..].trim_start();
        if let Some(label) = quoted(argument) {
            labels.insert(entity.to_owned(), label);
        }
    }
    labels
}

/// The statements of STEP text: split at each `;` outside a string literal. A literal
/// writes its own quote doubled, so a doubled quote neither opens nor closes one.
fn statements(text: &str) -> impl Iterator<Item = &str> {
    let mut quoted = false;
    let mut start = 0;
    let mut ends = Vec::new();
    for (i, c) in text.char_indices() {
        match c {
            // A doubled quote toggles twice, which leaves the state as it was.
            '\'' => quoted = !quoted,
            ';' if !quoted => {
                ends.push((start, i));
                start = i + 1;
            }
            _ => {}
        }
    }
    ends.push((start, text.len()));
    ends.into_iter().map(move |(from, to)| &text[from..to])
}

/// A STEP string literal at the start of `text`, quotes and escapes included.
fn quoted(text: &str) -> Option<String> {
    let mut chars = text.char_indices();
    if chars.next()?.1 != '\'' {
        return None;
    }
    let mut end = None;
    let mut chars = chars.peekable();
    while let Some((i, c)) = chars.next() {
        if c == '\'' {
            if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                chars.next();
                continue;
            }
            end = Some(i);
            break;
        }
    }
    Some(text[..=end?].to_owned())
}

/// `value` as a STEP string literal, written as Catallax's STEP writer writes it.
fn step_string(value: &str) -> String {
    let mut out = String::from("'");
    for c in value.chars() {
        match c {
            '\'' => out.push_str("''"),
            '\\' => out.push_str("\\\\"),
            ' '..='~' => out.push(c),
            _ => {
                out.push_str("\\X2\\");
                for unit in c.encode_utf16(&mut [0; 2]) {
                    out.push_str(&format!("{unit:04X}"));
                }
                out.push_str("\\X0\\");
            }
        }
    }
    out.push('\'');
    out
}

/// A value with no claimed bound.
fn estimate(value: f64) -> Measurement {
    Measurement { value, lo: value, hi: value, certainty: Certainty::Approximate }
}

/// A decimal for `value`, exact when it terminates, else to 20 significant places.
fn decimal(value: &Rational) -> String {
    let (lo, hi) = value.to_decimal_bounds(20);
    if lo == hi { lo } else { hi }
}

fn position(p: &[Rational; 3]) -> Vec3 {
    Vec3::from(p.each_ref().map(Rational::to_f64_nearest))
}

/// The point `distance` along a polyline.
fn along(points: &[Vec3], distance: f64) -> Vec3 {
    let mut left = distance;
    for pair in points.windows(2) {
        let step = pair[0].distance(pair[1]);
        if step >= left && step > 0.0 {
            return pair[0] + (pair[1] - pair[0]) * (left / step);
        }
        left -= step;
    }
    points.last().copied().unwrap_or(Vec3::ZERO)
}

fn bounds(points: impl Iterator<Item = Vec3>, padding: f64) -> Aabb {
    let mut min = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut max = -min;
    let mut any = false;
    for p in points {
        any = true;
        min = Vec3::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z));
        max = Vec3::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z));
    }
    let pad = Vec3::new(padding, padding, padding);
    if any { Aabb { min: min - pad, max: max + pad } } else { Aabb { min: Vec3::ZERO, max: Vec3::ZERO } }
}

/// Area and area centroid of triangles.
fn area_centroid(triangles: &[[Vec3; 3]]) -> (f64, Vec3) {
    let mut area = 0.0;
    let mut moment = Vec3::ZERO;
    for [a, b, c] in triangles {
        let weight = (*b - *a).cross(*c - *a).length() * 0.5;
        area += weight;
        moment = moment + (*a + *b + *c) * (weight / 3.0);
    }
    (area, if area > 0.0 { moment * (1.0 / area) } else { Vec3::ZERO })
}

/// Mass properties at unit density of the solid a closed triangle mesh encloses, by the
/// divergence theorem over signed tetrahedra from the centre of the mesh's box, so a part
/// far from the origin keeps its digits. Points at the same position are one vertex. The
/// mesh must be closed and consistently wound: every edge used once in each direction.
/// A mesh wound inward gives the same answer. The reason in words when it cannot answer.
fn mass_of_closed(positions: &[Vec3], triangles: &[[usize; 3]]) -> Result<MassProperties, &'static str> {
    const OPEN: &str = "The approximate display of this body is not closed, so its mass is not estimated.";
    // Weld points at the same position, as faces sample their shared edges alike.
    let mut welded: BTreeMap<[u64; 3], usize> = BTreeMap::new();
    let ids: Vec<usize> = positions
        .iter()
        .map(|p| {
            let next = welded.len();
            *welded.entry([p.x, p.y, p.z].map(|v| (v + 0.0).to_bits())).or_insert(next)
        })
        .collect();
    let mut directed: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut used = Vec::new();
    for triangle in triangles {
        let [a, b, c] = triangle.map(|v| ids[v]);
        if a == b || b == c || c == a {
            continue;
        }
        for edge in [(a, b), (b, c), (c, a)] {
            *directed.entry(edge).or_default() += 1;
        }
        used.push(*triangle);
    }
    if used.is_empty() || directed.iter().any(|(&(a, b), &n)| n != 1 || directed.get(&(b, a)) != Some(&1)) {
        return Err(OPEN);
    }
    let corners = used.iter().flatten().map(|v| positions[*v]);
    let mut min = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut max = -min;
    for p in corners {
        min = Vec3::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z));
        max = Vec3::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z));
    }
    let origin = (min + max) * 0.5;
    let mut volume = 0.0;
    let mut first = Vec3::ZERO;
    // Second moments about `origin`: integrals of xx, yy, zz, xy, yz, zx.
    let mut second = [0.0_f64; 6];
    let mut area = 0.0;
    for triangle in &used {
        let [a, b, c] = triangle.map(|v| positions[v] - origin);
        area += (b - a).cross(c - a).length() * 0.5;
        let six = a.dot(b.cross(c));
        volume += six / 6.0;
        first = first + (a + b + c) * (six / 24.0);
        let p = [[a.x, a.y, a.z], [b.x, b.y, b.z], [c.x, c.y, c.z]];
        let co = |i: usize, j: usize| -> f64 {
            // Integral of x_i x_j over the tetrahedron (0, a, b, c): its signed volume
            // over 20, times the sum of products plus the product of sums.
            let sum_i: f64 = p.iter().map(|v| v[i]).sum();
            let sum_j: f64 = p.iter().map(|v| v[j]).sum();
            let products: f64 = p.iter().map(|v| v[i] * v[j]).sum();
            (products + sum_i * sum_j) / 120.0 * six
        };
        for (k, (i, j)) in [(0, 0), (1, 1), (2, 2), (0, 1), (1, 2), (2, 0)].into_iter().enumerate() {
            second[k] += co(i, j);
        }
    }
    if !volume.is_finite() || volume.abs() <= f64::EPSILON * area.max(1.0) {
        return Err("The approximate display of this body encloses no volume to measure.");
    }
    // An inward winding flips every signed integral together.
    if volume < 0.0 {
        volume = -volume;
        first = -first;
        second = second.map(|s| -s);
    }
    let local = first * (1.0 / volume);
    let (cx, cy, cz) = (local.x, local.y, local.z);
    // Second moments about the centroid.
    let xx = second[0] - volume * cx * cx;
    let yy = second[1] - volume * cy * cy;
    let zz = second[2] - volume * cz * cz;
    let xy = second[3] - volume * cx * cy;
    let yz = second[4] - volume * cy * cz;
    let zx = second[5] - volume * cz * cx;
    let inertia = [[yy + zz, -xy, -zx], [-xy, xx + zz, -yz], [-zx, -yz, xx + yy]];
    let centroid = origin + local;
    let finite = [volume, area, centroid.x, centroid.y, centroid.z].iter().all(|v| v.is_finite())
        && inertia.iter().flatten().all(|v| v.is_finite());
    if !finite {
        return Err("The mass of this approximate body could not be estimated in floating point.");
    }
    Ok(MassProperties { volume, area, centroid, inertia: Some(inertia) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cube of side `size` from `corner`, each face with its own four points, as a
    /// display samples faces apart, wound outward.
    fn cube(corner: Vec3, size: f64) -> (Vec<Vec3>, Vec<[usize; 3]>) {
        let v = |x: f64, y: f64, z: f64| corner + Vec3::new(x * size, y * size, z * size);
        let quads = [
            [v(0., 0., 0.), v(0., 1., 0.), v(1., 1., 0.), v(1., 0., 0.)],
            [v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)],
            [v(0., 0., 0.), v(1., 0., 0.), v(1., 0., 1.), v(0., 0., 1.)],
            [v(0., 1., 0.), v(0., 1., 1.), v(1., 1., 1.), v(1., 1., 0.)],
            [v(0., 0., 0.), v(0., 0., 1.), v(0., 1., 1.), v(0., 1., 0.)],
            [v(1., 0., 0.), v(1., 1., 0.), v(1., 1., 1.), v(1., 0., 1.)],
        ];
        let positions = quads.iter().flatten().copied().collect();
        let triangles = (0..6).flat_map(|f| [[4 * f, 4 * f + 1, 4 * f + 2], [4 * f, 4 * f + 2, 4 * f + 3]]).collect();
        (positions, triangles)
    }

    fn assert_cube(mass: &MassProperties, corner: Vec3, size: f64) {
        let volume = size.powi(3);
        assert!((mass.volume - volume).abs() <= 1e-12 * volume, "{mass:?}");
        assert!((mass.area - 6.0 * size * size).abs() <= 1e-9 * size * size, "{mass:?}");
        let middle = corner + Vec3::new(size, size, size) * 0.5;
        assert!(mass.centroid.distance(middle) <= 1e-9 * size, "{mass:?}");
        // A cube of side s and volume V: V s^2 / 6 about each centroidal axis.
        let inertia = mass.inertia.unwrap();
        for (i, row) in inertia.iter().enumerate() {
            for (j, value) in row.iter().enumerate() {
                let expected = if i == j { volume * size * size / 6.0 } else { 0.0 };
                assert!((value - expected).abs() <= 1e-9 * volume * size * size, "{inertia:?}");
            }
        }
    }

    #[test]
    fn a_closed_cube_has_its_volume_centroid_and_inertia() {
        let (positions, triangles) = cube(Vec3::ZERO, 2.0);
        let mass = mass_of_closed(&positions, &triangles).unwrap();
        assert_cube(&mass, Vec3::ZERO, 2.0);
        let inward: Vec<_> = triangles.iter().map(|[a, b, c]| [*a, *c, *b]).collect();
        assert_eq!(mass_of_closed(&positions, &inward).unwrap().volume, mass.volume);
    }

    /// Integrated about the world origin, a unit cube 1e6 mm away had a centroidal inertia
    /// near 174 rather than 1/6, and 1e9 mm away it lost its volume.
    #[test]
    fn a_cube_far_from_the_origin_keeps_its_mass_properties() {
        for offset in [1e6, 1e9] {
            let corner = Vec3::new(offset, -offset, offset);
            let (positions, triangles) = cube(corner, 1.0);
            assert_cube(&mass_of_closed(&positions, &triangles).unwrap(), corner, 1.0);
        }
    }

    #[test]
    fn an_open_or_inconsistently_wound_mesh_has_no_mass_estimate() {
        let (positions, mut triangles) = cube(Vec3::ZERO, 1.0);
        let open = &triangles[1..];
        assert!(mass_of_closed(&positions, open).unwrap_err().contains("not closed"));
        triangles[0] = [triangles[0][0], triangles[0][2], triangles[0][1]];
        assert!(mass_of_closed(&positions, &triangles).unwrap_err().contains("not closed"));
        assert!(mass_of_closed(&positions, &[]).is_err());
    }

    /// A 10 x 8 x 6 mm block's numerical definition, the index of its top face and its
    /// recipe.
    fn block_definition() -> (catallax_app::NumericalDefinition, usize, catallax_recipe::Recipe) {
        use crate::CatallaxKernel;
        let kernel = CatallaxKernel::new();
        let profile = kernel
            .create_body_from_profile(&OpTag::new("p"), &Profile::rectangle(Plane::XY, [0.0, 0.0].into(), 10.0, 8.0))
            .unwrap()
            .body;
        let block = kernel.extrude(&OpTag::new("e"), profile, &Extrude { distance: 6.0, direction: None }).unwrap();
        let solid = kernel.solid(block.body).unwrap();
        let built = solid.app.checkpoint.built();
        let options = catallax_app::NumericalOptions::default();
        let mut dec = Decider::default();
        let (definition, trims) = catallax_features::numerical_embedding::definition_from_exact_model(
            &built.model,
            built.result,
            "block".into(),
            &options.tolerance_mm,
            &mut dec,
        )
        .unwrap();
        assert!(trims.is_empty(), "a planar block has no native trims");
        let top = definition
            .faces
            .iter()
            .position(|face| {
                let catallax_shapes::Surface::Plane(plane) = &face.surface else { return false };
                plane.normal.z.to_f64().abs() > 0.5 && plane.origin.z.to_f64() > 3.0
            })
            .unwrap();
        (definition, top, built.recipe().clone())
    }

    /// An open numerical shell: a block's definition without its top face. Catallax's STEP
    /// preview reads solids only, so its display says Coming soon rather than showing an
    /// empty body that measures nothing.
    #[test]
    fn an_open_numerical_sheet_says_its_display_is_coming() {
        let (mut definition, top, recipe) = block_definition();
        definition.faces.remove(top);
        let options = catallax_app::NumericalOptions::default();
        let sheet = catallax_topology::numerical::build_surface(definition, options, &mut Decider::default()).unwrap();
        let outcome = NumericalApplicationBody::new(sheet, Arc::new(recipe), std::collections::BTreeMap::new());
        let approximate = Approximate::new(Arc::new(outcome), &[], &BTreeMap::new(), &[], &mut 0);
        assert_eq!(approximate.kind(), BodyKind::Sheet);
        assert_eq!(approximate.accuracy(), BodyAccuracy::Approximate { tolerance_mm: "0.01".into() });
        let options = catallax_mesh::MeshOptions::default();
        for refusal in [
            approximate.mesh(&options).unwrap_err(),
            approximate.edges(&options).unwrap_err(),
            approximate.topology().unwrap_err(),
        ] {
            assert!(refusal.to_string().ends_with("is coming soon."), "{refusal}");
        }
        let mass = approximate.mass_properties().unwrap_err().to_string();
        assert!(mass.contains("surface body encloses no volume"), "{mass}");
    }

    /// A face key is any string, and STEP quotes it. A semicolon inside the quotes must not
    /// end the statement, or that face would go unnamed and the display be refused.
    #[test]
    fn a_face_key_with_a_semicolon_still_names_its_face() {
        let (mut definition, top, recipe) = block_definition();
        definition.faces[top].key = "cap;top".into();
        let options = catallax_app::NumericalOptions::default();
        let body = catallax_topology::numerical::build(definition, options, &mut Decider::default()).unwrap();
        let outcome = NumericalApplicationBody::new(body, Arc::new(recipe), std::collections::BTreeMap::new());
        let approximate = Approximate::new(Arc::new(outcome), &[], &BTreeMap::new(), &[], &mut 0);
        let topology = approximate.topology().unwrap();
        let top = topology.face(approximate.faces["cap;top"]).unwrap();
        assert!((top.area - 80.0).abs() < 1e-6 && (top.centroid.z - 6.0).abs() < 1e-6, "{top:?}");
        assert!((approximate.mass_properties().unwrap().volume - 480.0).abs() < 1e-6);
    }

    #[test]
    fn entity_labels_read_quoted_names_with_escaped_quotes() {
        let text =
            b"#12=ADVANCED_FACE('FaceId(3)',(#4),#5,.T.);\n#13 = EDGE_CURVE('it''s',#1,#2,#3,.T.);\n#14=PLANE('x',#9);";
        let labels = entity_labels(text, &["ADVANCED_FACE", "EDGE_CURVE"]);
        assert_eq!(labels.len(), 2);
        assert_eq!(labels["#12"], step_string("FaceId(3)"));
        assert_eq!(labels["#13"], step_string("it's"));
        // A semicolon or a doubled quote inside the quotes ends nothing.
        let text = b"#42=ADVANCED_FACE('cap;top',(#4),#5,.T.);\n#43=EDGE_CURVE('a'';b',#1,#2,#3,.T.);";
        let labels = entity_labels(text, &["ADVANCED_FACE", "EDGE_CURVE"]);
        assert_eq!(labels["#42"], step_string("cap;top"));
        assert_eq!(labels["#43"], step_string("a';b"));
    }
}
