//! Mate commands over MCP, and what `model_summary` says of them.

use crate::assembly::{AssemblySession, tools::summary};
use crate::state::AppState;
use serde_json::{Value, json};
use soliddraft_assembly::{
    Alignment, ComponentId, EndpointSelector, Entity, FaceEndpoint, FaceSelector, GeneratorChoice, Mate, MateEdit,
    MateRefusal, WidthConstraint, WidthInput,
};

/// A refusal as a tool's error: typed JSON, with what it involved.
pub(crate) fn refused(r: MateRefusal) -> String {
    serde_json::to_string(&r).unwrap_or_else(|_| r.to_string())
}

fn selector(v: &Value) -> Result<EndpointSelector, String> {
    let component = v["component"].as_u64().and_then(|c| u32::try_from(c).ok()).map(ComponentId);
    let component = component.ok_or("a face names its component by id")?;
    let face = match (v["face"].as_str(), v["feature"].as_str(), v["plane"].as_str()) {
        (None, None, Some(plane)) => FaceSelector::Plane(plane.to_owned()),
        (Some(label), None, None) => FaceSelector::Label(label.to_owned()),
        (None, Some(feature), None) => {
            let generator = match v["generator"].as_str() {
                Some("start_cap") => GeneratorChoice::StartCap,
                Some("end_cap") => GeneratorChoice::EndCap,
                Some("side") => GeneratorChoice::Side,
                _ => return Err("a feature's face names its generator: start_cap, end_cap or side".into()),
            };
            let normal = match v.get("normal").filter(|n| !n.is_null()) {
                None => None,
                Some(n) => {
                    let c: Vec<f64> = n.as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
                    let [x, y, z] = c[..] else { return Err("a normal is three numbers".into()) };
                    Some([x, y, z])
                }
            };
            FaceSelector::Generator { feature: feature.to_owned(), generator, normal }
        }
        _ => return Err("a face gives its label, its feature or a reference plane: one of them".into()),
    };
    Ok(EndpointSelector { component, face })
}

fn alignment(v: &Value) -> Result<Alignment, String> {
    match v.as_str() {
        Some("aligned") => Ok(Alignment::Aligned),
        Some("opposed") => Ok(Alignment::Opposed),
        _ => Err("the alignment is aligned or opposed".into()),
    }
}

fn component(v: &Value, what: &str) -> Result<ComponentId, String> {
    v.as_u64().and_then(|c| u32::try_from(c).ok()).map(ComponentId).ok_or_else(|| format!("{what} is a component id"))
}

fn pair(v: &Value, what: &str) -> Result<[EndpointSelector; 2], String> {
    match v.as_array().map(Vec::as_slice) {
        Some([a, b]) => Ok([selector(a)?, selector(b)?]),
        _ => Err(format!("{what} is two faces")),
    }
}

fn width(input: &Value) -> Result<WidthInput, String> {
    let flipped = input["flipped"].as_bool().unwrap_or(false);
    match input["mode"].as_str().unwrap_or("centered") {
        "centered" => Ok(WidthInput::Centered),
        "dimension" => Ok(WidthInput::Dimension { distance: text(input, "distance")?.to_owned(), flipped }),
        "percent" => {
            let percent = input["percent"].as_f64().ok_or("percent is a number from 0 to 100")?;
            Ok(WidthInput::Percent { percent, flipped })
        }
        _ => Err("the mode is centered, dimension or percent".into()),
    }
}

fn text<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    input[key].as_str().ok_or_else(|| format!("{key} is required"))
}

/// Runs the mate tools. `None` hands the call on.
pub(crate) fn run(state: &mut AppState, name: &str, input: &Value) -> Option<Result<Value, String>> {
    let known = matches!(
        name,
        "assembly_measure_planes"
            | "assembly_mate_origin"
            | "assembly_mate_distance"
            | "assembly_mate_coincident"
            | "assembly_mate_parallel"
            | "assembly_mate_perpendicular"
            | "assembly_mate_width"
            | "assembly_mate_lock"
            | "assembly_set_value"
            | "assembly_rename_value"
            | "assembly_drive_mate"
            | "assembly_edit_mate"
            | "assembly_delete_mate"
    );
    if !known {
        return None;
    }
    Some(call(state, name, input).map(|mut out| {
        out["assembly"] = summary(state).unwrap_or(Value::Null);
        out
    }))
}

fn call(state: &mut AppState, name: &str, input: &Value) -> Result<Value, String> {
    let s = state.assembly.session.as_mut().ok_or("no assembly is open")?;
    let strict = input["require_fully_constrained"].as_bool().unwrap_or(false);
    let doc = &mut s.doc;
    let done = match name {
        "assembly_measure_planes" => {
            let m = doc.measure_planes(&selector(&input["a"])?, &selector(&input["b"])?).map_err(refused)?;
            return Ok(json!({ "measurement": m }));
        }
        "assembly_mate_origin" => {
            let component = input["component"].as_u64().and_then(|c| u32::try_from(c).ok()).map(ComponentId);
            let component = component.ok_or("the component's id is required")?;
            let axes = input["align_axes"].as_bool().ok_or("align_axes is required")?;
            doc.mate_origin(text(input, "name")?, component, axes, strict).map(|id| json!({ "mate": id }))
        }
        "assembly_mate_distance" => {
            let (a, b) = (selector(&input["a"])?, selector(&input["b"])?);
            let side = alignment(&input["alignment"])?;
            doc.mate_distance(text(input, "name")?, &a, &b, text(input, "distance")?, side, strict)
                .map(|id| json!({ "mate": id }))
        }
        "assembly_mate_coincident" => {
            let (a, b) = (selector(&input["a"])?, selector(&input["b"])?);
            let side = alignment(&input["alignment"])?;
            doc.mate_coincident(text(input, "name")?, &a, &b, side, strict).map(|id| json!({ "mate": id }))
        }
        "assembly_mate_parallel" => {
            let (a, b) = (selector(&input["a"])?, selector(&input["b"])?);
            let side = alignment(&input["alignment"])?;
            doc.mate_parallel(text(input, "name")?, &a, &b, side, strict).map(|id| json!({ "mate": id }))
        }
        "assembly_mate_perpendicular" => {
            let (a, b) = (selector(&input["a"])?, selector(&input["b"])?);
            doc.mate_perpendicular(text(input, "name")?, &a, &b, strict).map(|id| json!({ "mate": id }))
        }
        "assembly_mate_width" => {
            let ([g0, g1], [t0, t1]) = (pair(&input["groove"], "groove")?, pair(&input["tab"], "tab")?);
            let side = alignment(&input["alignment"])?;
            let mode = width(input)?;
            doc.mate_width(text(input, "name")?, [&g0, &g1], [&t0, &t1], side, &mode, strict)
                .map(|id| json!({ "mate": id }))
        }
        "assembly_mate_lock" => {
            let (a, b) = (component(&input["a"], "a")?, component(&input["b"], "b")?);
            doc.mate_lock(text(input, "name")?, a, b, strict).map(|id| json!({ "mate": id }))
        }
        "assembly_set_value" => doc.set_value(text(input, "name")?, text(input, "expression")?).map(|()| json!({})),
        "assembly_rename_value" => doc.rename_value(text(input, "name")?, text(input, "to")?).map(|()| json!({})),
        "assembly_drive_mate" => {
            if input["parameter"].as_str() != Some("distance") {
                return Err("a mate's distance is what can be driven".into());
            }
            let replace = input["replace"].as_bool().unwrap_or(false);
            doc.drive_mate(text(input, "mate")?, text(input, "expression")?, replace).map(|()| json!({}))
        }
        "assembly_edit_mate" => {
            let mate = text(input, "mate")?.to_owned();
            let mut edits = Vec::new();
            if let Some(d) = input["distance"].as_str() {
                edits.push((mate.clone(), MateEdit::Distance(d.to_owned())));
            }
            if !input["alignment"].is_null() {
                edits.push((mate.clone(), MateEdit::Alignment(alignment(&input["alignment"])?)));
            }
            if let Some(on) = input["align_axes"].as_bool() {
                edits.push((mate.clone(), MateEdit::AlignAxes(on)));
            }
            if let Some(p) = input["percent"].as_f64() {
                edits.push((mate.clone(), MateEdit::Percent(p)));
            }
            if let Some(on) = input["flipped"].as_bool() {
                edits.push((mate.clone(), MateEdit::Flipped(on)));
            }
            if edits.is_empty() {
                return Err("name what to change: distance, alignment, align_axes, percent or flipped".into());
            }
            doc.edit_mates(&edits).map(|()| json!({}))
        }
        _ => doc.delete_mate(text(input, "mate")?).map(|()| json!({})),
    };
    done.map_err(refused)
}

/// A face endpoint as `model_summary` shows it.
fn endpoint(root: impl serde::Serialize, e: &FaceEndpoint) -> Value {
    let mut out = json!({ "root": root, "path": e.path, "document": e.document, "configuration": e.configuration });
    match &e.entity {
        Entity::Face(face) => out["face"] = json!(face),
        Entity::Plane(plane) => out["plane"] = json!(plane),
    }
    out
}

/// `model_summary`'s `mates`, `values` and `solver`.
pub(crate) fn rows(s: &mut AssemblySession) -> (Value, Value, Value) {
    let measured = s.doc.mate_measurements();
    let truth = s.doc.assembly();
    let values = truth.values.summary();
    let root = s.doc.id();
    let mates: Vec<Value> = truth
        .mates
        .iter()
        .map(|entry| {
            let Some(m) = entry.known() else {
                return json!({ "kind": "unknown", "status": "unknown_mate", "stored": entry });
            };
            let found = measured.iter().find(|(id, _)| *id == m.id()).map(|(_, r)| r);
            let mut row = json!({ "id": m.id(), "name": m.name(), "kind": m.kind() });
            let distance = |d: &soliddraft_model::Expression| {
                json!({
                    "expression": d.text(),
                    "driven": !d.references().is_empty(),
                    "reads": d.references(),
                    "value": truth.values.length(d).ok(),
                    "unit": "mm",
                })
            };
            match m {
                Mate::Origin { component, align_axes, .. } => {
                    row["component"] = json!(component);
                    row["align_axes"] = json!(align_axes);
                    row["status"] = json!("ok");
                    return row;
                }
                Mate::Lock { a, b, relative, .. } => {
                    row["a"] = json!({ "root": root, "path": a.path, "document": a.document });
                    row["b"] = json!({ "root": root, "path": b.path, "document": b.document });
                    row["relative"] = json!({
                        "translation": relative.translation.to_array(),
                        "rotation": relative.rotation.to_array(),
                    });
                    row["status"] = json!("ok");
                    return row;
                }
                Mate::Distance { distance: d, a, b, .. } => {
                    row["distance"] = distance(d);
                    row["a"] = endpoint(root, a);
                    row["b"] = endpoint(root, b);
                }
                Mate::Coincident { a, b, .. } | Mate::Parallel { a, b, .. } | Mate::Perpendicular { a, b, .. } => {
                    row["a"] = endpoint(root, a);
                    row["b"] = endpoint(root, b);
                }
                Mate::Width { groove, tab, constraint, .. } => {
                    row["groove"] = json!(groove.iter().map(|e| endpoint(root, e)).collect::<Vec<_>>());
                    row["tab"] = json!(tab.iter().map(|e| endpoint(root, e)).collect::<Vec<_>>());
                    row["width"] = match constraint {
                        WidthConstraint::Centered => json!({ "mode": "centered" }),
                        WidthConstraint::Dimension { distance: d, flipped } => {
                            json!({ "mode": "dimension", "distance": distance(d), "flipped": flipped })
                        }
                        WidthConstraint::Percent { percent, flipped } => {
                            json!({ "mode": "percent", "percent": percent, "flipped": flipped })
                        }
                    };
                }
            }
            if let Some(alignment) = m.alignment() {
                row["alignment"] = json!(alignment);
            }
            match found {
                Some(Ok(Some(measurement))) => {
                    row["measurement"] = json!(measurement);
                    row["status"] = json!("ok");
                }
                Some(Ok(None)) => row["status"] = json!("ok"),
                Some(Err(r)) => {
                    row["status"] = json!(r.code.as_str());
                    row["problem"] = json!(r.message);
                }
                None => row["status"] = json!("unmeasured"),
            }
            row
        })
        .collect();
    (Value::Array(mates), values, json!(s.doc.solver()))
}
