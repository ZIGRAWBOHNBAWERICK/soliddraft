//! Built bodies handed from one kernel to another through `catallax_app::handover`.
//!
//! The browser builds in its kernel worker and the page rebuilds the same definitions
//! on its own kernel. The worker keeps each Catallax build it makes and hands them over
//! as bytes; the page is offered them, and a build of the same definition takes the
//! offered body instead of evaluating it again. Definitions are matched by their
//! Catallax recipe with step ids renamed by position, because each kernel numbers its
//! steps from its own sequence.

use std::collections::BTreeMap;
use std::sync::Arc;

use catallax_app::Names;
use catallax_features::Built;
use catallax_numbers::Decider;
use serde_json::Value;
use sha2::{Digest, Sha256};
use soliddraft_kernel_api::{KernelError, KernelResult};

/// The first bytes of a hand-over, then its version.
const MAGIC: &[u8; 16] = b"soliddraft-hand1";

/// The key of a Catallax recipe that does not depend on the kernel that wrote it.
pub(crate) fn key(recipe: &catallax_recipe::Recipe) -> String {
    let mut value = serde_json::to_value(recipe).expect("Catallax recipes serialize");
    let ids: BTreeMap<String, String> = value["steps"]
        .as_array()
        .map(|steps| {
            steps
                .iter()
                .enumerate()
                .filter_map(|(index, step)| Some((step["id"].as_str()?.to_owned(), format!("step{index}"))))
                .collect()
        })
        .unwrap_or_default();
    rename(&mut value, &ids);
    let digest = Sha256::digest(value.to_string().as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn rename(value: &mut Value, ids: &BTreeMap<String, String>) {
    match value {
        Value::String(text) => {
            if let Some(id) = ids.get(text.as_str()) {
                *text = id.clone();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| rename(item, ids)),
        Value::Object(object) => object.values_mut().for_each(|item| rename(item, ids)),
        _ => {}
    }
}

/// Builds by key, as bytes another kernel can be offered.
pub(crate) fn encode(builds: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend((builds.len() as u32).to_le_bytes());
    for (key, body) in builds {
        out.extend((key.len() as u32).to_le_bytes());
        out.extend(key.as_bytes());
        out.extend((body.len() as u64).to_le_bytes());
        out.extend(body);
    }
    out
}

/// Reads a hand-over back into builds, each restored into its own Catallax stores.
pub(crate) fn decode(bytes: &[u8]) -> KernelResult<Vec<(String, Arc<Built>)>> {
    let broken = || KernelError::InvalidInput("The hand-over is damaged or from another version.".into());
    let mut rest = bytes.strip_prefix(MAGIC.as_slice()).ok_or_else(broken)?;
    let mut take = |len: usize| -> KernelResult<&[u8]> {
        let (head, tail) = rest.split_at_checked(len).ok_or_else(broken)?;
        rest = tail;
        Ok(head)
    };
    let count = u32::from_le_bytes(take(4)?.try_into().expect("four bytes"));
    let mut builds = Vec::new();
    for _ in 0..count {
        let key_len = u32::from_le_bytes(take(4)?.try_into().expect("four bytes")) as usize;
        let key = String::from_utf8(take(key_len)?.to_vec()).map_err(|_| broken())?;
        let body_len = u64::from_le_bytes(take(8)?.try_into().expect("eight bytes"));
        let body = take(usize::try_from(body_len).map_err(|_| broken())?)?;
        let built = catallax_app::handover::read_body(body, &mut Decider::default())
            .map_err(|refusal| KernelError::InvalidInput(format!("{}: {}", refusal.code, refusal.detail)))?;
        builds.push((key, Arc::new(built)));
    }
    if !rest.is_empty() {
        return Err(broken());
    }
    Ok(builds)
}

/// The bytes of one build, for [`encode`].
pub(crate) fn write(built: &Built) -> KernelResult<Vec<u8>> {
    catallax_app::handover::write_body(built, &mut Decider::default())
        .map_err(|refusal| KernelError::InvalidInput(format!("{}: {}", refusal.code, refusal.detail)))
}

/// The picking names of a handed-over build's result, as Catallax names a build's own.
pub(crate) fn names(built: &Built) -> Names {
    let (model, body) = (&built.model, built.result);
    Names {
        faces: model.faces_of_body(body).into_iter().map(|face| (face, model.face_origins(face).to_vec())).collect(),
        edges: model.edges_of_body(body).into_iter().map(|edge| (edge, model.edge_name(body, edge))).collect(),
        vertices: model
            .vertices_of_body(body)
            .into_iter()
            .map(|vertex| (vertex, model.vertex_name(body, vertex)))
            .collect(),
    }
}
