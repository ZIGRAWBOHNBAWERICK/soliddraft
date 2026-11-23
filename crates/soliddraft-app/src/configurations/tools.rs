//! The configuration and exploded view tools.

use crate::assembly::tools::summary;
use crate::state::AppState;
use glam::DVec3;
use serde_json::{Value, json};
use soliddraft_assembly::{AssemblyDocument, OccurrencePath};

/// The configurations and the exploded view, as `model_summary` and these tools give them.
pub(crate) fn describe(doc: &AssemblyDocument) -> Value {
    let truth = doc.assembly();
    let configurations: Vec<Value> =
        truth.configurations.iter().map(|c| json!({ "id": c.id, "name": c.name, "active": c.is_active() })).collect();
    let steps: Vec<Value> = truth
        .explode
        .iter()
        .map(|s| {
            json!({
                "id": s.id, "paths": s.occurrences.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "direction": s.direction, "distance": s.distance,
            })
        })
        .collect();
    json!({
        "configurations": configurations,
        "kept_from_another_version": truth.configurations.reserved().len(),
        "active_configuration": truth.active_configuration().map(|c| c.name.clone()),
        "explode": { "steps": steps, "shown": doc.is_exploded() },
    })
}

/// Where each occurrence an explode step moves shows in the exploded view.
fn exploded(doc: &mut AssemblyDocument) -> Value {
    let moved: Vec<OccurrencePath> = doc.assembly().explode.iter().flat_map(|s| s.occurrences.clone()).collect();
    let rows: serde_json::Map<String, Value> = doc
        .exploded_worlds()
        .into_iter()
        .filter(|(p, _)| moved.iter().any(|m| p.starts_with(m)))
        .map(|(p, w)| (p.to_string(), json!(w.translation.to_array())))
        .collect();
    Value::Object(rows)
}

fn direction(v: &Value) -> Result<Option<DVec3>, String> {
    if v.is_null() {
        return Ok(None);
    }
    let a: Vec<f64> = v.as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
    match a[..] {
        [x, y, z] => Ok(Some(DVec3::new(x, y, z))),
        _ => Err("direction is [x, y, z]".into()),
    }
}

fn name(input: &Value, key: &str) -> Result<String, String> {
    input[key].as_str().map(str::to_owned).ok_or_else(|| format!("{key} is required"))
}

fn id_of(doc: &AssemblyDocument, name: &str) -> Result<u32, String> {
    doc.configuration_id(name).ok_or_else(|| format!("there is no configuration called {name}"))
}

/// Runs the configuration tools. `None` hands the call on.
pub(crate) fn run(state: &mut AppState, tool: &str, input: &Value) -> Option<Result<Value, String>> {
    if !matches!(
        tool,
        "assembly_configurations"
            | "assembly_explode"
            | "assembly_add_configuration"
            | "assembly_activate_configuration"
            | "assembly_rename_configuration"
            | "assembly_delete_configuration"
            | "assembly_explode_step"
            | "assembly_edit_explode_step"
            | "assembly_delete_explode_step"
            | "assembly_auto_explode"
            | "assembly_component_configuration"
    ) {
        return None;
    }
    Some(run_one(state, tool, input))
}

fn run_one(state: &mut AppState, tool: &str, input: &Value) -> Result<Value, String> {
    let s = state.assembly.session.as_mut().ok_or("no assembly is open")?;
    let doc = &mut s.doc;
    doc.finish_loading();
    let edit = |r: Result<(), soliddraft_assembly::EditError>| r.map_err(|e| e.to_string());
    let mut extra = json!({});
    match tool {
        "assembly_configurations" => {}
        "assembly_explode" => {
            let on = input["exploded"].as_bool().ok_or("exploded is true or false")?;
            edit(doc.set_exploded(on))?;
            extra = json!({ "exploded": exploded(doc) });
        }
        "assembly_add_configuration" => {
            let id = doc.add_configuration(&name(input, "name")?).map_err(|e| e.to_string())?;
            extra = json!({ "added": id });
        }
        "assembly_activate_configuration" => {
            let id = id_of(doc, &name(input, "name")?)?;
            edit(doc.activate_configuration(id))?;
        }
        "assembly_rename_configuration" => {
            let id = id_of(doc, &name(input, "name")?)?;
            edit(doc.rename_configuration(id, &name(input, "to")?))?;
        }
        "assembly_delete_configuration" => {
            let id = id_of(doc, &name(input, "name")?)?;
            edit(doc.delete_configuration(id))?;
        }
        "assembly_explode_step" => {
            let paths: Vec<OccurrencePath> = input["paths"]
                .as_array()
                .ok_or("paths lists what moves")?
                .iter()
                .map(|p| p.as_str().ok_or("a path is text such as \"4\"")?.parse().map_err(|e| format!("{e}")))
                .collect::<Result<_, String>>()?;
            let d = direction(&input["direction"])?.ok_or("direction is required")?;
            let mm = input["distance"].as_f64().ok_or("distance is a number of millimetres")?;
            let id = doc.add_explode_step(paths, d, mm).map_err(|e| e.to_string())?;
            extra = json!({ "added": id, "exploded": exploded(doc) });
        }
        "assembly_edit_explode_step" => {
            let id = input["step"].as_u64().ok_or("step is the step's id")? as u32;
            edit(doc.edit_explode_step(id, direction(&input["direction"])?, input["distance"].as_f64()))?;
            extra = json!({ "exploded": exploded(doc) });
        }
        "assembly_delete_explode_step" => {
            let id = input["step"].as_u64().ok_or("step is the step's id")? as u32;
            edit(doc.delete_explode_step(id))?;
        }
        "assembly_auto_explode" => {
            let mm = input["distance"].as_f64().unwrap_or(50.0);
            let ids = doc.auto_explode(mm).map_err(|e| e.to_string())?;
            extra = json!({ "added": ids, "exploded": exploded(doc) });
        }
        "assembly_component_configuration" => {
            let id = input["component"].as_u64().ok_or("component is the component's id")? as u32;
            let component = soliddraft_assembly::ComponentId(id);
            edit(doc.set_configuration(component, input["configuration"].as_str()))?;
            doc.finish_loading();
            let choices = doc.configurations_for(component).map_err(|e| e.to_string())?;
            let shown = doc.configuration_of(component).map_err(|e| e.to_string())?;
            extra = json!({ "component": { "id": id, "configuration": shown, "configurations": choices } });
        }
        _ => unreachable!("listed in run"),
    }
    doc.finish_loading();
    let described = describe(doc);
    let mut out = json!({ "configurations": described, "assembly": summary(state) });
    if let (Value::Object(o), Value::Object(e)) = (&mut out, extra) {
        o.extend(e);
    }
    Ok(out)
}
