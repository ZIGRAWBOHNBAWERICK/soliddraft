//! `soliddraft asm config` and `soliddraft asm explode`: configurations and exploded views
//! without a window, through the same document calls the interface makes.

use glam::DVec3;
use serde_json::{Value, json};
use soliddraft_assembly::AssemblyDocument;

use super::asm::{Args, numbers, paths};

/// `asm config <file> <list | add NAME | activate NAME | rename NAME NEW | delete NAME>`.
pub(crate) fn config(doc: &mut AssemblyDocument, rest: &[String], a: &Args) -> Result<bool, String> {
    let id = |doc: &AssemblyDocument, name: &str| {
        doc.configuration_id(name).ok_or(format!("there is no configuration called {name}"))
    };
    match rest {
        [verb] if verb == "list" => {
            print_configurations(doc, a.json);
            return Ok(false);
        }
        [verb, name] if verb == "add" => {
            let made = doc.add_configuration(name).map_err(|e| e.to_string())?;
            println!("added configuration {name} ({made})");
        }
        [verb, name] if verb == "activate" => {
            let c = id(doc, name)?;
            doc.activate_configuration(c).map_err(|e| e.to_string())?;
            println!("activated {name}");
        }
        [verb, name, new] if verb == "rename" => {
            let c = id(doc, name)?;
            doc.rename_configuration(c, new).map_err(|e| e.to_string())?;
            println!("renamed {name} to {new}");
        }
        [verb, name] if verb == "delete" => {
            let c = id(doc, name)?;
            doc.delete_configuration(c).map_err(|e| e.to_string())?;
            println!("deleted {name}");
        }
        _ => return Err("config takes list, add NAME, activate NAME, rename NAME NEW or delete NAME".into()),
    }
    Ok(true)
}

/// The configurations as JSON: each with its id, name and whether it is active.
pub(crate) fn configurations_json(doc: &AssemblyDocument) -> Value {
    let rows: Vec<Value> = doc
        .assembly()
        .configurations
        .iter()
        .map(|c| json!({"id": c.id, "name": c.name, "active": c.is_active()}))
        .collect();
    json!({
        "active": doc.assembly().active_configuration().map(|c| c.name.clone()),
        "configurations": rows,
        "kept_from_another_version": doc.assembly().configurations.reserved().len(),
    })
}

fn print_configurations(doc: &AssemblyDocument, as_json: bool) {
    if as_json {
        println!("{}", configurations_json(doc));
        return;
    }
    let list = &doc.assembly().configurations;
    if !list.reserved().is_empty() {
        println!(
            "{} keeps {} configuration rows of another SolidDraft version as saved",
            doc.assembly().name,
            list.reserved().len()
        );
    } else if list.is_empty() {
        println!("{} has no configurations", doc.assembly().name);
    }
    for c in list {
        println!("{} {}{}", c.id, c.name, if c.is_active() { "  (active)" } else { "" });
    }
}

/// `asm explode <file> <list | add PATH... --direction X,Y,Z --distance MM | edit STEP
/// [--direction X,Y,Z] [--distance MM] | delete STEP | auto [--distance MM]>`.
pub(crate) fn explode(doc: &mut AssemblyDocument, rest: &[String], a: &Args) -> Result<bool, String> {
    let direction =
        a.value("--direction").map(|t| numbers::<3>("--direction", t).map(DVec3::from_array)).transpose()?;
    let distance = a
        .value("--distance")
        .map(|t| t.parse::<f64>().map_err(|_| "--distance takes a number of millimetres".to_owned()))
        .transpose()?;
    let step = |t: &str| t.parse::<u32>().map_err(|_| format!("{t} is not an explode step id"));
    match rest.split_first() {
        Some((verb, [])) if verb == "list" => {
            print_explode(doc, a.json);
            return Ok(false);
        }
        Some((verb, targets)) if verb == "add" => {
            let d = direction.ok_or("explode add needs --direction")?;
            let mm = distance.ok_or("explode add needs --distance")?;
            let id = doc.add_explode_step(paths(targets)?, d, mm).map_err(|e| e.to_string())?;
            println!("added explode step {id}");
        }
        Some((verb, [id])) if verb == "edit" => {
            if direction.is_none() && distance.is_none() {
                return Err("explode edit needs --direction or --distance".into());
            }
            doc.edit_explode_step(step(id)?, direction, distance).map_err(|e| e.to_string())?;
            println!("changed explode step {id}");
        }
        Some((verb, [id])) if verb == "delete" => {
            doc.delete_explode_step(step(id)?).map_err(|e| e.to_string())?;
            println!("deleted explode step {id}");
        }
        Some((verb, [])) if verb == "auto" => {
            let ids = doc.auto_explode(distance.unwrap_or(50.0)).map_err(|e| e.to_string())?;
            println!("made {} explode steps", ids.len());
        }
        _ => {
            return Err(
                "explode takes list, add PATH... --direction X,Y,Z --distance MM, edit STEP, delete STEP or auto"
                    .into(),
            );
        }
    }
    Ok(true)
}

/// The exploded view's steps as JSON, with where each occurrence shows exploded.
pub(crate) fn explode_json(doc: &mut AssemblyDocument) -> Value {
    let steps: Vec<Value> = doc
        .assembly()
        .explode
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "occurrences": s.occurrences.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "direction": s.direction,
                "distance": s.distance,
            })
        })
        .collect();
    let moved: Vec<String> =
        doc.assembly().explode.iter().flat_map(|s| s.occurrences.iter().map(ToString::to_string)).collect();
    let worlds: Vec<Value> = doc
        .exploded_worlds()
        .into_iter()
        .filter(|(p, _)| moved.contains(&p.to_string()))
        .map(|(p, w)| json!({"path": p.to_string(), "at": w.translation.to_array()}))
        .collect();
    json!({"steps": steps, "exploded": worlds})
}

fn print_explode(doc: &mut AssemblyDocument, as_json: bool) {
    if as_json {
        println!("{}", explode_json(doc));
        return;
    }
    if doc.assembly().explode.is_empty() {
        println!("{} has no exploded view", doc.assembly().name);
    }
    for s in &doc.assembly().explode {
        let what: Vec<String> = s.occurrences.iter().map(ToString::to_string).collect();
        let [x, y, z] = s.direction;
        println!("{} {} along {x},{y},{z} by {} mm", s.id, what.join(" "), s.distance);
    }
}
