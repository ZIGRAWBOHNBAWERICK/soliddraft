//! Merge 2 of the ribbon layout: what a pointer cannot prove exhaustively. The tab and
//! group contract comes first, then every command compared with its frozen state on main
//! before the merge. Moving a command on the ribbon changes where it is drawn, and nothing
//! else about it.
//!
//! Requires a renderer: a run without one fails.
//!
//! Run it away from the other GPU targets:
//! `cargo test -p soliddraft-app --test ribbon_registry -- --test-threads=1`.

#[path = "support/ribbon.rs"]
mod ribbon;

#[path = "support/configuration_registry.rs"]
mod configuration_registry;

use ribbon::App;
use serde_json::{Map, Value, json};
use soliddraft_app::commands::{self, Agents, Context, History, Tab};
use soliddraft_app::state::{AppState, RibbonTab};
use std::collections::HashSet;

/// A schema without its prose: names, types and requiredness are the contract.
fn plain(schema: &Value) -> Value {
    match schema {
        Value::Object(fields) => Value::Object(
            fields.iter().filter(|(k, _)| *k != "description").map(|(k, v)| (k.clone(), plain(v))).collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(plain).collect()),
        other => other.clone(),
    }
}

fn agents(kind: Agents) -> &'static str {
    match kind {
        Agents::None => "none",
        Agents::App(_) => "app",
        Agents::Tool => "tool",
        Agents::Verb => "verb",
        Agents::Mcp => "mcp",
    }
}

/// What a move on the ribbon must leave alone, command by command.
fn contract(fixtures: &[(&str, &AppState)]) -> Value {
    let mut rows = Map::new();
    for c in commands::all() {
        let keys: Vec<Value> = c
            .keys
            .iter()
            .map(|k| json!({ "scope": format!("{:?}", k.scope), "chord": k.text(), "while_typing": k.fires_while_typing() }))
            .collect();
        let tool = c.agents.has_tool().then(|| {
            let spec = c.tool_spec();
            json!({ "name": spec.name, "input": plain(&spec.input_schema) })
        });
        let mut availability = Map::new();
        for (name, state) in fixtures {
            let here = json!({ "enabled": c.is_enabled(state), "offered": c.available(Context::of(state)) });
            availability.insert((*name).to_owned(), here);
        }
        rows.insert(
            c.id.to_owned(),
            json!({
                "workspace": format!("{:?}", c.workspace),
                "keys": keys,
                "undo": c.undo.label(),
                "agents": agents(c.agents),
                "tool": tool,
                "availability": availability,
            }),
        );
    }
    let sorted = |ids: Vec<String>| {
        let mut ids = ids;
        ids.sort();
        ids
    };
    json!({
        "commands": rows,
        "agent_tools": sorted(commands::for_agents().map(|c| c.tool_spec().name.to_string()).collect()),
        "verbs": sorted(commands::verbs().map(|c| c.id.to_owned()).collect()),
    })
}

/// The four fixtures, settled, in one contract.
fn observed() -> Value {
    fn settled(mut app: App) -> App {
        app.run_steps(30);
        ribbon::until(&mut app, "the proof", |a| !a.state().state.doc.proof_pending());
        app.render().expect("the fixture renders");
        app
    }
    let blank = settled(ribbon::app(ribbon::blank()));
    let sketch = settled(ribbon::app(ribbon::sketch()));
    let plate = settled(ribbon::app(ribbon::plate()));
    let mut assembly = settled(ribbon::app(ribbon::blank()));
    ribbon::menu(&mut assembly, "File", "New assembly");
    assembly.run_steps(30);
    assert!(assembly.state().state.assembly.is_active(), "the assembly is the active document");
    assembly.render().expect("the assembly renders");
    contract(&[
        ("blank", &blank.state().state),
        ("sketch", &sketch.state().state),
        ("plate", &plate.state().state),
        ("assembly", &assembly.state().state),
    ])
}

/// The contract main had before the merge, frozen from a run on its sources.
/// Shell gained one optional input, in its tool and in its proposal line. Takes it out after
/// checking it, so the rest of the frozen contract still has to match.
fn before_all_faces(id: &str, current: &mut Value) {
    let properties = match id {
        "shell" => current.pointer_mut("/tool/input/properties"),
        "propose" => current
            .pointer_mut("/tool/input/properties/commands/items/anyOf")
            .and_then(Value::as_array_mut)
            .and_then(|lines| lines.iter_mut().find(|line| line["properties"]["op"]["const"] == "shell"))
            .map(|line| &mut line["properties"]),
        _ => return,
    };
    let properties = properties.and_then(Value::as_object_mut).expect("the shell inputs");
    let added = properties.remove("all_faces").expect("shell takes all_faces");
    assert_eq!(added["type"], "boolean", "all_faces is one optional flag");
}

/// The `overflow` input `fillet` and `edit_fillet` gained with Catallax's fillet overflow
/// policy: `"roll_over"` or `{"keep_neighbours": [{edge, at, setback}]}`.
fn overflow_schema() -> Value {
    let station = json!({
        "type":"object", "additionalProperties":false, "required":["edge","at","setback"],
        "properties": {"edge":{"type":"string"}, "at":{"type":"string"}, "setback":{"type":"number"}},
    });
    json!({"anyOf": [
        {"type":"string", "const":"roll_over"},
        {
            "type":"object", "additionalProperties":false, "required":["keep_neighbours"],
            "properties": {"keep_neighbours": {"type":"array", "items": station}},
        },
    ]})
}

/// The `radius_laws` input `fillet` and `edit_fillet` gained with Catallax's variable-radius
/// laws: `[{edge, from, radius: {kind: "linear", start, end}}]`.
fn radius_laws_schema() -> Value {
    let linear = json!({
        "type":"object", "additionalProperties":false, "required":["kind","start","end"],
        "properties": {"kind":{"type":"string", "const":"linear"}, "start":{"type":"number"}, "end":{"type":"number"}},
    });
    json!({"type":"array", "items": {
        "type":"object", "additionalProperties":false, "required":["edge","from","radius"],
        "properties": {"edge":{"type":"string"}, "from":{"type":"string"}, "radius":{"anyOf":[linear]}},
    }})
}

/// The `corner` input `fillet` and `edit_fillet` gained with Catallax's fillet corner
/// choice: `"cliff"` or `"smooth"`.
fn corner_schema() -> Value {
    json!({"anyOf": [{"type":"string", "const":"cliff"}, {"type":"string", "const":"smooth"}]})
}

/// Removes the optional inputs one fillet line gained after the freeze, once each is still
/// optional and matches its declared schema whole.
fn take_fillet_additions(line: &mut Value) -> Result<(), String> {
    for (name, declared) in
        [("overflow", overflow_schema()), ("radius_laws", radius_laws_schema()), ("corner", corner_schema())]
    {
        if line["required"].as_array().is_some_and(|required| required.contains(&json!(name))) {
            return Err(format!("{name} became required"));
        }
        let properties = line["properties"].as_object_mut().ok_or("the line has no properties")?;
        let added = properties.remove(name).ok_or_else(|| format!("{name} is missing"))?;
        if added != declared {
            return Err(format!("{name} is not the declared schema: {added}"));
        }
    }
    Ok(())
}

/// Takes away the inputs `fillet` and `edit_fillet` gained with Catallax's fillet recipe
/// fields, here and in their `propose` lines.
fn before_fillet_additions(id: &str, current: &mut Value) {
    let lines: Vec<&mut Value> = match id {
        "fillet" | "edit_fillet" => current.pointer_mut("/tool/input").into_iter().collect(),
        "propose" => current
            .pointer_mut("/tool/input/properties/commands/items/anyOf")
            .and_then(Value::as_array_mut)
            .map(|lines| {
                lines
                    .iter_mut()
                    .filter(|line| matches!(line["properties"]["op"]["const"].as_str(), Some("fillet" | "edit_fillet")))
                    .collect()
            })
            .unwrap_or_default(),
        _ => return,
    };
    assert!(!lines.is_empty(), "{id} has fillet inputs");
    for line in lines {
        take_fillet_additions(line).unwrap_or_else(|why| panic!("{id}: {why}"));
    }
}

#[test]
fn fillet_additions_are_checked_whole_before_they_are_removed() {
    let line = || {
        json!({
            "type":"object", "additionalProperties":false, "required":["feature","radius"],
            "properties": {
                "feature":{"type":"string"}, "radius":{"type":"number"},
                "overflow": overflow_schema(), "radius_laws": radius_laws_schema(), "corner": corner_schema(),
            },
        })
    };
    let mut declared = line();
    assert_eq!(take_fillet_additions(&mut declared), Ok(()));
    assert_eq!(declared["properties"], json!({"feature":{"type":"string"}, "radius":{"type":"number"}}));

    // A station whose setback is no longer a number.
    let mut station = line();
    station["properties"]["overflow"]["anyOf"][1]["properties"]["keep_neighbours"]["items"]["properties"]["setback"] =
        json!({"type":"string"});
    assert!(take_fillet_additions(&mut station).unwrap_err().starts_with("overflow is not the declared schema"));

    // A third overflow choice.
    let mut extra = line();
    extra["properties"]["overflow"]["anyOf"].as_array_mut().unwrap().push(json!({"type":"string", "const":"stop"}));
    assert!(take_fillet_additions(&mut extra).unwrap_err().starts_with("overflow is not the declared schema"));

    // A law that takes a field Catallax does not spell.
    let mut law = line();
    law["properties"]["radius_laws"]["items"]["properties"]["at"] = json!({"type":"string"});
    assert!(take_fillet_additions(&mut law).unwrap_err().starts_with("radius_laws is not the declared schema"));

    // A third corner choice.
    let mut corner = line();
    corner["properties"]["corner"]["anyOf"].as_array_mut().unwrap().push(json!({"type":"string", "const":"mitre"}));
    assert!(take_fillet_additions(&mut corner).unwrap_err().starts_with("corner is not the declared schema"));

    // The corner choice made required.
    let mut needed = line();
    needed["required"] = json!(["feature", "radius", "corner"]);
    assert_eq!(take_fillet_additions(&mut needed), Err("corner became required".into()));

    // Either input made required.
    let mut required = line();
    required["required"] = json!(["feature", "radius", "overflow"]);
    assert_eq!(take_fillet_additions(&mut required), Err("overflow became required".into()));
}

fn before_trim_modes(id: &str, current: &mut Value) {
    let variants = match id {
        "sketch_geometry" => current.pointer_mut("/tool/input/properties/operations/items/anyOf"),
        "propose" => current
            .pointer_mut("/tool/input/properties/commands/items/anyOf")
            .and_then(Value::as_array_mut)
            .and_then(|lines| lines.iter_mut().find(|line| line["properties"]["op"]["const"] == "sketch_geometry"))
            .and_then(|line| line.pointer_mut("/properties/operations/items/anyOf")),
        _ => return,
    }
    .and_then(Value::as_array_mut)
    .expect("sketch operation variants");
    for tool in ["trim", "trim_corner", "trim_bounded"] {
        let variant = variants.iter_mut().find(|v| v["properties"]["tool"]["const"] == tool).unwrap();
        assert!(!variant["required"].as_array().unwrap().contains(&json!("options")), "trim options remain optional");
        let options =
            variant["properties"].as_object_mut().unwrap().remove("options").expect("trim construction options");
        assert_eq!(
            options,
            json!({
                "type":"object", "additionalProperties":false, "required":[],
                "properties": {"keep_as_construction":{"type":"boolean"}, "ignore_construction":{"type":"boolean"}},
            })
        );
    }
    let index =
        variants.iter().position(|v| v["properties"]["tool"]["const"] == "trim_corner").expect("corner operation");
    let added = variants.remove(index);
    let point = json!({"type":"object", "properties":{"u":{"type":"number"}, "v":{"type":"number"}}, "required":["u","v"], "additionalProperties":false});
    assert_eq!(
        added,
        json!({
            "type":"object", "additionalProperties":false,
            "properties": {
                "tool":{"type":"string","const":"trim_corner"},
                "a":{"type":"integer"}, "b":{"type":"integer"},
                "keep_a":point.clone(), "keep_b":point,
            },
            "required":["tool","a","b","keep_a","keep_b"]
        }),
        "only the declared corner operation extends the frozen schema"
    );
    let index = variants
        .iter()
        .position(|v| v["properties"]["tool"]["const"] == "trim_bounded")
        .expect("bounded trim operation");
    assert_eq!(
        variants.remove(index),
        json!({
            "type":"object", "additionalProperties":false,
            "properties": {
                "tool":{"type":"string","const":"trim_bounded"},
                "entity":{"type":"integer"}, "a":{"type":"integer"}, "b":{"type":"integer"},
                "outside":{"type":"boolean"},
            },
            "required":["tool","entity","a","b","outside"]
        }),
        "only the declared bounded trim operation extends the frozen schema"
    );
}

fn baseline() -> Value {
    serde_json::from_str(include_str!("fixtures/ribbon_registry/baseline.json")).expect("the frozen contract")
}

/// A tab's panels with their command ids, as the ribbon reads them.
fn panels(tab: Tab) -> Vec<(&'static str, Vec<&'static str>)> {
    commands::ribbon(tab).into_iter().map(|(p, items)| (p, items.iter().map(|(c, _)| c.id).collect())).collect()
}

fn tab_called(label: &str) -> Tab {
    Tab::all().into_iter().find(|t| t.label() == label).unwrap_or_else(|| panic!("no tab called {label}"))
}

/// A tab holds exactly these groups in this order, with exactly these commands.
fn holds(tab: &str, groups: &[(&str, &[&str])]) {
    let found = panels(tab_called(tab));
    let names: Vec<&str> = found.iter().map(|(name, _)| *name).collect();
    let wanted: Vec<&str> = groups.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, wanted, "the groups of {tab}");
    for ((name, ids), (_, expected)) in found.iter().zip(groups) {
        let mut ids = ids.clone();
        let mut expected = expected.to_vec();
        ids.sort_unstable();
        expected.sort_unstable();
        assert_eq!(ids, expected, "{tab} > {name}");
    }
}

#[test]
fn the_ribbon_moves_commands_and_changes_nothing_else_about_them() {
    let _case = ribbon::case("u1");

    // The new tabs and groups come first: on main this is what fails.
    holds(
        "Solid",
        &[
            ("Sketch", &["sketch"]),
            ("Create", &["extrude", "revolve", "sweep", "loft"]),
            ("Remove", &["cut", "hole", "slot"]),
            ("Modify", &["fillet", "chamfer", "shell", "draft", "lattice", "gyroid"]),
            ("Pattern", &["linear_pattern", "circular_pattern", "mirror"]),
            ("Reference", &["reference_plane", "reference_axis", "reference_point"]),
        ],
    );
    holds(
        "Sketch",
        &[
            ("Sketch", &["sketch", "exit_sketch"]),
            (
                "Draw",
                &["sketch_line", "sketch_circle", "sketch_rectangle", "sketch_arc", "sketch_slot", "sketch_point"],
            ),
            (
                "Modify",
                &[
                    "sketch_polygon",
                    "sketch_trim",
                    "sketch_extend",
                    "sketch_offset",
                    "sketch_mirror",
                    "sketch_linear_pattern",
                    "sketch_circular_pattern",
                ],
            ),
            ("Constrain", &["smart_dimension", "relations", "construction"]),
            ("Use sketch", &["extrude", "revolve", "cut"]),
            ("View", &["normal_to"]),
        ],
    );
    holds(
        "Inspect",
        &[
            ("Measure", &["measure", "mass_properties"]),
            ("Check", &["check", "section"]),
            ("Surface quality", &["draft_check"]),
        ],
    );
    let view: Vec<&str> = panels(tab_called("View")).iter().map(|(name, _)| *name).collect();
    assert_eq!(view, ["Orientation", "Display", "Projection", "Panels"]);
    for tab in Tab::all() {
        let mut seen = HashSet::new();
        for id in panels(tab).into_iter().flat_map(|(_, ids)| ids) {
            assert!(seen.insert(id), "{id} is on {} twice", tab.label());
        }
    }

    // The tabs by their stable names, whatever they read.
    let tabs: Vec<(&str, bool)> = RibbonTab::ALL.iter().map(|t| (t.id(), t.planned().is_none())).collect();
    assert_eq!(
        tabs,
        [
            ("solid", true),
            ("sketch", true),
            ("assembly", true),
            ("inspect", true),
            ("view", true),
            ("surface", false),
            ("point_cloud", false),
        ]
    );

    // Every command as main left it.
    let (frozen, now) = (baseline(), observed());
    let rows = frozen["commands"].as_object().expect("the frozen commands");
    for (id, row) in rows {
        if id == "reference_plane" {
            let plane = commands::get(id).unwrap();
            assert!(matches!(plane.agents, Agents::Verb));
            assert_eq!(plane.undo, commands::Undo::Step);
            assert!(plane.is_enabled(&AppState::new(ribbon::blank())));
            continue;
        }
        let current = now["commands"].get(id).unwrap_or_else(|| panic!("{id} is gone"));
        let mut current = configuration_registry::before_configurations(id, current, plain);
        before_all_faces(id, &mut current);
        before_trim_modes(id, &mut current);
        before_fillet_additions(id, &mut current);
        if id == "relations" {
            assert_eq!(current["agents"], "app", "Relations is now an interactive sketch command");
            current["agents"] = row["agents"].clone();
        }
        if id == "sweep" {
            assert_eq!(current["agents"], "verb");
            assert_eq!(current["undo"], "step");
            assert_eq!(current["tool"]["name"], "sweep");
            // Sweep gained a tool and history; its other ribbon contracts stay frozen.
            for field in ["agents", "undo", "tool"] {
                current[field] = row[field].clone();
            }
        }
        assert_eq!(&current, row, "{id} changed");
    }
    for list in ["agent_tools", "verbs"] {
        for name in frozen[list].as_array().expect("a frozen list") {
            assert!(now[list].as_array().unwrap().contains(name), "{list} lost {name}");
        }
    }

    // The relocated operations are still the same tools, and two checks stay two.
    for id in [
        "extrude",
        "cut",
        "fillet",
        "chamfer",
        "shell",
        "draft",
        "linear_pattern",
        "circular_pattern",
        "mirror",
        "measure",
        "mass_properties",
    ] {
        let command = commands::get(id).unwrap_or_else(|| panic!("{id} is gone"));
        assert!(command.agents.has_tool(), "{id} keeps its tool");
        assert_eq!(command.tool_spec().name, id, "{id} keeps its name");
    }
    let history = History::default();
    let part = Context::default();
    let first = |query: &str, context: Context| commands::search_here(query, &history, context)[0].command.id;
    assert_eq!(first("check", part), "check");
    assert_eq!(first("check model", part), "check_model");
    assert!(commands::get("check").unwrap().action.is_none(), "Check is not the assistant's check");
    assert!(commands::get("check_model").unwrap().action.is_some());

    // What left the part's ribbon can still be found where it works.
    let drawing = Context { drawing: true, ..Default::default() };
    let assembly = Context { assembly: true, ..Default::default() };
    for (id, contexts) in [
        ("make_drawing", &[part][..]),
        ("show_drawing", &[part]),
        ("ask", &[part, drawing, assembly]),
        ("check_model", &[part]),
        ("explain", &[part]),
        ("summarise", &[part]),
        ("find_issues", &[part]),
        ("provider_settings", &[part]),
        ("auto_apply", &[part]),
        ("undo", &[part, drawing, assembly]),
        ("redo", &[part, drawing, assembly]),
    ] {
        let label = commands::get(id).unwrap_or_else(|| panic!("{id} is gone")).label;
        for context in contexts {
            let found = commands::search_here(label, &history, *context);
            assert!(found.iter().any(|m| m.command.id == id), "searching {label} finds {id} in {context:?}");
        }
    }
}
