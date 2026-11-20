//! Manufacturing information over MCP: add, change, remove and list a part's items.

use serde_json::{Value, json};
use soliddraft_kernel_api::pmi::{
    Anchor, At, Characteristic, Decimal, Feature, Limits, MaterialCondition, PartNotes, PmiItem, PmiKind, Process,
    SizeKind,
};
use soliddraft_model::FaceRef;

use crate::command_tools::{face_by_name, name_of_face};
use crate::document::Document;
use crate::state::AppState;

fn text<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty())
}

/// A decimal given as text or as a number.
fn decimal(input: &Value, key: &str) -> Result<Decimal, String> {
    let read = match input.get(key) {
        Some(Value::String(text)) => Decimal::parse(text),
        Some(Value::Number(number)) => number.as_f64().and_then(Decimal::from_f64),
        _ => return Err(format!("{key} is required")),
    };
    read.ok_or_else(|| format!("{key} is a decimal number, such as 0.05"))
}

/// The faces of one feature, from their names.
fn feature(doc: &Document, value: &Value, what: &str) -> Result<Feature<FaceRef>, String> {
    let names =
        value.as_array().filter(|names| !names.is_empty()).ok_or_else(|| format!("{what} is a list of face names"))?;
    let mut faces = Vec::with_capacity(names.len());
    for name in names {
        let name = name.as_str().ok_or_else(|| format!("{what} is a list of face names"))?;
        faces.push(
            face_by_name(doc, name)
                .ok_or_else(|| format!("no face named `{name}`; `feature` lists each feature's faces by name"))?,
        );
    }
    doc.model().pmi_feature(&faces).map_err(|error| error.to_string())
}

/// One feature from `faces`, or the features of a pattern from `features`.
fn features(doc: &Document, input: &Value) -> Result<Vec<Feature<FaceRef>>, String> {
    let given = |key: &str| input.get(key).filter(|value| !value.is_null());
    match (given("faces"), given("features")) {
        (Some(faces), None) => Ok(vec![feature(doc, faces, "faces")?]),
        (None, Some(list)) => list
            .as_array()
            .filter(|list| !list.is_empty())
            .ok_or("features is a list of features, each a list of face names")?
            .iter()
            .map(|faces| feature(doc, faces, "a feature"))
            .collect(),
        _ => Err("give faces for one feature, or features for a pattern".into()),
    }
}

fn one_feature(doc: &Document, input: &Value) -> Result<Feature<FaceRef>, String> {
    feature(doc, input.get("faces").ok_or("faces is required")?, "faces")
}

fn limits(input: &Value) -> Result<Option<Limits>, String> {
    match text(input, "limits") {
        None => Ok(None),
        Some(written) => Limits::parse(written)
            .map(Some)
            .ok_or_else(|| format!("limits `{written}` are not H7, \u{b1}0.1, +0.1/0 or +0.2/-0.1")),
    }
}

fn anchor(doc: &Document, input: &Value, end: &str) -> Result<Anchor<FaceRef>, String> {
    let faces = format!("{end}_faces");
    let feature = feature(doc, input.get(&faces).ok_or_else(|| format!("{faces} is required"))?, &faces)?;
    let at = match text(input, &format!("{end}_at")) {
        None | Some("feature") => At::Feature,
        Some("middle") => At::Middle,
        Some("inner") => At::Inner,
        Some("outer") => At::Outer,
        Some(other) => return Err(format!("{end}_at `{other}` is not feature, middle, inner or outer")),
    };
    Ok(Anchor { feature, at })
}

fn characteristic(name: &str) -> Result<Characteristic, String> {
    let wanted = name.to_ascii_lowercase().replace(['-', '_'], " ");
    Characteristic::ALL.into_iter().find(|c| c.label().to_ascii_lowercase().replace('-', " ") == wanted).ok_or_else(
        || {
            let names: Vec<String> = Characteristic::ALL.iter().map(|c| c.label().to_ascii_lowercase()).collect();
            format!("no characteristic `{name}` (there are: {})", names.join(", "))
        },
    )
}

/// The item a `pmi_add` call describes, named, on the part's faces.
fn item(doc: &Document, input: &Value) -> Result<PmiItem<FaceRef>, String> {
    let pmi = doc.model().pmi();
    let replaces = text(input, "replaces");
    let kind = match text(input, "kind").ok_or("kind is required")? {
        "datum" => {
            let letter = match text(input, "letter") {
                Some(letter) => letter.to_owned(),
                // A replaced datum keeps its letter unless another is given.
                None => replaces
                    .and_then(|name| pmi.item(name))
                    .and_then(|old| match &old.kind {
                        PmiKind::Datum { letter, .. } => Some(letter.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| pmi.next_datum_letter()),
            };
            PmiKind::Datum { letter, feature: one_feature(doc, input)? }
        }
        size @ ("diameter" | "radius") => PmiKind::Size {
            size: if size == "diameter" { SizeKind::Diameter } else { SizeKind::Radius },
            features: features(doc, input)?,
            limits: limits(input)?,
            nominal: None,
        },
        "distance" => PmiKind::Location {
            from: anchor(doc, input, "from")?,
            to: anchor(doc, input, "to")?,
            limits: limits(input)?,
            nominal: None,
        },
        "tolerance" => PmiKind::Tolerance {
            characteristic: characteristic(text(input, "characteristic").ok_or("characteristic is required")?)?,
            value: decimal(input, "value")?,
            diameter_zone: input["diameter_zone"].as_bool().unwrap_or(false),
            material: match text(input, "material") {
                None => None,
                Some("maximum") => Some(MaterialCondition::Maximum),
                Some("least") => Some(MaterialCondition::Least),
                Some(other) => return Err(format!("material `{other}` is not maximum or least")),
            },
            datums: match input.get("datums").filter(|value| !value.is_null()) {
                None => Vec::new(),
                Some(list) => list
                    .as_array()
                    .ok_or("datums is a list of letters")?
                    .iter()
                    .map(|letter| letter.as_str().map(str::to_owned).ok_or("datums is a list of letters"))
                    .collect::<Result<_, _>>()?,
            },
            features: features(doc, input)?,
        },
        "finish" => PmiKind::Texture {
            parameter: text(input, "parameter").unwrap_or("Ra").to_owned(),
            value: decimal(input, "value")?,
            process: match text(input, "process") {
                None | Some("any") => Process::Any,
                Some("removal") => Process::Removal,
                Some("no_removal") => Process::NoRemoval,
                Some(other) => return Err(format!("process `{other}` is not any, removal or no_removal")),
            },
            feature: one_feature(doc, input)?,
        },
        other => return Err(format!("kind `{other}` is not datum, diameter, radius, distance, tolerance or finish")),
    };
    let mut item = PmiItem { id: String::new(), kind };
    item.id = match (text(input, "name"), replaces) {
        (Some(name), _) => name.to_owned(),
        (None, Some(old)) => old.to_owned(),
        (None, None) => pmi.free_name(&match &item.kind {
            PmiKind::Datum { letter, .. } => format!("Datum {letter}"),
            _ => item.kind_label().to_owned(),
        }),
    };
    Ok(item)
}

/// The part's items as rows of JSON: name, kind, callout, faces and any problem.
pub(crate) fn rows(doc: &Document) -> Value {
    let rows: Vec<Value> = doc
        .model()
        .pmi_rows()
        .iter()
        .map(|row| {
            // Faces by the names the part gives them now, or as they were called when
            // the item was made if they are gone.
            let faces: Vec<Vec<String>> = match &row.on_body {
                Some(on_body) => on_body
                    .features()
                    .into_iter()
                    .map(|feature| feature.iter().filter_map(|face| name_of_face(doc, *face)).collect())
                    .collect(),
                None => row
                    .item
                    .features()
                    .into_iter()
                    .map(|feature| feature.iter().map(|face| face.label.clone()).collect())
                    .collect(),
            };
            let mut value = json!({
                "name": row.item.id,
                "kind": row.item.kind_label(),
                "callout": row.callout(),
                "faces": faces,
            });
            if let Some(nominal) = row.nominal {
                value["nominal_mm"] = json!(nominal);
            }
            if let Some(problem) = &row.problem {
                value["problem"] = json!(problem);
            }
            value
        })
        .collect();
    let mut out = json!({ "items": rows, "notes": doc.model().part_notes() });
    if let Some(report) = &doc.pmi_import {
        out["read_from_file"] = json!(report);
    }
    out
}

fn notes(doc: &Document, input: &Value) -> Result<PartNotes, String> {
    let mut notes = doc.model().pmi().part.clone();
    // An empty text clears a field; a field left out keeps its value.
    let field = |key: &str, slot: &mut Option<String>| match input.get(key) {
        Some(Value::String(written)) => *slot = Some(written.trim().to_owned()).filter(|text| !text.is_empty()),
        Some(Value::Null) | None => {}
        Some(_) => {}
    };
    field("general_tolerance", &mut notes.general_tolerance);
    field("material", &mut notes.material);
    if let Some(list) = input.get("notes").filter(|value| !value.is_null()) {
        notes.notes = list
            .as_array()
            .ok_or("notes is a list of texts")?
            .iter()
            .map(|note| note.as_str().map(|text| text.trim().to_owned()).ok_or("notes is a list of texts"))
            .collect::<Result<_, _>>()?;
    }
    Ok(notes)
}

/// Runs the manufacturing information tools. `None` hands the call on.
pub(crate) fn run(state: &mut AppState, name: &str, input: &Value) -> Option<Result<Value, String>> {
    let doc = &mut state.doc;
    Some(match name {
        "pmi_add" => (|| {
            let item = item(doc, input)?;
            let name = item.id.clone();
            doc.put_pmi(item, text(input, "replaces").map(str::to_owned))?;
            let listed = rows(doc);
            let row = listed["items"].as_array().and_then(|rows| rows.iter().find(|row| row["name"] == name.as_str()));
            Ok(json!({ "added": row }))
        })(),
        "pmi_remove" => (|| {
            let name = text(input, "name").ok_or("name is required")?;
            doc.remove_pmi(name)?;
            Ok(json!({ "removed": name }))
        })(),
        "pmi_notes" => (|| {
            let notes = notes(doc, input)?;
            doc.set_part_notes(notes)?;
            Ok(json!({ "notes": doc.model().part_notes() }))
        })(),
        "pmi_list" => Ok(rows(doc)),
        _ => return None,
    })
}
