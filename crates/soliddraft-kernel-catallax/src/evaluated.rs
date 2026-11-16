//! A body's exact result: a Catallax checkpoint that later features continue, or a build
//! handed over from another kernel through `catallax_app::handover`.

use std::collections::BTreeMap;
use std::sync::Arc;

use catallax_app::{DEFAULT_BUILD_WORK, Error, EvaluatedApplicationBody, Names};
use catallax_features::{Built, EvaluationCheckpoint};
use catallax_numbers::{Decider, operation_budget_active, with_operation_budget};
use catallax_recipe::Recipe;

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
    /// Named in this kernel's step ids.
    pub names: Names,
    /// An adopted build's recipe in this kernel's step ids. The build's own recipe numbers
    /// its steps as the handing kernel did.
    recipe: Option<Arc<Recipe>>,
}

impl From<EvaluatedApplicationBody> for AppBody {
    fn from(app: EvaluatedApplicationBody) -> Self {
        AppBody::built_here(app.checkpoint, app.names)
    }
}

impl AppBody {
    /// A body this kernel built.
    pub(crate) fn built_here(checkpoint: EvaluationCheckpoint, names: Names) -> Self {
        AppBody { checkpoint: Exact::Checkpoint(checkpoint), names, recipe: None }
    }

    /// A build another kernel made and handed over for `ours`, named as Catallax names
    /// its own. Each kernel numbers recipe steps from its own sequence, so the build's
    /// names are renamed from the handing kernel's step ids to the ids of `ours` at the
    /// same positions. `None` if the recipes do not line up step by step.
    pub(crate) fn adopted(built: Arc<Built>, ours: &Recipe) -> Option<Self> {
        let theirs = &built.recipe().steps;
        if theirs.len() != ours.steps.len() {
            return None;
        }
        let renames: BTreeMap<String, String> = theirs
            .iter()
            .zip(&ours.steps)
            .filter(|(theirs, ours)| theirs.id != ours.id)
            .map(|(theirs, ours)| (theirs.id.as_str().to_owned(), ours.id.as_str().to_owned()))
            .collect();
        let names = crate::handover::renamed_names(crate::handover::names(&built), &renames)?;
        Some(AppBody { checkpoint: Exact::Adopted(built), names, recipe: Some(Arc::new(ours.clone())) })
    }

    /// The body's recipe in this kernel's step ids.
    pub(crate) fn recipe(&self) -> &Recipe {
        self.recipe.as_deref().unwrap_or_else(|| self.checkpoint.built().recipe())
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
