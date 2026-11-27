//! A body's exact result: a Catallax checkpoint that later features continue, or a build
//! handed over from another kernel through `catallax_app::handover`.

use std::collections::BTreeMap;
use std::sync::Arc;

use catallax_app::{Error, EvaluatedApplicationBody, Names};
use catallax_features::{Built, EvaluationCheckpoint};
use catallax_numbers::Decider;
use catallax_recipe::Recipe;

use crate::RequestPolicy;

/// Where a body's exact result came from.
#[derive(Clone, Debug)]
pub(crate) enum Exact {
    /// Built here. A later feature continues from it without rebuilding the prefix.
    Checkpoint(EvaluationCheckpoint),
    /// Built by another kernel and handed over. A later feature rebuilds its prefix.
    Adopted(Arc<Built>),
    /// Built by another kernel and handed over, read from its bytes only when exact
    /// geometry is first needed. A later feature rebuilds its prefix.
    Offered(Box<Unread>),
}

/// A handed-over build not read yet, and the recipe to evaluate should its bytes not read.
#[derive(Clone, Debug)]
pub(crate) struct Unread {
    offer: crate::handover::Offer,
    recipe: Arc<Recipe>,
    evaluated: Arc<std::sync::OnceLock<EvaluationCheckpoint>>,
}

impl Exact {
    /// The exact body, its intermediate bodies and its recipe. An offered build is read
    /// here the first time; bytes that do not read are replaced by evaluating the recipe.
    pub(crate) fn built(&self) -> &Built {
        match self {
            Exact::Checkpoint(checkpoint) => checkpoint.built(),
            Exact::Adopted(built) => built,
            Exact::Offered(unread) => match unread.offer.read() {
                Ok(built) => built,
                Err(_) => unread
                    .evaluated
                    .get_or_init(|| {
                        catallax_features::evaluate_incremental(&unread.recipe, None, &mut Decider::default())
                            .expect("the handing kernel built this recipe")
                    })
                    .built(),
            },
        }
    }

    /// Whether the exact body is in memory, rather than in bytes not read yet.
    pub(crate) fn is_read(&self) -> bool {
        match self {
            Exact::Offered(unread) => unread.offer.is_read(),
            _ => true,
        }
    }

    /// The checkpoint a later feature continues from, if this kernel built the body.
    pub(crate) fn continuation(&self) -> Option<&EvaluationCheckpoint> {
        match self {
            Exact::Checkpoint(checkpoint) => Some(checkpoint),
            Exact::Adopted(_) | Exact::Offered(_) => None,
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

    /// A build another kernel made and handed over for `ours`, adopted without reading
    /// its bytes: its names come with the hand-over, renamed as [`AppBody::adopted`] does.
    /// `None` if the hand-over carries no names or they do not line up with `ours`.
    pub(crate) fn offered(offer: crate::handover::Offer, ours: &Recipe) -> Option<Self> {
        let identity = offer.identity()?.clone();
        let theirs = &identity.steps;
        if theirs.len() != ours.steps.len() {
            return None;
        }
        let renames: BTreeMap<String, String> = theirs
            .iter()
            .zip(&ours.steps)
            .filter(|(theirs, ours)| theirs.as_str() != ours.id.as_str())
            .map(|(theirs, ours)| (theirs.clone(), ours.id.as_str().to_owned()))
            .collect();
        let names = identity.names(&renames)?;
        let recipe = Arc::new(ours.clone());
        let unread = Unread { offer, recipe: recipe.clone(), evaluated: Arc::default() };
        Some(AppBody { checkpoint: Exact::Offered(Box::new(unread)), names, recipe: Some(recipe) })
    }

    /// The kernel instance an offered build came from.
    pub(crate) fn handed_source(&self) -> Option<u64> {
        match &self.checkpoint {
            Exact::Offered(unread) => Some(unread.offer.source),
            _ => None,
        }
    }

    /// What came with an offered build for adopting it unread.
    pub(crate) fn handed_identity(&self) -> Option<&Arc<crate::handover::HandedIdentity>> {
        match &self.checkpoint {
            Exact::Offered(unread) => unread.offer.identity(),
            _ => None,
        }
    }

    /// What came with an offered build, while its bytes are still unread: facts an edit
    /// asks for are answered from it instead of reading them.
    pub(crate) fn unread(&self) -> Option<&crate::handover::HandedIdentity> {
        if self.checkpoint.is_read() { None } else { self.handed_identity().map(|identity| identity.as_ref()) }
    }

    /// The size of an offered build's unread bytes.
    pub(crate) fn unread_bytes(&self) -> Option<usize> {
        match &self.checkpoint {
            Exact::Offered(unread) if !unread.offer.is_read() => Some(unread.offer.len()),
            _ => None,
        }
    }

    /// Whether the body is empty, from its names while its bytes are unread.
    pub(crate) fn is_empty(&self) -> bool {
        if !self.checkpoint.is_read() {
            return self.names.faces.is_empty();
        }
        let exact = self.checkpoint.built();
        exact.model.body(exact.result).kind == catallax_topology::BodyKind::Empty
    }

    /// Whether the body is open faces enclosing no volume, as Catallax builds a surface
    /// model a STEP file holds, rather than a closed solid.
    pub(crate) fn is_surface(&self) -> bool {
        // A body adopted unread brought its kind with it.
        if let Some(handed) = self.unread() {
            return handed.surface;
        }
        let exact = self.checkpoint.built();
        exact.model.body(exact.result).kind == catallax_topology::BodyKind::Surface
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
        let (result, work) = RequestPolicy::Display.run(|| {
            let built = self.checkpoint.built();
            catallax_mesh::mesh_body(&built.model, built.result, options, decider).map_err(Error::Display)
        });
        if work.exhausted_here() { Err(Error::ResourceLimit) } else { result }
    }

    /// The rendering answer, as `EvaluatedApplicationBody::display_result` gives it: a typed
    /// refusal when display work runs out, on the caller's allowance or the default.
    pub(crate) fn display_result(
        &self,
        options: &catallax_mesh::MeshOptions,
        decider: &mut Decider,
    ) -> Result<catallax_numbers::OperationResult<catallax_mesh::DisplayMesh>, Error> {
        RequestPolicy::Display
            .run(|| {
                let built = self.checkpoint.built();
                let revision = serde_json::to_string(built.recipe()).map_err(|error| {
                    Error::Display(catallax_numbers::Refusal::new("app.display-revision", error.to_string()))
                })?;
                Ok(catallax_mesh::display_mesh_body(&built.model, built.result, options, &revision, decider))
            })
            .0
    }
}
