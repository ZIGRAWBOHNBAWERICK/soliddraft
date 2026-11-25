//! Display and picking for the default STEP reader's separately admitted bodies.

use catallax_import::tolerant::opened::{BodyView, OpenedStep};
use catallax_mesh::PreviewOptions;
use catallax_numbers::{Decider, Refusal};

use super::*;

impl Approximate {
    pub(crate) fn imported(opened: Arc<OpenedStep>, index: usize, tag: &OpTag, next: &mut u64) -> KernelResult<Self> {
        let mut id = || {
            *next += 1;
            *next
        };
        let (face_keys, edge_keys, vertex_keys): (Vec<_>, Vec<_>, Vec<_>) = match opened.body(index) {
            Some(BodyView::Certified { model, body }) => (
                model.faces_of_body(body).into_iter().map(|face| checked_key(face.index())).collect(),
                model.edges_of_body(body).into_iter().map(|edge| checked_key(edge.index())).collect(),
                model.vertices_of_body(body).into_iter().map(|vertex| checked_key(vertex.index())).collect(),
            ),
            Some(BodyView::Tolerant { import, body }) => {
                let definition = import.bodies()[body].definition();
                let used_edges: BTreeSet<_> = definition
                    .shells()
                    .iter()
                    .flat_map(|shell| &shell.faces)
                    .flat_map(|face| &face.loops)
                    .flat_map(|boundary| &boundary.edges)
                    .map(|usage| usage.edge)
                    .collect();
                let used_vertices: BTreeSet<_> = used_edges
                    .iter()
                    .flat_map(|edge| definition.edges()[*edge].vertices)
                    .chain(
                        definition
                            .shells()
                            .iter()
                            .flat_map(|shell| &shell.faces)
                            .flat_map(|face| &face.loops)
                            .filter_map(|boundary| boundary.vertex),
                    )
                    .collect();
                (
                    definition.shells().iter().flat_map(|shell| &shell.faces).map(|face| face.name.clone()).collect(),
                    used_edges.into_iter().map(|edge| definition.edges()[edge].name.clone()).collect(),
                    used_vertices.into_iter().map(|vertex| definition.vertices()[vertex].source.name.clone()).collect(),
                )
            }
            Some(BodyView::Numerical(body)) => {
                let definition = body.definition();
                (
                    definition.faces.iter().map(|face| face.key.clone()).collect(),
                    definition.edges.iter().map(|edge| edge.key.clone()).collect(),
                    definition.vertices.iter().map(|vertex| vertex.key.clone()).collect(),
                )
            }
            _ => return Err(KernelError::InvalidInput("This imported body has no admitted solid geometry.".into())),
        };
        let faces: BTreeMap<_, _> = face_keys.into_iter().map(|key| (key, FaceId::from_raw(id()))).collect();
        let edges = edge_keys.into_iter().map(|key| (key, EdgeId::from_raw(id()))).collect();
        let vertices = vertex_keys.into_iter().map(|key| (key, VertexId::from_raw(id()))).collect();
        let face_origins = faces
            .iter()
            .map(|(key, face)| {
                (*face, vec![Origin { tag: tag.clone(), role: FaceRole::Imported { key: key.clone() } }])
            })
            .collect();
        Ok(Self {
            geometry: Geometry::Imported { opened, index },
            faces,
            edges,
            vertices,
            face_origins,
            samples: crate::MeshCache::new(),
            topology: OnceLock::new(),
            mass: OnceLock::new(),
        })
    }
}

pub(super) fn sample(
    opened: &OpenedStep,
    index: usize,
    options: &PreviewOptions,
    dec: &mut Decider,
) -> Result<Sample, Refusal> {
    match opened.body(index) {
        Some(BodyView::Certified { model, body }) => {
            // The checked model is the actual healed geometry, not a certificate against the source.
            let preview = catallax_mesh::preview_mesh_body(model, body, options, dec)?;
            let shown: BTreeSet<_> =
                preview.faces.iter().filter(|face| !face.triangles.is_empty()).map(|face| face.face).collect();
            if model.faces_of_body(body).into_iter().any(|face| !shown.contains(&face)) {
                return Err(Refusal::new(INCOMPLETE, "the healed model has an undisplayed face"));
            }
            Ok(catallax_mesh::PreviewMesh {
                positions: preview.positions,
                triangles: preview
                    .triangles
                    .into_iter()
                    .map(|triangle| catallax_mesh::Triangle {
                        vertices: triangle.vertices,
                        face: checked_key(triangle.face.index()),
                    })
                    .collect(),
                faces: preview
                    .faces
                    .into_iter()
                    .map(|face| catallax_mesh::PreviewFace {
                        face: checked_key(face.face.index()),
                        triangles: face.triangles,
                    })
                    .collect(),
                edges: preview
                    .edges
                    .into_iter()
                    .map(|edge| catallax_mesh::PreviewEdge {
                        edge: checked_key(edge.edge.index()),
                        vertices: edge.vertices,
                    })
                    .collect(),
            })
        }
        Some(BodyView::Tolerant { import, body }) => catallax_mesh::preview_tolerant_body(import, body, options, dec),
        Some(BodyView::Numerical(body)) => super::sample_through_step(body, options, dec),
        _ => Err(Refusal::new(INCOMPLETE, "the selected source body has no admitted display geometry")),
    }
}

pub(super) fn topology(this: &Approximate, opened: &Arc<OpenedStep>, index: usize) -> KernelResult<Topology> {
    if let Some(BodyView::Certified { model, body }) = opened.body(index) {
        return checked_topology(this, model, body);
    }
    let Some(BodyView::Tolerant { import, body }) = opened.body(index) else {
        return this.compute_numerical_topology(this.body()?);
    };
    let definition = import.bodies()[body].definition();
    let options = catallax_mesh::MeshOptions::default();
    let chord = options.chord_tolerance.to_f64_nearest();
    let sample = this.sample(&options)?;
    let mut dec = Decider::default();
    let source = catallax_import::direct::DirectSolid::from_opened(opened.clone(), index, &mut dec)
        .map_err(|failure| crate::query_error("topology", error(failure)))?;
    let source_faces: BTreeMap<_, _> = source.faces().iter().map(|face| (face.name.as_str(), face)).collect();
    let point = |i: usize| Vec3::from(sample.positions[i]);
    let mut adjacency: BTreeMap<usize, Vec<FaceId>> = BTreeMap::new();
    let mut faces = Vec::new();
    for face in definition.shells().iter().flat_map(|shell| &shell.faces) {
        let id = this.faces[&face.name];
        let triangles: Vec<_> = sample
            .faces
            .iter()
            .filter(|f| f.face == face.name)
            .flat_map(|f| sample.triangles[f.triangles.clone()].iter())
            .map(|triangle| triangle.vertices.map(point))
            .collect();
        let (area, centroid) = area_centroid(&triangles);
        if area <= 0.0 {
            return Err(KernelError::InvalidInput(
                "One of the imported faces could not be displayed. The file was not imported.".into(),
            ));
        }
        let mut edges = Vec::new();
        for usage in face.loops.iter().flat_map(|boundary| &boundary.edges) {
            let adjacent = adjacency.entry(usage.edge).or_default();
            if !adjacent.contains(&id) {
                adjacent.push(id);
            }
            let edge = this.edges[&definition.edges()[usage.edge].name];
            if !edges.contains(&edge) {
                edges.push(edge);
            }
        }
        let surface = source_faces.get(face.name.as_str()).map_or(&face.surface, |face| &face.surface);
        let (surface, normal) = match surface {
            catallax_import::Surface::Plane(placement) => {
                let normal = Vec3::from(placement.axis.each_ref().map(|value| value.to_f64_nearest())).normalized();
                (SurfaceKind::Plane, normal.map(|normal| if face.same_sense { normal } else { -normal }))
            }
            catallax_import::Surface::Cylinder(..) => (SurfaceKind::Cylinder, None),
            catallax_import::Surface::Cone(..) | catallax_import::Surface::Cone45(..) => (SurfaceKind::Cone, None),
            catallax_import::Surface::Sphere(..) => (SurfaceKind::Sphere, None),
            catallax_import::Surface::Torus(..) => (SurfaceKind::Torus, None),
            catallax_import::Surface::BSpline(..) => (SurfaceKind::Spline, None),
            catallax_import::Surface::Retained(..) => (SurfaceKind::Other, None),
        };
        faces.push(FaceInfo {
            id,
            surface,
            normal,
            accuracy: FaceAccuracy::Unknown,
            area,
            centroid,
            bbox: bounds(triangles.iter().flatten().copied(), chord),
            edges,
            origins: this.face_origins[&id].clone(),
        });
    }
    let polylines: BTreeMap<_, _> = sample.edges.iter().map(|edge| (edge.edge.as_str(), edge)).collect();
    let mut edges = Vec::new();
    for (index, edge) in definition.edges().iter().enumerate() {
        if !this.edges.contains_key(&edge.name) {
            continue;
        }
        let points: Vec<_> = match polylines.get(edge.name.as_str()) {
            Some(polyline) => polyline.vertices.iter().map(|vertex| point(*vertex)).collect(),
            None => edge
                .vertices
                .iter()
                .map(|vertex| {
                    Vec3::from(definition.vertices()[*vertex].source.point.each_ref().map(|v| v.to_f64_nearest()))
                })
                .collect(),
        };
        let length = points.windows(2).map(|points| points[0].distance(points[1])).sum();
        edges.push(EdgeInfo {
            id: this.edges[&edge.name],
            curve: match edge.curve {
                catallax_import::Curve::Line(..) => CurveKind::Line,
                catallax_import::Curve::Circle(..) => CurveKind::Circle,
                catallax_import::Curve::Ellipse(..) => CurveKind::Ellipse,
                catallax_import::Curve::BSpline(..) => CurveKind::Spline,
                catallax_import::Curve::Retained(..) => CurveKind::Other,
            },
            length,
            midpoint: along(&points, length / 2.0),
            bbox: bounds(points.iter().copied(), chord),
            faces: adjacency.remove(&index).unwrap_or_default(),
            construction_origins: None,
            vertices: edge
                .vertices
                .iter()
                .map(|vertex| this.vertices[&definition.vertices()[*vertex].source.name])
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        });
    }
    let vertices: Vec<_> = definition
        .vertices()
        .iter()
        .filter(|vertex| this.vertices.contains_key(&vertex.source.name))
        .map(|vertex| {
            let id = this.vertices[&vertex.source.name];
            VertexInfo {
                id,
                position: Vec3::from(vertex.source.point.each_ref().map(|v| v.to_f64_nearest())),
                edges: edges.iter().filter(|edge| edge.vertices.contains(&id)).map(|edge| edge.id).collect(),
            }
        })
        .collect();
    Ok(Topology {
        kind: BodyKind::Solid,
        faces,
        edges,
        vertex_count: vertices.len(),
        vertices,
        bbox: bounds(sample.positions.iter().copied().map(Vec3::from), chord),
        display_chord_height: Some(chord),
    })
}

fn checked_key(index: u32) -> String {
    format!("checked:{index}")
}

fn checked_topology(
    this: &Approximate,
    model: &catallax_topology::Model,
    body: catallax_topology::BodyId,
) -> KernelResult<Topology> {
    let options = catallax_mesh::MeshOptions::default();
    let chord = options.chord_tolerance.to_f64_nearest();
    let sample = this.sample(&options)?;
    let ranges: BTreeMap<_, _> = sample.faces.iter().map(|face| (face.face.as_str(), &face.triangles)).collect();
    let mut adjacency: BTreeMap<catallax_topology::EdgeId, Vec<FaceId>> = BTreeMap::new();
    let mut faces = Vec::new();
    for native in model.faces_of_body(body) {
        let key = checked_key(native.index());
        let id = this.faces[&key];
        let range = ranges.get(key.as_str()).ok_or_else(|| crate::gap("Display of an imported face"))?;
        let triangles: Vec<_> = sample.triangles[(*range).clone()]
            .iter()
            .map(|triangle| triangle.vertices.map(|vertex| Vec3::from(sample.positions[vertex])))
            .collect();
        let (area, centroid) = area_centroid(&triangles);
        if area <= 0.0 {
            return Err(KernelError::InvalidInput("One of the healed faces could not be displayed.".into()));
        }
        let face = model.face(native);
        let mut edges = Vec::new();
        for boundary in &face.loops {
            if let catallax_topology::Loop::Coedges(coedges) = model.loop_(*boundary) {
                for coedge in coedges {
                    let edge = model.coedge(*coedge).edge;
                    let edge_id = this.edges[&checked_key(edge.index())];
                    if !edges.contains(&edge_id) {
                        edges.push(edge_id);
                    }
                    let adjacent = adjacency.entry(edge).or_default();
                    if !adjacent.contains(&id) {
                        adjacent.push(id);
                    }
                }
            }
        }
        let surface = model.geom().surface(face.surface);
        let (origin, normal) =
            convert::outward_plane(surface, face.sense).map_err(|error| crate::query_error("face_plane", error))?;
        let centroid = match (origin, normal) {
            (Some(origin), Some(normal)) => centroid + normal * (origin.dot(normal) - centroid.dot(normal)),
            _ => centroid,
        };
        faces.push(FaceInfo {
            id,
            surface: convert::surface_kind(surface),
            normal,
            accuracy: FaceAccuracy::Unknown,
            area,
            centroid,
            bbox: bounds(triangles.iter().flatten().copied(), chord),
            edges,
            origins: this.face_origins[&id].clone(),
        });
    }
    let polylines: BTreeMap<_, _> = sample.edges.iter().map(|edge| (edge.edge.as_str(), edge)).collect();
    let mut edges = Vec::new();
    for native in model.edges_of_body(body) {
        let key = checked_key(native.index());
        let polyline = polylines.get(key.as_str()).ok_or_else(|| crate::gap("Display of a healed edge"))?;
        let points: Vec<_> = polyline.vertices.iter().map(|vertex| Vec3::from(sample.positions[*vertex])).collect();
        let length = points.windows(2).map(|points| points[0].distance(points[1])).sum();
        let edge = model.edge(native);
        let curve = match model.geom().curve(edge.curve).kind() {
            catallax_shapes::CurveKind::Line => CurveKind::Line,
            catallax_shapes::CurveKind::Circle => CurveKind::Circle,
            catallax_shapes::CurveKind::Ellipse => CurveKind::Ellipse,
            catallax_shapes::CurveKind::BSpline => CurveKind::Spline,
            _ => CurveKind::Other,
        };
        edges.push(EdgeInfo {
            id: this.edges[&key],
            curve,
            length,
            midpoint: along(&points, length / 2.0),
            bbox: bounds(points.iter().copied(), chord),
            faces: adjacency.remove(&native).unwrap_or_default(),
            construction_origins: None,
            vertices: [edge.start, edge.end]
                .into_iter()
                .flatten()
                .map(|vertex| this.vertices[&checked_key(vertex.index())])
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        });
    }
    let vertices: Vec<_> = model
        .vertices_of_body(body)
        .into_iter()
        .map(|native| {
            let id = this.vertices[&checked_key(native.index())];
            VertexInfo {
                id,
                position: model.geom().approx_point(model.vertex(native).point).into(),
                edges: edges.iter().filter(|edge| edge.vertices.contains(&id)).map(|edge| edge.id).collect(),
            }
        })
        .collect();
    Ok(Topology {
        kind: BodyKind::Solid,
        faces,
        edges,
        vertex_count: vertices.len(),
        vertices,
        bbox: bounds(sample.positions.iter().copied().map(Vec3::from), chord),
        display_chord_height: Some(chord),
    })
}

pub(super) fn export_step(opened: &OpenedStep, index: usize, options: &StepOptions) -> KernelResult<StepExport> {
    let bytes = super::on_request_allowance(crate::RequestPolicy::Export, "export_step", |dec| {
        let bytes = match opened.body(index) {
            Some(BodyView::Certified { model, body }) => {
                let mut bytes = Vec::new();
                catallax_step::write_step(model, &[body], &Default::default(), dec, &mut bytes)?;
                bytes
            }
            Some(BodyView::Tolerant { import, body }) => import.export_body(body, dec).map_err(error)?.into_bytes(),
            Some(BodyView::Numerical(body)) => catallax_step::numerical::build(
                body,
                &catallax_step::numerical::NumericalStepOptions {
                    product_name: options.product_name.clone(),
                    ..Default::default()
                },
                dec,
            )?
            .bytes()
            .to_vec(),
            _ => return Err(Refusal::new("import.not-solid", "the selected body has no admitted solid geometry")),
        };
        Ok(bytes)
    })
    .map_err(|error| crate::query_error("export_step", error))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| KernelError::InvalidInput("The imported solid could not be written as STEP.".into()))?;
    let mut counts = [0; 3];
    for statement in super::statements(&text) {
        let Some((_, definition)) = statement.split_once('=') else { continue };
        for (index, kind) in ["ADVANCED_FACE(", "EDGE_CURVE(", "VERTEX_POINT("].iter().enumerate() {
            if definition.trim_start().starts_with(kind) {
                counts[index] += 1;
            }
        }
    }
    Ok(StepExport {
        text,
        report: StepReport {
            max_deviation: NOT_ESTABLISHED.into(),
            max_deviation_mm: 0.0,
            fit_mm: 0.0,
            within_tolerance: false,
            solids: 1,
            faces: counts[0],
            edges: counts[1],
            vertices: counts[2],
            fitted_curves: 0,
            added_vertices: 0,
            added_edges: 0,
            removed_faces: 0,
            removed_edges: 0,
            warnings: Vec::new(),
            approximate: None,
            tolerant: true,
        },
    })
}

fn error(error: catallax_import::tolerant::Error) -> Refusal {
    match error {
        catallax_import::tolerant::Error::Numerical(error) => error,
        catallax_import::tolerant::Error::Source(_) => {
            Refusal::new("import.tolerant-export", "the admitted source body could not be written")
        }
    }
}
