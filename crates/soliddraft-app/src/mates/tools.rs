//! Mate commands over MCP, and what `model_summary` says of them.

use crate::assembly::{AssemblySession, tools::summary};
use crate::state::AppState;
use serde_json::{Value, json};
use soliddraft_assembly::{
    Alignment, ComponentId, EndpointSelector, Entity, FaceEndpoint, FaceSelector, GeneratorChoice, Mate, MateEdit,
    MateRefusal, SlotConstraint, SlotInput, WidthConstraint, WidthInput,
};

/// A refusal as a tool's error: typed JSON, with what it involved.
pub(crate) fn refused(r: MateRefusal) -> String {
    serde_json::to_string(&r).unwrap_or_else(|_| r.to_string())
}

fn selector(v: &Value) -> Result<EndpointSelector, String> {
    let component = v["component"].as_u64().and_then(|c| u32::try_from(c).ok()).map(ComponentId);
    let component = component.ok_or("a face names its component by id")?;
    let labels = |key: &str| -> Option<Vec<String>> {
        v.get(key)
            .filter(|l| !l.is_null())
            .map(|l| l.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect())
    };
    let (edge, vertex) = (labels("edge"), labels("vertex"));
    let face = match (v["face"].as_str(), v["feature"].as_str(), v["plane"].as_str(), edge, vertex) {
        (None, None, None, Some(edge), None) => match <[String; 2]>::try_from(edge) {
            Ok(pair) => FaceSelector::Edge(pair),
            Err(_) => return Err("an edge names the labels of the two faces it separates".into()),
        },
        (None, None, None, None, Some(vertex)) if vertex.len() >= 2 => FaceSelector::Vertex(vertex),
        (None, None, None, None, Some(_)) => {
            return Err("a vertex names the labels of the faces that meet there".into());
        }
        (None, None, Some(plane), None, None) => FaceSelector::Plane(plane.to_owned()),
        (Some(label), None, None, None, None) => FaceSelector::Label(label.to_owned()),
        (None, Some(feature), None, None, None) => {
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
        _ => {
            return Err(
                "a face gives its label, its feature, a reference plane, an edge or a vertex: one of them".into()
            );
        }
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

/// An alignment that defaults to aligned.
fn optional_alignment(v: &Value) -> Result<Alignment, String> {
    if v.is_null() { Ok(Alignment::Aligned) } else { alignment(v) }
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
        "free" => Ok(WidthInput::Free),
        "centered" => Ok(WidthInput::Centered),
        "dimension" => Ok(WidthInput::Dimension { distance: text(input, "distance")?.to_owned(), flipped }),
        "percent" => {
            let percent = input["percent"].as_f64().ok_or("percent is a number from 0 to 100")?;
            Ok(WidthInput::Percent { percent, flipped })
        }
        _ => Err("the mode is free, centered, dimension or percent".into()),
    }
}

fn slot(input: &Value) -> Result<SlotInput, String> {
    let flipped = input["flipped"].as_bool().unwrap_or(false);
    match input["mode"].as_str().unwrap_or("free") {
        "free" => Ok(SlotInput::Free),
        "center" => Ok(SlotInput::Center),
        "distance" => Ok(SlotInput::Distance { distance: text(input, "distance")?.to_owned(), flipped }),
        "percent" => {
            let percent = input["percent"].as_f64().ok_or("percent is a number from 0 to 100")?;
            Ok(SlotInput::Percent { percent, flipped })
        }
        _ => Err("the mode is free, center, distance or percent".into()),
    }
}

fn ratio(v: &Value) -> Result<[f64; 2], String> {
    match v.as_array().map(|r| r.iter().map(Value::as_f64).collect::<Vec<_>>()).as_deref() {
        Some([Some(a), Some(b)]) => Ok([*a, *b]),
        _ => Err("the ratio is two numbers: how far the first part slides while the second slides the other".into()),
    }
}

fn optional_component(v: &Value, what: &str) -> Result<Option<ComponentId>, String> {
    if v.is_null() { Ok(None) } else { component(v, what).map(Some) }
}

fn text<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    input[key].as_str().ok_or_else(|| format!("{key} is required"))
}

/// Runs the mate tools. `None` hands the call on.
pub(crate) fn run(state: &mut AppState, name: &str, input: &Value) -> Option<Result<Value, String>> {
    if name == "assembly_drag" {
        return Some(drag(state, input).map(|_| json!({"assembly": summary(state), "pending": state.assembly.session.as_ref().is_some_and(|s| s.doc.drag_pending()), "motion": state.assembly.session.as_ref().and_then(|s| s.doc.drag_report())})));
    }

    if matches!(
        name,
        "assembly_mate_path" | "assembly_mate_cam" | "assembly_mate_universal_joint" | "assembly_physical_dynamics"
    ) {
        return Some(Err(refused(MateRefusal::new(
            soliddraft_assembly::RefusalCode::UnsupportedMotion,
            format!("{name} is outside the static rigid-motion mate solver"),
        ))));
    }
    let known = matches!(
        name,
        "assembly_mate_gear"
            | "assembly_mate_rack_and_pinion"
            | "assembly_mate_screw"
            | "assembly_mate_angle"
            | "assembly_mate_limit_angle"
            | "assembly_measure_planes"
            | "assembly_mate_origin"
            | "assembly_mate_distance"
            | "assembly_mate_coincident"
            | "assembly_mate_parallel"
            | "assembly_mate_perpendicular"
            | "assembly_mate_width"
            | "assembly_mate_lock"
            | "assembly_mate_concentric"
            | "assembly_mate_tangent"
            | "assembly_mate_hinge"
            | "assembly_mate_symmetric"
            | "assembly_mate_linear_coupler"
            | "assembly_mate_limit_distance"
            | "assembly_mate_profile_center"
            | "assembly_mate_slot"
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
        "assembly_mate_gear" => {
            let ratio = if input["ratio"].is_null() { [1.0, 1.0] } else { ratio(&input["ratio"])? };
            doc.mate_gear(
                text(input, "name")?,
                [text(input, "joint_a")?, text(input, "joint_b")?],
                ratio,
                input["reverse"].as_bool().unwrap_or(false),
                strict,
            )
            .map(|id| json!({"mate":id}))
        }
        "assembly_mate_rack_and_pinion" => {
            let rack = selector(&input["rack"])?;
            let reference = optional_component(&input["reference"], "reference")?;
            let travel =
                input["travel_per_turn"].as_f64().ok_or("travel_per_turn is a positive number in millimetres")?;
            doc.mate_rack_and_pinion(
                text(input, "name")?,
                text(input, "joint")?,
                &rack,
                reference,
                travel,
                input["reverse"].as_bool().unwrap_or(false),
                strict,
            )
            .map(|id| json!({"mate":id}))
        }
        "assembly_mate_screw" => {
            let pitch = input["pitch"].as_f64().ok_or("pitch is a positive number in millimetres per turn")?;
            doc.mate_screw(
                text(input, "name")?,
                text(input, "joint")?,
                pitch,
                input["reverse"].as_bool().unwrap_or(false),
                strict,
            )
            .map(|id| json!({"mate":id}))
        }
        "assembly_mate_angle" | "assembly_mate_limit_angle" => {
            let (a, b, reference) = (selector(&input["a"])?, selector(&input["b"])?, selector(&input["reference"])?);
            let number = |key| input[key].as_f64().ok_or_else(|| format!("{key} is a signed angle in degrees"));
            if name == "assembly_mate_angle" {
                doc.mate_angle(text(input, "name")?, &a, &b, &reference, number("degrees")?, strict)
            } else {
                doc.mate_limit_angle(
                    text(input, "name")?,
                    &a,
                    &b,
                    &reference,
                    [number("min_degrees")?, number("max_degrees")?],
                    strict,
                )
            }
            .map(|id| json!({"mate":id}))
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
        "assembly_mate_concentric" => {
            let (a, b) = (selector(&input["a"])?, selector(&input["b"])?);
            let side = optional_alignment(&input["alignment"])?;
            let lock = input["lock_rotation"].as_bool().unwrap_or(false);
            doc.mate_concentric(text(input, "name")?, &a, &b, side, lock, strict).map(|id| json!({ "mate": id }))
        }
        "assembly_mate_tangent" => {
            let (a, b) = (selector(&input["a"])?, selector(&input["b"])?);
            let side = alignment(&input["alignment"])?;
            doc.mate_tangent(text(input, "name")?, &a, &b, side, strict).map(|id| json!({ "mate": id }))
        }
        "assembly_mate_hinge" => {
            let ([a0, a1], [p0, p1]) = (pair(&input["axes"], "axes")?, pair(&input["planes"], "planes")?);
            let (side, planes) =
                (optional_alignment(&input["alignment"])?, optional_alignment(&input["plane_alignment"])?);
            let limits = input
                .get("angle_limits")
                .filter(|v| !v.is_null())
                .map(|v| {
                    serde_json::from_value::<[f64; 2]>(v.clone())
                        .map_err(|_| "angle_limits is [min,max] signed degrees".to_owned())
                })
                .transpose()?;
            doc.mate_hinge_limited(text(input, "name")?, [&a0, &a1], [&p0, &p1], [side, planes], limits, strict)
                .map(|id| json!({ "mate": id }))
        }
        "assembly_mate_symmetric" => {
            let (a, b, plane) = (selector(&input["a"])?, selector(&input["b"])?, selector(&input["plane"])?);
            let side = optional_alignment(&input["alignment"])?;
            doc.mate_symmetric(text(input, "name")?, [&a, &b, &plane], side, strict).map(|id| json!({ "mate": id }))
        }
        "assembly_mate_linear_coupler" => {
            let (a, b) = (selector(&input["a"])?, selector(&input["b"])?);
            let references = [
                optional_component(&input["reference_a"], "reference_a")?,
                optional_component(&input["reference_b"], "reference_b")?,
            ];
            let ratio = if input["ratio"].is_null() { [1.0, 1.0] } else { ratio(&input["ratio"])? };
            let reverse = input["reverse"].as_bool().unwrap_or(false);
            doc.mate_linear_coupler(text(input, "name")?, &a, &b, references, ratio, reverse, strict)
                .map(|id| json!({ "mate": id }))
        }
        "assembly_mate_limit_distance" => {
            let (a, b) = (selector(&input["a"])?, selector(&input["b"])?);
            let side = alignment(&input["alignment"])?;
            let limits = [text(input, "min")?, text(input, "max")?];
            doc.mate_limit_distance(text(input, "name")?, &a, &b, limits, side, strict).map(|id| json!({ "mate": id }))
        }
        "assembly_mate_profile_center" => {
            let (a, b) = (selector(&input["a"])?, selector(&input["b"])?);
            let side = alignment(&input["alignment"])?;
            let distance = input["distance"].as_str().unwrap_or("0 mm");
            let lock = input["lock_rotation"].as_bool().unwrap_or(false);
            doc.mate_profile_center(text(input, "name")?, &a, &b, side, distance, lock, strict)
                .map(|id| json!({ "mate": id }))
        }
        "assembly_mate_slot" => {
            let (at, pin) = (selector(&input["slot"])?, selector(&input["pin"])?);
            let side = optional_alignment(&input["alignment"])?;
            let lock = input["lock_rotation"].as_bool().unwrap_or(false);
            doc.mate_slot(text(input, "name")?, &at, &pin, side, &slot(input)?, lock, strict)
                .map(|id| json!({ "mate": id }))
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
            if let Some(v) = input["degrees"].as_f64() {
                edits.push((mate.clone(), MateEdit::Degrees(v)));
            }
            if !input["angle_limits"].is_null() {
                let v = input["angle_limits"].as_array().ok_or("angle_limits is [min_degrees, max_degrees]")?;
                if v.len() != 2 {
                    return Err("angle_limits is [min_degrees, max_degrees]".into());
                }
                let min = v[0].as_f64().ok_or("least degrees must be a number")?;
                let max = v[1].as_f64().ok_or("greatest degrees must be a number")?;
                edits.push((mate.clone(), MateEdit::AngleLimits([min, max])));
            }
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
            if !input["plane_alignment"].is_null() {
                edits.push((mate.clone(), MateEdit::PlaneAlignment(alignment(&input["plane_alignment"])?)));
            }
            if let Some(on) = input["lock_rotation"].as_bool() {
                edits.push((mate.clone(), MateEdit::LockRotation(on)));
            }
            if let Some(d) = input["min"].as_str() {
                edits.push((mate.clone(), MateEdit::Min(d.to_owned())));
            }
            if let Some(d) = input["max"].as_str() {
                edits.push((mate.clone(), MateEdit::Max(d.to_owned())));
            }
            if let Some(pitch) = input["pitch"].as_f64() {
                edits.push((mate.clone(), MateEdit::Pitch(pitch)));
            }
            if !input["ratio"].is_null() {
                edits.push((mate.clone(), MateEdit::Ratio(ratio(&input["ratio"])?)));
            }
            if let Some(on) = input["reverse"].as_bool() {
                edits.push((mate.clone(), MateEdit::Reverse(on)));
            }
            if edits.is_empty() {
                return Err(
                    "name what to change: distance, alignment, align_axes, percent, flipped, plane_alignment, lock_rotation, min, max, ratio or reverse"
                        .into(),
                );
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
        Entity::Edge(edge) => out["edge"] = json!(edge),
        Entity::Vertex(vertex) => out["vertex"] = json!(vertex),
    }
    if let Some(support) = &e.support {
        out["support"] = json!(support);
    }
    out
}

/// `model_summary`'s `mates`, `values` and `solver`.
pub(crate) fn rows(s: &mut AssemblySession) -> (Value, Value, Value) {
    let bounded = s.doc.solver().bounded;
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
            if let Some(p)=bounded.iter().find(|p|p.mate==m.name()) {row["proof"]=json!(p);}
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
                Mate::Angle {a,b,reference,degrees,..} => {
                    row["a"]=endpoint(root,a);row["b"]=endpoint(root,b);row["reference"]=endpoint(root,reference);row["degrees"]=json!(degrees);
                }
                Mate::LimitAngle {a,b,reference,min_degrees,max_degrees,..} => {
                    row["a"]=endpoint(root,a);row["b"]=endpoint(root,b);row["reference"]=endpoint(root,reference);row["min_degrees"]=json!(min_degrees);row["max_degrees"]=json!(max_degrees);
                }
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
                Mate::Coincident { a, b, .. }
                | Mate::Parallel { a, b, .. }
                | Mate::Perpendicular { a, b, .. }
                | Mate::Tangent { a, b, .. } => {
                    row["a"] = endpoint(root, a);
                    row["b"] = endpoint(root, b);
                }
                Mate::Concentric { a, b, lock_rotation, .. } => {
                    row["a"] = endpoint(root, a);
                    row["b"] = endpoint(root, b);
                    row["lock_rotation"] = json!(lock_rotation.is_some());
                    if let Some(turn) = lock_rotation {
                        row["locked_turn"] = json!(turn);
                    }
                }
                Mate::Symmetric { a, b, plane, .. } => {
                    row["a"] = endpoint(root, a);
                    row["b"] = endpoint(root, b);
                    row["plane"] = endpoint(root, plane);
                }
                Mate::LinearCoupler { a, b, reference_a, reference_b, ratio, reverse, offset, .. } => {
                    row["a"] = endpoint(root, a);
                    row["b"] = endpoint(root, b);
                    let part = |r: &soliddraft_assembly::PartEndpoint| json!({ "root": root, "path": r.path, "document": r.document });
                    row["reference_a"] = reference_a.as_ref().map_or(Value::Null, part);
                    row["reference_b"] = reference_b.as_ref().map_or(Value::Null, part);
                    row["ratio"] = json!(ratio);
                    row["reverse"] = json!(reverse);
                    row["offset"] = json!(offset);
                }
                Mate::LimitDistance { a, b, min, max, .. } => {
                    row["a"] = endpoint(root, a);
                    row["b"] = endpoint(root, b);
                    row["min"] = distance(min);
                    row["max"] = distance(max);
                }
                Mate::ProfileCenter { a, b, distance: d, lock_rotation, .. } => {
                    row["a"] = endpoint(root, a);
                    row["b"] = endpoint(root, b);
                    row["distance"] = distance(d);
                    row["lock_rotation"] = json!(lock_rotation.is_some());
                    if let Some(turn) = lock_rotation {
                        row["locked_turn"] = json!(turn);
                    }
                }
                Mate::Slot { slot, pin, constraint, lock_rotation, .. } => {
                    row["slot"] = endpoint(root, slot);
                    row["pin"] = endpoint(root, pin);
                    row["position"] = match constraint {
                        SlotConstraint::Free => json!({ "mode": "free" }),
                        SlotConstraint::Center => json!({ "mode": "center" }),
                        SlotConstraint::Distance { distance: d, flipped } => {
                            json!({ "mode": "distance", "distance": distance(d), "flipped": flipped })
                        }
                        SlotConstraint::Percent { percent, flipped } => {
                            json!({ "mode": "percent", "percent": percent, "flipped": flipped })
                        }
                    };
                    row["lock_rotation"] = json!(lock_rotation.is_some());
                    if let Some(turn) = lock_rotation {
                        row["locked_turn"] = json!(turn);
                    }
                }
                Mate::Mechanical { mechanism, joints, factor, reverse, offset, angles, reference, .. } => {
                    row["mechanism"] = json!(mechanism);
                    row["joints"] = json!(joints);
                    row["factor"] = json!(factor);
                    row["reverse"] = json!(reverse);
                    row["offset"] = json!(offset);
                    row["angles"] = json!(angles);
                    row["reference"] = json!(reference.as_ref().map(|r| r.path.to_string()));
                }
                Mate::Hinge { axes, planes, plane_alignment, angle_limits, .. } => {
                    row["angle_limits"] = json!(angle_limits);
                    row["axes"] = json!(axes.iter().map(|e| endpoint(root, e)).collect::<Vec<_>>());
                    row["planes"] = json!(planes.iter().map(|e| endpoint(root, e)).collect::<Vec<_>>());
                    row["plane_alignment"] = json!(plane_alignment);
                }
                Mate::Width { groove, tab, constraint, .. } => {
                    row["groove"] = json!(groove.iter().map(|e| endpoint(root, e)).collect::<Vec<_>>());
                    row["tab"] = json!(tab.iter().map(|e| endpoint(root, e)).collect::<Vec<_>>());
                    row["width"] = match constraint {
                        WidthConstraint::Free => json!({ "mode": "free" }),
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

fn drag(state: &mut AppState, input: &Value) -> Result<(), String> {
    let s = state.assembly.session.as_mut().ok_or("no assembly is open")?;
    s.doc.poll_drag();
    let guarded = input["collision"].as_bool().unwrap_or(false) || !input["clearance"].is_null();
    let clearance = input["clearance"].as_str().map(crate::interference::tools::length).transpose()?;
    match input["phase"].as_str().unwrap_or("move") {
        "begin" => {
            let path =
                serde_json::from_value(input["path"].clone()).map_err(|e| format!("the occurrence path: {e}"))?;
            if guarded { s.doc.begin_guarded_drag(path, clearance) } else { s.doc.begin_drag(path) }
                .map_err(|e| e.to_string())
        }
        "update" => {
            let target =
                serde_json::from_value(input["placement"].clone()).map_err(|e| format!("the placement: {e}"))?;
            s.doc.preview_drag(target).map_err(refused)
        }
        "poll" => Ok(()),
        "commit" => s.doc.commit_drag().map_err(refused),
        "cancel" => {
            s.doc.cancel_drag();
            Ok(())
        }
        "move" => {
            let path =
                serde_json::from_value(input["path"].clone()).map_err(|e| format!("the occurrence path: {e}"))?;
            let target =
                serde_json::from_value(input["placement"].clone()).map_err(|e| format!("the placement: {e}"))?;
            if guarded { s.doc.begin_guarded_drag(path, clearance) } else { s.doc.begin_drag(path) }
                .map_err(|e| e.to_string())?;
            if let Err(e) = s.doc.preview_drag(target) {
                s.doc.cancel_drag();
                return Err(refused(e));
            }
            s.doc.commit_drag().map_err(refused)
        }
        _ => Err("phase is begin, update, poll, commit, cancel or move".into()),
    }
}
