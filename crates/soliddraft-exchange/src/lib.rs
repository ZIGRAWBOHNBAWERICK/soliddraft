//! Part export: STEP AP242 and IGES from exact geometry, STL, 3MF and OBJ from meshes.
//!
//! Every export returns the file and a report of how far the file may lie from the exact
//! body. A STEP report carries the kernel's proven deviation. A mesh report carries the
//! chord height the mesh was made at, which the kernel guarantees after rounding to the
//! mesh's 32-bit coordinates, and every requested control the kernel could not guarantee.
//!
//! See `docs/exchange.md`.

pub mod mesh;
mod obj;
mod stl;
mod three_mf;

use std::path::Path;

use serde::Serialize;
use soliddraft_kernel_api::{BodyId, Kernel, KernelError, StepOptions, StepReport};
use soliddraft_tessellation::ExportTolerance;

pub use mesh::Welded;

/// A file format SolidDraft exports parts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// STEP AP242, exact geometry.
    Step,
    /// IGES 5.3, supported exact surfaces with a whole-boundary certificate.
    Iges,
    /// Binary STL, triangles.
    Stl,
    /// 3MF core, one closed mesh in millimetres.
    ThreeMf,
    /// Wavefront OBJ, triangles with face groups and vertex normals.
    Obj,
    /// STEP AP242 mass stand-in of a terminal field: the field's source envelope as a
    /// massless surface and unit-density proxy solids. It is never the field's boundary
    /// and is only written when asked for by name.
    StandIn,
}

impl Format {
    /// Every format a file extension picks, in menu order. A mass stand-in is asked for
    /// by name, never by extension.
    pub const ALL: [Format; 5] = [Format::Step, Format::Iges, Format::Stl, Format::ThreeMf, Format::Obj];

    /// The file extension written, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Step | Format::StandIn => "step",
            Format::Iges => "iges",
            Format::Stl => "stl",
            Format::ThreeMf => "3mf",
            Format::Obj => "obj",
        }
    }

    /// The format's name for people.
    pub fn name(self) -> &'static str {
        match self {
            Format::Step => "STEP AP242",
            Format::Iges => "IGES 5.3",
            Format::Stl => "STL",
            Format::ThreeMf => "3MF",
            Format::Obj => "OBJ",
            Format::StandIn => "STEP mass stand-in",
        }
    }

    /// Whether `path` has an extension this format is written under, ignoring case.
    pub fn suits(self, path: &Path) -> bool {
        Format::of_path(path).is_some_and(|f| f.extension() == self.extension())
    }

    /// The format a path's extension names, ignoring case. `.stp` is STEP too.
    pub fn of_path(path: &Path) -> Option<Format> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "step" | "stp" => Some(Format::Step),
            "iges" | "igs" => Some(Format::Iges),
            "stl" => Some(Format::Stl),
            "3mf" => Some(Format::ThreeMf),
            "obj" => Some(Format::Obj),
            _ => None,
        }
    }
}

/// How far an exported mesh may lie from the exact body.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MeshReport {
    /// Triangles written.
    pub triangles: usize,
    /// Vertices in the indexed mesh, shared across faces except where OBJ keeps normals separate.
    pub vertices: usize,
    /// The kernel's bound, in millimetres, on the distance between the mesh and the exact
    /// surface, after rounding to the file's 32-bit coordinates.
    pub chord_height: f64,
    /// True when every edge joins exactly two triangles that run it in opposite
    /// directions, so the mesh bounds a volume.
    pub closed: bool,
    /// Requested controls the kernel could not guarantee, in words.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unavailable_controls: Vec<String>,
}

/// What an export wrote, by kind.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Report {
    /// Exact geometry, with the kernel's proven deviation.
    Step(StepReport),
    /// Supported exact surfaces with a proven whole-boundary deviation.
    Iges(soliddraft_kernel_api::IgesReport),
    /// A mesh, with its chord height.
    Mesh(MeshReport),
    /// A certified field approximation with a labelled file.
    FieldMesh(soliddraft_kernel_api::FieldReport),
    /// A field's explicit mass stand-in, not its boundary.
    StandIn(Box<soliddraft_kernel_api::StandInReport>),
}

impl Report {
    /// One line for a status bar or a terminal.
    pub fn summary(&self) -> String {
        match self {
            Report::Step(r) => {
                // A fitted surface's distance from its exact definition adds to the file's own.
                let bound = if r.fit_mm > 0.0 { (r.max_deviation_mm + r.fit_mm).next_up() } else { r.max_deviation_mm };
                let mut s = format!(
                    "{} faces, {} edges; within {} mm of the exact part (proven)",
                    r.faces,
                    r.edges,
                    bound_mm(bound)
                );
                if !r.within_tolerance {
                    s += "; over the requested tolerance";
                }
                s
            }
            Report::Iges(r) => format!(
                "{} faces, {} edges; within {} mm of the exact part (proven)",
                r.faces,
                r.edges,
                bound_mm(r.max_deviation_mm)
            ),
            Report::FieldMesh(r) => {
                format!(
                    "{} triangles; approximate field mesh, proven within {} mm",
                    r.triangles,
                    bound_mm(r.deviation_bound_mm)
                )
            }
            Report::StandIn(r) => format!(
                "{} unit-density proxy solids and the massless source envelope; {}, within the field's proven moments",
                r.mass_bodies, r.label
            ),
            Report::Mesh(r) => {
                let mut s = format!("{} triangles; within {} mm of the exact part", r.triangles, r.chord_height);
                if !r.closed {
                    s += "; the mesh is not closed";
                }
                s
            }
        }
    }
}

/// An exported file and its report.
#[derive(Clone, Debug)]
pub struct Export {
    /// The file's bytes.
    pub bytes: Vec<u8>,
    /// What was written.
    pub report: Report,
}

/// Why an export failed. A failed export writes nothing.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// The kernel refused the export or its mesh.
    #[error(transparent)]
    Kernel(#[from] KernelError),
    /// The export settings are out of range.
    #[error("{0}")]
    Invalid(String),
    /// 3MF needs a closed mesh and the kernel's mesh is not.
    #[error("3MF needs a closed mesh; this one has open or doubled edges")]
    NotClosed,
    /// Writing the package failed.
    #[error("cannot write the package: {0}")]
    Package(String),
}

/// Exports `body` as `format`. `name` is written where the format supports it; meshes use
/// `tolerance`, the document's export setting.
pub fn export(
    kernel: &dyn Kernel,
    body: BodyId,
    name: &str,
    format: Format,
    tolerance: &ExportTolerance,
) -> Result<Export, ExportError> {
    match format {
        Format::Step => {
            let options = StepOptions { product_name: name.to_owned(), ..StepOptions::default() };
            let step = kernel.export_step(body, &options)?;
            Ok(Export { bytes: step.text.into_bytes(), report: Report::Step(step.report) })
        }
        Format::Iges => {
            let iges = kernel.export_iges(body, &soliddraft_kernel_api::IgesOptions::default())?;
            Ok(Export { bytes: iges.text.into_bytes(), report: Report::Iges(iges.report) })
        }
        Format::StandIn => {
            Err(ExportError::Invalid("a mass stand-in comes from a part's active lattice or gyroid".into()))
        }
        Format::Stl | Format::ThreeMf | Format::Obj => {
            tolerance.check().map_err(ExportError::Invalid)?;
            let mesh = kernel.tessellate(body, &tolerance.params())?;
            let welded = mesh::weld(&mesh);
            let closed = welded.is_closed();
            let bytes = match format {
                Format::Obj => obj::write(&mesh, name, tolerance.chord_height),
                Format::Stl => stl::write(&welded, name),
                Format::ThreeMf if closed => three_mf::write(&welded, name)?,
                Format::ThreeMf => return Err(ExportError::NotClosed),
                Format::Step | Format::Iges | Format::StandIn => unreachable!(),
            };
            let report = MeshReport {
                triangles: welded.triangles.len(),
                vertices: if format == Format::Obj { mesh.positions.len() } else { welded.positions.len() },
                chord_height: tolerance.chord_height,
                closed,
                unavailable_controls: mesh.unavailable_controls,
            };
            Ok(Export { bytes, report: Report::Mesh(report) })
        }
    }
}

/// A proven bound for people: rounded up to the micrometre's thousandth, so the shown
/// figure is never smaller than the proof. A bound below that reads as the smallest step.
///
/// The rounding works on the bound's exact decimal value, so a binary64 that sits just above
/// a step, such as `0.01` itself, reads as the next step rather than as the step below it.
fn bound_mm(bound: f64) -> String {
    const DECIMALS: usize = 6;
    if !bound.is_finite() || bound <= 0.0 {
        return if bound == 0.0 { "0".into() } else { bound.to_string() };
    }
    let (whole, fraction) = exact_decimal(bound);
    let (kept, rest) = fraction.split_at(DECIMALS);
    let mut digits: Vec<u8> = whole.bytes().chain(kept.bytes()).map(|b| b - b'0').collect();
    if rest.bytes().any(|b| b != b'0') {
        let mut i = digits.len();
        loop {
            if i == 0 {
                digits.insert(0, 1);
                break;
            }
            i -= 1;
            if digits[i] == 9 {
                digits[i] = 0;
            } else {
                digits[i] += 1;
                break;
            }
        }
    }
    let digits: String = digits.iter().map(|d| char::from(b'0' + d)).collect();
    let (whole, fraction) = digits.split_at(digits.len() - DECIMALS);
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() { whole.to_owned() } else { format!("{whole}.{fraction}") }
}

/// The exact decimal value of a finite, non-negative `value`: its whole part and every
/// fractional digit. A binary64 has at most 1074 fractional digits, and Rust prints the
/// requested precision exactly, so nothing here is rounded.
fn exact_decimal(value: f64) -> (String, String) {
    let text = format!("{value:.1074}");
    let (whole, fraction) = text.split_once('.').expect("a fixed-precision float has a point");
    (whole.to_owned(), fraction.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(max_deviation_mm: f64, fit_mm: f64) -> Report {
        Report::Step(StepReport {
            max_deviation: max_deviation_mm.to_string(),
            max_deviation_mm,
            fit_mm,
            within_tolerance: true,
            solids: 1,
            faces: 7,
            edges: 15,
            vertices: 10,
            fitted_curves: 0,
            added_vertices: 0,
            added_edges: 0,
            removed_faces: 0,
            removed_edges: 0,
            warnings: Vec::new(),
        })
    }

    #[test]
    fn a_proven_bound_reads_as_a_short_number_never_below_the_proof() {
        // The tray of the real-use pass printed every digit of its binary64 bound.
        assert_eq!(
            step(1.3844827940065006e-16, 0.0).summary(),
            "7 faces, 15 edges; within 0.000001 mm of the exact part (proven)"
        );
        // The ring fillet's fitted band: 0.0000188... rounds up, not to the nearest.
        assert_eq!(
            step(0.0, 0.000018850358009776183).summary(),
            "7 faces, 15 edges; within 0.000019 mm of the exact part (proven)"
        );
        assert!(step(0.0, 0.0).summary().contains("within 0 mm"));
        // The binary64 nearest 0.01 is a little above it, so it reads as the next step.
        assert_eq!(bound_mm(0.01), "0.010001");
        assert_eq!(bound_mm(0.0123451), "0.012346");
        assert_eq!(bound_mm(2.5), "2.5");
    }

    /// Whether the decimal `shown` is at or above the exact value of `bound`.
    fn not_below(shown: &str, bound: f64) -> bool {
        let (whole, fraction) = shown.split_once('.').unwrap_or((shown, ""));
        let (bound_whole, bound_fraction) = exact_decimal(bound);
        let fraction = format!("{fraction:0<1074}");
        (whole.len(), whole, fraction.as_str()) >= (bound_whole.len(), bound_whole.as_str(), bound_fraction.as_str())
    }

    #[test]
    fn a_shown_bound_is_never_below_the_exact_binary_value() {
        let cases: &[(f64, &str)] = &[
            // The review's case: within 1e-12 of 0.01 relatively, but above it.
            (0.010000000000005, "0.010001"),
            // Exact binary fractions with six decimals or fewer read as themselves.
            (0.5, "0.5"),
            (0.015625, "0.015625"),
            (3.0, "3"),
            // Either side of a step: 0.000002 in binary64 is just below 2e-6, so it reads as
            // 2e-6; one ulp up reads as the next step.
            (0.000002, "0.000002"),
            (0.000002f64.next_up(), "0.000003"),
            (0.000002f64.next_down(), "0.000002"),
            (0.123456, "0.123456"),
            (0.123456f64.next_up(), "0.123457"),
            (0.123456f64.next_down(), "0.123456"),
            (0.1234565, "0.123457"),
            // A carry runs through every digit.
            (0.9999995, "1"),
            (9.9999999, "10"),
            // Very small bounds read as the smallest step.
            (f64::MIN_POSITIVE, "0.000001"),
            (5e-324, "0.000001"),
            (1e-7, "0.000001"),
            // Very large bounds keep their whole digits.
            (1e15, "1000000000000000"),
            (123456789.123, "123456789.123"),
            (123456789.125, "123456789.125"),
        ];
        for &(bound, expected) in cases {
            let shown = bound_mm(bound);
            assert_eq!(shown, expected, "{bound:e}");
            assert!(not_below(&shown, bound), "{shown} is below {bound:e}");
        }
        let huge = bound_mm(f64::MAX);
        assert!(not_below(&huge, f64::MAX) && huge.len() == 309, "{huge}");
    }

    #[test]
    fn a_fitted_step_bound_keeps_its_margin_when_shown() {
        // The sum is rounded and then moved one ulp up; that ulp must survive the display.
        for (deviation, fit) in [(0.005_f64, 0.005_f64), (0.0000005, 0.0000005), (0.25, 0.25), (1e-17, 0.000018)] {
            let bound = (deviation + fit).next_up();
            let summary = step(deviation, fit).summary();
            let shown = summary.split("within ").nth(1).and_then(|s| s.split(' ').next()).unwrap();
            assert!(not_below(shown, bound), "{shown} is below {bound:e}");
            assert!(not_below(shown, deviation) && not_below(shown, fit), "{summary}");
        }
        assert!(step(0.005, 0.005).summary().contains("within 0.010001 mm"));
        assert!(step(0.25, 0.25).summary().contains("within 0.500001 mm"));
    }
}
