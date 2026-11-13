//! Saved feature instances expanded by Catallax's editor, with explicit input bindings.

use std::collections::{BTreeMap, BTreeSet};

use catallax_app::{
    pattern::{FeatureInstance, expand_feature},
    soliddraft::Converted,
};
use catallax_numbers::{Decider, Rational};
use catallax_recipe::{Operation, Origin, Step, StepId};
use catallax_shapes::{Isometry, RPoint3, RVector3};
use serde_json::{Value, json};
use soliddraft_kernel_api::{KernelError, KernelResult, OpTag, PatternLayout};

pub(super) fn refusal(code: &str) -> KernelError {
    let message = match code {
        "pattern.expansion-exact-parameter" => {
            "This feature pattern rotation cannot be saved exactly. Try quarter or half turns"
        }
        "pattern.expansion" => {
            "The source feature's inputs or references do not match this copy. Select a boss, cut or hole"
        }
        "numbers.operation-budget" | "app.resource-limit" => {
            "Pattern needs too much work to finish. Reduce the number of copies"
        }
        _ => "Pattern could not be verified for this feature and layout. Try fewer copies or a simpler source",
    };
    KernelError::InvalidInput(message.into())
}

fn id(value: &str) -> KernelResult<StepId> {
    StepId::new(value).map_err(|_| refusal("pattern.expansion"))
}

fn placement(layout: &PatternLayout, index: u32, dec: &mut Decider) -> KernelResult<Isometry> {
    let n = Rational::from_i64(i64::from(index));
    Ok(match *layout {
        PatternLayout::Linear { step, .. } => {
            let [x, y, z] = crate::native::vector(step, "pattern step")?.0.map(|v| v.value().clone());
            Isometry::translation(RVector3::new(x * &n, y * &n, z * &n))
        }
        PatternLayout::Circular { axis_origin, axis_direction, angle_deg, .. } => {
            let [x, y, z] = crate::native::point(axis_origin, "pattern axis")?.0.map(|v| v.value().clone());
            let [a, b, c] = crate::native::vector(axis_direction, "pattern axis")?.0.map(|v| v.value().clone());
            let angle = crate::native::number(angle_deg, "pattern angle")?.value().clone() * n;
            Isometry::around_axis(&RPoint3::new(x, y, z), &RVector3::new(a, b, c), &angle, dec)
                .map_err(|e| refusal(e.code))?
        }
    })
}

fn union(target: &StepId, tool: &StepId) -> Operation {
    serde_json::from_value(json!({"op":"boolean", "kind":"union", "target":target, "tool":tool}))
        .expect("a generated union is a valid operation")
}

/// Replaces one retained placeholder at its original position, ahead of later features.
pub(super) fn expand(converted: &mut Converted, saved: &Value) -> KernelResult<()> {
    let pattern = id(saved["id"].as_str().ok_or_else(|| refusal("pattern.expansion"))?)?;
    let source = id(saved["source"].as_str().ok_or_else(|| refusal("pattern.expansion"))?)?;
    let base = id(saved["base"].as_str().ok_or_else(|| refusal("pattern.expansion"))?)?;
    let before = saved["before"].as_str().map(id).transpose()?;
    let layout: PatternLayout =
        serde_json::from_value(saved["layout"].clone()).map_err(|_| refusal("pattern.expansion"))?;
    let count = match layout {
        PatternLayout::Linear { count, .. } | PatternLayout::Circular { count, .. } => count,
    };
    if !(2..=1000).contains(&count) {
        return Err(KernelError::InvalidInput("A feature pattern needs between 2 and 1000 copies".into()));
    }
    let selected = saved["graph"]
        .as_array()
        .ok_or_else(|| refusal("pattern.expansion"))?
        .iter()
        .map(|v| v.as_str().ok_or_else(|| refusal("pattern.expansion")))
        .collect::<KernelResult<BTreeSet<_>>>()?;
    let index =
        converted.recipe.steps.iter().position(|s| s.id == pattern).ok_or_else(|| refusal("pattern.expansion"))?;
    let mut recipe = converted.recipe.clone();
    let final_result = recipe.result.clone();
    let mut suffix = recipe.steps.split_off(index);
    suffix.remove(0);
    recipe.result = base;
    let graph = recipe
        .steps
        .iter()
        .filter(|step| {
            selected.iter().any(|source| {
                let name = step.id.as_str();
                name == *source
                    || name.starts_with(&format!("{source}_path"))
                    || name.starts_with(&format!("{source}_copy"))
                    || name.starts_with(&format!("{source}_join"))
            })
        })
        .map(|step| step.id.clone())
        .collect::<Vec<_>>();
    if graph.is_empty() || !graph.contains(&source) {
        return Err(refusal("pattern.expansion"));
    }
    let members = graph.iter().cloned().collect::<BTreeSet<_>>();
    let external = recipe
        .steps
        .iter()
        .filter(|s| !members.contains(&s.id))
        .map(|s| (s.id.clone(), s.id.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut dec = Decider::default();
    for copy in 1..count {
        let previous = recipe.result.clone();
        let steps = graph
            .iter()
            .map(|source| Ok((source.clone(), id(&format!("{}_copy{copy}_{}", pattern.as_str(), source.as_str()))?)))
            .collect::<KernelResult<BTreeMap<_, _>>>()?;
        let mut inputs = external.clone();
        if let Some(before) = &before {
            inputs.insert(before.clone(), previous.clone());
        }
        let instance = FeatureInstance {
            placement: placement(&layout, copy, &mut dec)?,
            steps: steps.clone(),
            inputs,
            origins: external.clone(),
        };
        recipe = expand_feature(&recipe, &graph, &source, &[instance], &mut dec).map_err(|e| refusal(e.code))?;
        if before.is_none() {
            let joined = id(&format!("{}_join{copy}", pattern.as_str()))?;
            recipe.steps.push(Step { id: joined.clone(), op: union(&previous, &recipe.result) });
            recipe.result = joined.clone();
            converted.steps.insert(joined.as_str().into(), joined);
        }
        for (_, target) in steps {
            converted.steps.insert(target.as_str().into(), target);
        }
    }
    recipe.steps.push(Step { id: pattern.clone(), op: union(&recipe.result, &recipe.result) });
    recipe.steps.extend(suffix);
    recipe.result = final_result;
    recipe.check().map_err(|_| refusal("pattern.expansion"))?;
    converted.recipe = recipe;
    Ok(())
}

/// The source operation and instance encoded by an expanded step's saved identity.
pub(super) fn instance(step: &str) -> Option<(u32, &str)> {
    let (_, copy) = step.split_once("_copy")?;
    let (index, source) = copy.split_once('_')?;
    Some((index.parse().ok()?, source))
}

/// Use caller tags in the source key so edits never name a transient recipe counter.
pub(super) fn source_key(origin: &Origin, tags: &BTreeMap<String, OpTag>) -> Option<String> {
    fn canonical_step(step: &str, tags: &BTreeMap<String, OpTag>) -> Option<Value> {
        let tag = tags.get(step)?.as_str();
        Some(if let Some((copy, source)) = instance(step) {
            json!({"step":tag, "copy":copy, "of":canonical_step(source, tags)?})
        } else {
            json!(tag)
        })
    }
    fn canonical(value: &mut Value, tags: &BTreeMap<String, OpTag>) -> Option<()> {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    if key == "step" {
                        let step = value.as_str()?;
                        *value = canonical_step(step, tags)?;
                    } else {
                        canonical(value, tags)?;
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    canonical(value, tags)?;
                }
            }
            _ => (),
        }
        Some(())
    }
    let mut role = serde_json::to_value(&origin.role).ok()?;
    canonical(&mut role, tags)?;
    let source = if let Some((copy, step)) = instance(origin.step.as_str()) {
        let source = Origin { step: StepId::new(step).ok()?, role: origin.role.clone() };
        json!({"step":tags.get(origin.step.as_str())?.as_str(), "copy":copy, "of":source_key(&source, tags)?})
    } else {
        json!({"step":tags.get(origin.step.as_str())?.as_str(), "role":role})
    };
    Some(source.to_string())
}
