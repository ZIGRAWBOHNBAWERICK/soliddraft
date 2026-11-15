//! A body's exact result: a Catallax checkpoint that later features continue, or a build
//! handed over from another kernel through `catallax_app::handover`.

use std::sync::Arc;

use catallax_app::{DEFAULT_BUILD_WORK, Error, EvaluatedApplicationBody, Names};
use catallax_features::{Built, EvaluationCheckpoint};
use catallax_numbers::{Decider, operation_budget_active, with_operation_budget};

/// Where a body's exact result came from.
#[derive(Clone, Debug)]
pub(crate) enum Exact {
    /// Built here. A later feature continues from it without rebuilding the prefix.
    Checkpoint(EvaluationCheckpoint),
    /// Built by another kernel and handed over. A later feature rebuilds its prefix.
    Adopted(Arc<Built>),
}

impl Exact {
    /// The exact body, its intermediate bodies and its recipe.
    pub(crate) fn built(&self) -> &Built {
        match self {
            Exact::Checkpoint(checkpoint) => checkpoint.built(),
            Exact::Adopted(built) => built,
        }
    }

    /// The checkpoint a later feature continues from, if this kernel built the body.
    pub(crate) fn continuation(&self) -> Option<EvaluationCheckpoint> {
        match self {
            Exact::Checkpoint(checkpoint) => Some(checkpoint.clone()),
            Exact::Adopted(_) => None,
        }
    }
}

/// A body's exact result and the picking names of its entities.
#[derive(Clone, Debug)]
pub(crate) struct AppBody {
    pub checkpoint: Exact,
    pub names: Names,
}

impl From<EvaluatedApplicationBody> for AppBody {
    fn from(app: EvaluatedApplicationBody) -> Self {
        AppBody { checkpoint: Exact::Checkpoint(app.checkpoint), names: app.names }
    }
}

impl AppBody {
    /// A build another kernel made and handed over, named as Catallax names its own.
    pub(crate) fn adopted(built: Arc<Built>) -> Self {
        let names = crate::handover::names(&built);
        AppBody { checkpoint: Exact::Adopted(built), names }
    }

    /// The certified mesh, as `EvaluatedApplicationBody::display` gives it: the caller's
    /// allowance, or the application default with exhaustion as a resource limit.
    pub(crate) fn display(
        &self,
        options: &catallax_mesh::MeshOptions,
        decider: &mut Decider,
    ) -> Result<catallax_numbers::Answer<catallax_mesh::Mesh>, Error> {
        let mut run = move || {
            let built = self.checkpoint.built();
            catallax_mesh::mesh_body(&built.model, built.result, options, decider).map_err(Error::Display)
        };
        if operation_budget_active() {
            return run();
        }
        let (result, work) = with_operation_budget(DEFAULT_BUILD_WORK, run);
        if work.exhausted { Err(Error::ResourceLimit) } else { result }
    }

    /// The rendering answer, as `EvaluatedApplicationBody::display_result` gives it: a typed
    /// refusal when display work runs out, on the caller's allowance or the default.
    pub(crate) fn display_result(
        &self,
        options: &catallax_mesh::MeshOptions,
        decider: &mut Decider,
    ) -> Result<catallax_numbers::OperationResult<catallax_mesh::DisplayMesh>, Error> {
        let mut run = move || {
            let built = self.checkpoint.built();
            let revision = serde_json::to_string(built.recipe()).map_err(|error| {
                Error::Display(catallax_numbers::Refusal::new("app.display-revision", error.to_string()))
            })?;
            Ok(catallax_mesh::display_mesh_body(&built.model, built.result, options, &revision, decider))
        };
        if operation_budget_active() { run() } else { with_operation_budget(DEFAULT_BUILD_WORK, run).0 }
    }
}
