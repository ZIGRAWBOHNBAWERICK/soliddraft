//! Open scan admission, classic fitting and replay of the retained native recipe.

use catallax_import::scan::ScanReference;
use catallax_numbers::{Decider, Rational, Refusal};
use catallax_recipe::{FileRef, Number, Operation, Pose};
use serde_json::Value;
use soliddraft_kernel_api::{KernelError, KernelResult, scan as api};
use std::collections::BTreeMap;

fn refused(r: Refusal) -> KernelError {
    let text = match r.code {
        "import.scan-too-large" => "A scan can contain up to 200,000 triangles or 1,000,000 points.".into(),
        "import.mesh-malformed" => "This file could not be read as an STL or OBJ scan.".into(),
        "reconstruction.too-few-samples" => "Fit scan to part needs at least 100 points.".into(),
        "reconstruction.too-many-samples" => "Fit scan to part accepts up to 1,000,000 points.".into(),
        "reconstruction.no-stock" => "The scan needs six stock faces in three perpendicular pairs. Fit supports block stock, bosses, bores and convex stock-edge rounds.".into(),
        "reconstruction.deviation-too-large" => "The deviation map would exceed 1,000,000 samples. Use a smaller scan or part.".into(),
        "numbers.operation-budget" | "numbers.budget-exhausted" | "numbers.deadline" => "This scan could not be fitted within the available work limit. Try a smaller scan.".into(),
        "reconstruction.nonfinite-sample" => "The scan coordinates are too large to fit.".into(),
        "reconstruction.nonfinite-deviation" => "The scan distances could not be measured as finite numbers.".into(),
        "reconstruction.recipe" | "reconstruction.invalid-part" => "The fitted surfaces could not make a valid solid. The scan reference is retained.".into(),
        "reconstruction.unexplained-region" | "reconstruction.unplaced-surface" => r.detail,
        _ => format!("The scan could not be fitted: {}", r.detail),
    };
    KernelError::InvalidInput(text)
}

fn step(source: &api::ScanSource) -> KernelResult<Operation> {
    if source.bytes.is_empty() || catallax_import::sha256_hex(&source.bytes) != source.sha256 {
        return Err(KernelError::InvalidInput("The saved scan does not match its original file.".into()));
    }
    Ok(Operation::ScanImport {
        file: FileRef { name: source.name.clone(), sha256: source.sha256.clone() },
        pose: Pose::identity(),
        scale: Number::new(Rational::one()),
    })
}

pub(super) fn reference(source: &api::ScanSource) -> KernelResult<api::ScanReference> {
    let scan = ScanReference::from_step(&step(source)?, &source.bytes, &mut Decider::default()).map_err(refused)?;
    if scan.approx_points().iter().flatten().any(|v| !v.is_finite() || !(*v as f32).is_finite()) {
        return Err(KernelError::InvalidInput("The scan coordinates are too large to display.".into()));
    }
    let r = scan.report();
    Ok(api::ScanReference {
        points: scan.approx_points().to_vec(),
        triangles: scan.triangles().to_vec(),
        triangle_count: r.triangles,
        boundary_edges: r.boundary_edges,
        inconsistent_edges: r.inconsistent_edges,
        nonmanifold_edges: r.nonmanifold_edges,
        isolated_points: r.isolated_points,
    })
}

pub(super) fn fit(source: &api::ScanSource) -> KernelResult<api::ScanFit> {
    use catallax_field::reconstruction::scan::{ScanFitOptions, fit_scan};
    let mut dec = Decider::default();
    let result =
        fit_scan(&step(source)?, &source.bytes, &ScanFitOptions { workers: 1, ..Default::default() }, &mut dec)
            .map_err(refused)?;
    let part = result.part.map_err(refused)?;
    let side = |s: catallax_field::reconstruction::scan::DeviationSide| api::ScanDeviationSide {
        samples: s.samples,
        max_mm: s.max_mm,
        rms_mm: s.rms_mm,
        p95_mm: s.p95_mm,
    };
    let d = part.deviation;
    Ok(api::ScanFit {
        recipe: serde_json::to_value(part.recipe).map_err(|e| KernelError::InvalidInput(e.to_string()))?,
        scan_to_part: side(d.scan_to_part),
        part_to_scan: side(d.part_to_scan),
        chord_mm: d.chord_mm,
        scan_map: result
            .samples
            .into_iter()
            .zip(d.scan_map)
            .map(|(point, distance_mm)| api::ScanMapPoint { point, distance_mm })
            .collect(),
        part_map: d
            .part_map
            .into_iter()
            .map(|(point, distance_mm, _)| api::ScanMapPoint { point, distance_mm })
            .collect(),
        warnings: result.refusals.into_iter().map(|r| r.detail).collect(),
    })
}

pub(super) fn validate(source: &api::ScanSource, fit: &api::ScanFit) -> KernelResult<()> {
    let expected = step(source)?;
    let recipe = catallax_recipe::Recipe::parse(&fit.recipe.to_string())
        .map_err(|_| KernelError::InvalidInput("The saved fitted part could not be read.".into()))?;
    if recipe.steps.first().is_none_or(|s| s.op != expected) {
        return Err(KernelError::InvalidInput("The fitted part belongs to a different scan.".into()));
    }
    Ok(())
}

/// Expands one retained fit into ordinary recipe steps, with names unique to its
/// application operation. All generated origins belong to the scan feature.
pub(super) fn expand(
    converted: &mut catallax_app::soliddraft::Converted,
    tag: &str,
    mut recipe: Value,
) -> KernelResult<()> {
    let profile = converted.steps.remove(&(tag.to_owned() + "_fit_profile")).expect("fit profile");
    converted.recipe.steps.retain(|s| s.id != profile);
    let result = converted.steps[tag].clone();
    let old_result = recipe["result"]
        .as_str()
        .ok_or_else(|| KernelError::InvalidInput("The fitted part has no result.".into()))?
        .to_owned();
    let steps =
        recipe["steps"].as_array().ok_or_else(|| KernelError::InvalidInput("The fitted part has no steps.".into()))?;
    let mut names = BTreeMap::new();
    for (i, s) in steps.iter().enumerate() {
        let old =
            s["id"].as_str().ok_or_else(|| KernelError::InvalidInput("The fitted part has an unnamed step.".into()))?;
        let new = if old == old_result { result.as_str().to_owned() } else { format!("{}_scan_{i}", result.as_str()) };
        names.insert(old.to_owned(), new);
    }
    fn rename(v: &mut Value, names: &BTreeMap<String, String>) {
        match v {
            Value::String(s) => {
                if let Some(new) = names.get(s) {
                    *s = new.clone();
                }
            }
            Value::Array(a) => {
                for v in a {
                    rename(v, names);
                }
            }
            Value::Object(o) => {
                for v in o.values_mut() {
                    rename(v, names);
                }
            }
            _ => {}
        }
    }
    rename(&mut recipe, &names);
    let fitted =
        catallax_recipe::Recipe::parse(&recipe.to_string()).map_err(|e| KernelError::InvalidInput(e.to_string()))?;
    let index = converted.recipe.steps.iter().position(|s| s.id == result).expect("fit result");
    for (i, s) in fitted.steps.iter().enumerate() {
        converted.steps.insert(format!("{tag}_scan_{i}"), s.id.clone());
    }
    converted.recipe.steps.splice(index..=index, fitted.steps);
    Ok(())
}
