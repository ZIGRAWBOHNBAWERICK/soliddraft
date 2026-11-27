//! Native part files.
//!
//! A part file (`.dwprt`) is a document envelope (see `soliddraft-document`) with two
//! entries:
//!
//! - `manifest.json`: the part's id, kind and format, the application and kernel that
//!   wrote it, its semantic hash (the revision drawings and assemblies pin) and its
//!   geometry hash, and its sections.
//! - `part.json`: the truth. The format tag (`soliddraft-part/2`, `/3` for explicit
//!   body identities, `/4` from development builds that wrote extrusion options or the
//!   first part options or Hole Wizard features, `/5` for parameter, configuration and
//!   appearance data, `/6` for extrusion options, `/7` for Hole Wizard features, or
//!   `/8` for graded wall thickness), the part's name and
//!   units, every feature in tree order with its parameters, sketch and persistent
//!   references, the rollback bar and the next free feature id, and when it has them
//!   the part's text properties and its material and appearance assignments with copies
//!   of what they use (see [`materials`](crate::materials)). It never holds kernel ids or
//!   geometry the kernel computes, so it rebuilds on any backend.
//! - `settings.json`, when any setting differs from its default: viewport quality,
//!   export accuracy and whether to save display data (see `soliddraft-tessellation`).
//!   It is an optional section outside both hashes, so a quality change never gives the
//!   part a new revision. A reader that cannot read it opens the part with the default
//!   settings.
//! - `view.json`, when the user showed or hid anything: the features, reference planes
//!   and origin axes chosen, by id (see [`view`](crate::view)). An optional section
//!   outside both hashes, like the settings; a reader that cannot read it opens the part
//!   with every item at its default.
//! - `saved-views.json`, when the part has named PMI views: their order, camera poses
//!   and visible PMI item IDs. Optional, outside both hashes.
//! - `cache/mesh/coarse.bin` and `cache/mesh/standard.bin`, when the settings save
//!   display data and the part has a body: its meshes at two display levels, so it can
//!   be shown without a rebuild (see [`display`](crate::display)). Optional, outside both
//!   hashes.
//! - `thumbnail.png`, when the app saved the part with one: a small picture of the part
//!   for a start page or a file browser. Optional, outside both hashes.
//!
//! Files from before the envelope (`.dwpart`, `soliddraft-part/1`) hold the same truth
//! as `recipe.json` beside a `meta.json`. They still open, with an id derived from their
//! content, and are written in the current format when saved.
//!
//! Writing is deterministic: the same document always gives the same bytes, so saving
//! an opened file reproduces it exactly. Floating-point values round-trip bit for bit.
//! The format's major version is the number after the slash; a file with a newer major
//! version, or with a required section this version does not know, is refused rather
//! than misread.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use soliddraft_document::manifest::{self, Entry, LEGACY_PART_TRUTH, Manifest, Package, Section};
use soliddraft_document::{
    ContentHash, DocKind, DocumentId, EnvelopeError,
    kind::{format_major, found_tag},
};
use soliddraft_kernel_api::{BackendInfo, Kernel};

use soliddraft_materials::Snapshots;
use soliddraft_tessellation::{Settings, settings};

use crate::display;
use crate::document::{Document, PartView};
use crate::feature::{DatumPlane, Feature, FeatureId};
use crate::materials::{AppearanceAssignment, MaterialAssignment, PartMaterials};
use crate::view::{self, Visibility};

pub mod saved_view;

/// The format for a part using the original document envelope.
pub const FORMAT: &str = "soliddraft-part/2";
/// Parts with explicit body identities require this version.
pub const MULTIBODY_FORMAT: &str = "soliddraft-part/3";
/// Parts that hold formulas, parameter comments, configuration suppression, body
/// materials and appearances, or display states require this version.
pub const PART_OPTIONS_FORMAT: &str = "soliddraft-part/5";
/// Parts whose extrusions start off their sketch plane, go a second direction, are thin
/// or use chosen contours require this version, which also reads everything before it.
/// An older reader would drop those choices and build a different part, so it must refuse.
pub const OPTIONS_FORMAT: &str = "soliddraft-part/6";
const OPTIONS_VERSION: u32 = 6;
/// Parts with Hole Wizard features (position sketches, standards, end conditions, options)
/// require this version, which also reads everything before it.
pub const HOLE_WIZARD_FORMAT: &str = "soliddraft-part/7";
const HOLE_WIZARD_VERSION: u32 = 7;
/// Parts with a graded thickness in Shell or Thicken require this version.
pub const GRADED_WALL_FORMAT: &str = "soliddraft-part/8";
const GRADED_WALL_VERSION: u32 = 8;
/// Development builds wrote extrusion options, the first part options or Hole Wizard
/// features under this tag. All still open: each is read by what it holds.
const EARLY_OPTIONS_FORMAT: &str = "soliddraft-part/4";
const EARLY_OPTIONS_VERSION: u32 = 4;
/// The newest part major version this reader accepts.
pub const NEWEST_FORMAT: u32 = GRADED_WALL_VERSION;
/// The format tag of part files from before the document envelope.
pub const LEGACY_FORMAT: &str = "soliddraft-part/1";
/// The file name extension parts are saved under.
pub const EXTENSION: &str = soliddraft_document::PART_EXTENSION;
/// The extension of part files from before the envelope, which still open.
pub const LEGACY_EXTENSION: &str = soliddraft_document::LEGACY_PART_EXTENSION;

/// The truth entry.
const PART: &str = "part.json";

/// Why a part file could not be written or read.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    /// Reading or writing the file failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
    /// The file is not a zip archive, or an entry is missing.
    #[error("not a SolidDraft part file: {0}")]
    Archive(String),
    /// `part.json` is not valid.
    #[error("the part's recipe is not valid: {0}")]
    Json(#[from] serde_json::Error),
    /// The format tag is not one this version reads.
    #[error(
        "the file {}, this version reads {GRADED_WALL_FORMAT}, {HOLE_WIZARD_FORMAT}, {OPTIONS_FORMAT}, {PART_OPTIONS_FORMAT}, {EARLY_OPTIONS_FORMAT}, {MULTIBODY_FORMAT}, {FORMAT} and {LEGACY_FORMAT}",
        found_tag(.found)
    )]
    Format {
        /// The tag in the file.
        found: String,
    },
    /// The file needs a newer SolidDraft, or is another kind of document.
    #[error("{0}")]
    Unsupported(String),
    /// The recipe parses but is inconsistent.
    #[error("the part's recipe is inconsistent: {0}")]
    Invalid(String),
}

impl From<EnvelopeError> for FileError {
    fn from(e: EnvelopeError) -> FileError {
        match e {
            EnvelopeError::Io(e) => FileError::Io(e),
            EnvelopeError::Archive(e) => FileError::Archive(e.to_string()),
            EnvelopeError::Json(e) => FileError::Json(e),
            EnvelopeError::Unsupported(m) => FileError::Unsupported(m),
        }
    }
}

fn first_body_id() -> u32 {
    1
}
fn is_first_body_id(id: &u32) -> bool {
    *id == 1
}

/// `part.json`, and `recipe.json` before the envelope.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipe {
    format: String,
    name: String,
    units: Units,
    features: Vec<Feature>,
    #[serde(default, skip_serializing_if = "crate::values::Values::is_empty")]
    values: crate::values::Values,
    #[serde(default, skip_serializing_if = "crate::configs::Configurations::is_empty")]
    configurations: crate::configs::Configurations,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    properties: crate::configs::Properties,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    datum_names: BTreeMap<DatumPlane, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rollback: Option<FeatureId>,
    next_feature: u32,
    #[serde(default = "first_body_id", skip_serializing_if = "is_first_body_id")]
    next_body: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    material: Vec<MaterialAssignment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    appearances: Vec<AppearanceAssignment>,
    #[serde(default, skip_serializing_if = "Snapshots::is_empty")]
    snapshots: Snapshots,
    #[serde(default, skip_serializing_if = "crate::pmi::PartPmi::is_empty")]
    pmi: crate::pmi::PartPmi,
    #[serde(default, skip_serializing_if = "soliddraft_document::display_state::DisplayStates::is_empty")]
    display_states: soliddraft_document::display_state::DisplayStates<crate::materials::PartDisplay>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Units {
    /// Millimetres and degrees.
    Mm,
}

/// An immutable file snapshot without kernel bodies, meshes or undo history.
/// Capturing it only copies document data; encoding and hashing can run on a worker.
pub struct Snapshot {
    id: DocumentId,
    recipe: Recipe,
    settings: Settings,
    view: PartView,
    backend: BackendInfo,
}

impl Snapshot {
    /// Encodes a recovery copy and returns the manifest describing those exact bytes.
    /// Display data and thumbnails are deliberately absent.
    pub fn encode(&self) -> Result<(Manifest, Vec<u8>), FileError> {
        let truth = serde_json::to_value(&self.recipe)?;
        let m = part_manifest(self.id, &self.backend, &truth, &self.settings, &self.view);
        let mut entries = vec![Entry::new(PART, (serde_json::to_string_pretty(&self.recipe)? + "\n").into_bytes())];
        entries.extend(optional_entries(&self.settings, &self.view)?);
        let bytes = manifest::write_package(&m, &entries)?;
        Ok((m, bytes))
    }
}

impl Document {
    /// Copies the data a recovery file needs without serializing it or retaining builds.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            id: self.id,
            recipe: self.recipe(),
            settings: self.settings.clone(),
            view: self.view.clone(),
            backend: self.kernel.backend(),
        }
    }

    fn recipe(&self) -> Recipe {
        let format = if uses_graded_walls(&self.features) {
            GRADED_WALL_FORMAT
        } else if let Some(tag) = feature_options_after(&self.features, EARLY_OPTIONS_VERSION) {
            tag
        } else if self.values.uses_formula_settings()
            || self.configurations.uses_formula_settings()
            || has_extended_styles(&self.materials)
        {
            PART_OPTIONS_FORMAT
        } else if self.next_body > 1 {
            MULTIBODY_FORMAT
        } else {
            FORMAT
        };
        Recipe {
            format: format.into(),
            name: self.name.clone(),
            units: Units::Mm,
            features: self
                .features
                .iter()
                .map(|feature| {
                    let mut saved = feature.clone();
                    if let crate::FeatureKind::ImportStep(import) = &mut saved.kind
                        && let Some(record) = self.import_record(feature.id)
                    {
                        import.record = Some(record.clone());
                    }
                    saved
                })
                .collect(),
            values: self.values.for_file(self.configurations.holds_formulas()),
            configurations: self.configurations.clone(),
            properties: self.properties.clone(),
            datum_names: self.datum_names.clone(),
            rollback: self.saved_rollback(),
            next_feature: self.next_id,
            next_body: self.next_body,
            material: self.materials.material.clone(),
            appearances: self.materials.appearances.clone(),
            snapshots: self.materials.snapshots.clone(),
            pmi: self.pmi.clone(),
            display_states: self.materials.display_states.clone(),
        }
    }

    /// The part's `part.json`: features, parameters and references, no kernel state.
    pub fn part_json(&self) -> String {
        serde_json::to_string_pretty(&self.recipe()).expect("a recipe always serialises") + "\n"
    }

    /// The part's truth as JSON, what its hashes are computed over.
    pub fn truth(&self) -> Value {
        serde_json::to_value(self.recipe()).expect("a recipe always serialises")
    }

    /// The part's revision: BLAKE3 over its canonical truth, unsaved edits included.
    /// Equal while nothing about the part changed; a drawing or assembly pins it.
    pub fn semantic_hash(&self) -> ContentHash {
        manifest::semantic_hash(DocKind::Part, &self.truth())
    }

    /// The hash of what decides the part's shape: the truth without the part's and
    /// features' names, value expressions, text properties, repair-prompt labels, provenance, material, appearances,
    /// manufacturing information
    /// and the unused feature-id counter. Renaming a feature changes
    /// the semantic hash and not this one.
    pub fn geometry_hash(&self) -> ContentHash {
        shape_hash(&self.truth())
    }

    /// The part's manifest as this version writes it.
    pub fn manifest(&self) -> Manifest {
        self.manifest_for(&Extras::default())
    }

    fn manifest_for(&self, extras: &Extras<'_>) -> Manifest {
        let mut m = part_manifest(self.id, &self.kernel.backend(), &self.truth(), &self.settings, &self.view);
        if self.saves_display_data(extras) {
            m.sections.push(Section::optional(display::FOLDER));
        }
        if extras.thumbnail.is_some() {
            m.sections.push(Section::optional(manifest::THUMBNAIL));
        }
        m
    }

    /// True when saving writes display data: the settings ask for it, the save does not
    /// leave it out and the body's rebuild matches the current definitions.
    fn saves_display_data(&self, extras: &Extras<'_>) -> bool {
        self.settings.save_display_data && !extras.no_display_data && !self.is_dirty() && !self.live_bodies().is_empty()
    }

    /// The part file's bytes. With display data on, the part is tessellated at its
    /// coarse and standard levels; a kernel that cannot do it leaves the data out
    /// rather than failing the save.
    pub fn to_bytes(&self) -> Result<Vec<u8>, FileError> {
        self.to_bytes_with(&Extras::default())
    }

    /// The part file's bytes, with what `extras` adds or leaves out.
    pub fn to_bytes_with(&self, extras: &Extras<'_>) -> Result<Vec<u8>, FileError> {
        let mut entries = vec![Entry::new(PART, self.part_json().into_bytes())];
        entries.extend(optional_entries(&self.settings, &self.view)?);
        let mut m = self.manifest_for(extras);
        if self.saves_display_data(extras) {
            match self.display_data(&self.settings.viewport, &display::SAVED) {
                Some(Ok(data)) => {
                    for d in data {
                        entries.push(Entry::new(display::entry(d.level), d.mesh.encode(&d.made_by)));
                    }
                }
                _ => m.sections.retain(|s| s.name != display::FOLDER),
            }
        }
        if let Some(png) = extras.thumbnail {
            entries.push(Entry::stored(manifest::THUMBNAIL, png));
        }
        Ok(manifest::write_package(&m, &entries)?)
    }

    /// Writes the part file atomically: a failed save never leaves a half-written part.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), FileError> {
        Ok(soliddraft_document::save_atomic(path.as_ref(), &self.to_bytes()?)?)
    }

    /// Reads a part from a file's bytes and rebuilds it on `kernel`. A file from before
    /// the envelope gets the id derived from its content.
    pub fn from_bytes(bytes: &[u8], kernel: Arc<dyn Kernel>) -> Result<Document, FileError> {
        Self::from_bytes_with_input(bytes, kernel, None, |_| {})
    }

    /// Loads through the ordinary parser and rebuild loop, exposing authored input
    /// once before kernel work so a crash journal does not need the source file.
    /// A matching document on the same kernel may supply retained builds. Its cache
    /// budget is preserved; exact input keys decide reuse, and trimming waits until
    /// the rebuilt tree is live. `None` always starts with a cold cache.
    pub fn from_bytes_with_input(
        bytes: &[u8],
        kernel: Arc<dyn Kernel>,
        source: Option<&Document>,
        input: impl FnOnce(&Document),
    ) -> Result<Document, FileError> {
        let mut doc = Self::from_bytes_unbuilt(bytes, kernel)?;
        input(&doc);
        if let Some(source) = source.filter(|source| source.id == doc.id && Arc::ptr_eq(&source.kernel, &doc.kernel)) {
            doc.cache.set_budget(source.cache.budget());
            doc.cache.import(&source.cache);
        }
        doc.rebuild();
        Ok(doc)
    }

    fn from_bytes_unbuilt(bytes: &[u8], kernel: Arc<dyn Kernel>) -> Result<Document, FileError> {
        let mut package = Package::from_bytes(bytes)?;
        let Some(m) = package.manifest().cloned() else {
            let text = package
                .read_text(LEGACY_PART_TRUTH)?
                .ok_or_else(|| FileError::Archive("no manifest.json or recipe.json".into()))?;
            return Self::read_truth(&text, kernel, 1, None);
        };
        m.check(DocKind::Part, &[PART])?;
        let text = package.read_text(PART)?.ok_or_else(|| FileError::Archive("no part.json".into()))?;
        let mut doc = Self::read_truth(&text, kernel, NEWEST_FORMAT, Some(&m.format))?;
        doc.id = m.id;
        if let Ok(Some(text)) = package.read_text(settings::ENTRY) {
            doc.settings = Settings::from_json(&text).unwrap_or_default();
        }
        if let Ok(Some(text)) = package.read_text(view::ENTRY) {
            doc.view.visibility = Visibility::from_json(&text).unwrap_or_default();
        }
        if let Some(text) = package.read_text(saved_view::ENTRY)? {
            doc.view.saved =
                serde_json::from_str(&text).map_err(|err| FileError::Invalid(format!("saved views: {err}")))?;
        }
        Ok(doc)
    }

    /// Reads a part from its `part.json` (or a pre-envelope `recipe.json`) text and
    /// rebuilds it on `kernel`. Without a manifest there is no recorded id, so the part
    /// gets the id derived from its content, as a pre-envelope file does.
    pub fn from_part_json(text: &str, kernel: Arc<dyn Kernel>) -> Result<Document, FileError> {
        let mut doc = Self::read_truth(text, kernel, NEWEST_FORMAT, None)?;
        doc.rebuild();
        Ok(doc)
    }

    /// Decodes authored truth without starting a kernel operation.
    pub(crate) fn from_part_json_unbuilt(text: &str, kernel: Arc<dyn Kernel>) -> Result<Document, FileError> {
        Self::read_truth(text, kernel, NEWEST_FORMAT, None)
    }

    /// Reads truth text whose format's major version is at most `newest`.
    fn read_truth(
        text: &str,
        kernel: Arc<dyn Kernel>,
        newest: u32,
        manifest_format: Option<&str>,
    ) -> Result<Document, FileError> {
        let probe: Value = serde_json::from_str(text)?;
        let found = probe.get("format").and_then(|f| f.as_str()).unwrap_or_default().to_string();
        let Some(version) = format_major(&found, "soliddraft-part").filter(|m| (1..=newest).contains(m)) else {
            return Err(FileError::Format { found });
        };
        if let Some(tag) = manifest_format
            && format_major(tag, "soliddraft-part") != Some(version)
        {
            return Err(FileError::Invalid(format!("manifest format {tag} does not match part.json format {found}")));
        }
        let mut recipe: Recipe = serde_json::from_str(text)?;
        let mut ids: Vec<FeatureId> = recipe.features.iter().map(|f| f.id).collect();
        ids.sort();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err(FileError::Invalid("two features share an id".into()));
        }
        if ids.last().is_some_and(|id| id.0 >= recipe.next_feature) {
            return Err(FileError::Invalid("next_feature is not above every feature id".into()));
        }
        if let Some(r) = recipe.rollback
            && !ids.contains(&r)
        {
            return Err(FileError::Invalid(format!("the rollback bar names {r}, which is not a feature")));
        }
        // A feature may read one that was deleted (it reports that when it rebuilds),
        // but never one below it.
        if let Some((feature, dep)) = crate::inputs::misordered(&recipe.features) {
            return Err(FileError::Invalid(format!("{feature} reads {dep}, which comes after it")));
        }
        // A terminal field is the only one and the last feature; a file that puts CAD after it
        // is damaged, not a different part.
        Document::check_field_last(&recipe.features).map_err(|e| FileError::Invalid(e.to_string()))?;
        // An imported file is immutable: bytes that no longer match their hash are a damaged
        // file, not a different geometry.
        for f in &recipe.features {
            if let crate::FeatureKind::Scan(scan) = &f.kind
                && crate::sha256_hex(&scan.source.bytes) != scan.source.sha256
            {
                return Err(FileError::Invalid(format!("{} holds a scan that does not match its sha256", f.id)));
            }
            if let crate::feature::FeatureKind::ImportStep(i) = &f.kind
                && !i.hash_matches()
            {
                return Err(FileError::Invalid(format!("{} holds a file that does not match its sha256", f.id)));
            }
        }
        let mut materials = PartMaterials {
            material: recipe.material,
            appearances: recipe.appearances,
            snapshots: recipe.snapshots,
            display_states: recipe.display_states,
        };
        if version < EARLY_OPTIONS_VERSION
            && (recipe.values.uses_formula_settings()
                || recipe.configurations.uses_formula_settings()
                || has_extended_styles(&materials))
        {
            return Err(FileError::Invalid(format!("the part options require {PART_OPTIONS_FORMAT} or later")));
        }
        if version != EARLY_OPTIONS_VERSION
            && let Some(tag) = feature_options_after(&recipe.features, version)
        {
            return Err(FileError::Invalid(format!("the feature options require {tag} or later")));
        }
        // Pre-canonical files could carry graded walls under their original tags.
        if version > EARLY_OPTIONS_VERSION && version < GRADED_WALL_VERSION && uses_graded_walls(&recipe.features) {
            return Err(FileError::Invalid(format!("graded walls require {GRADED_WALL_FORMAT} or later")));
        }
        materials.snapshots.normalise();
        materials.check().map_err(FileError::Invalid)?;
        let mut doc = Document::new(recipe.name, kernel);
        doc.id = manifest::legacy_identity(DocKind::Part, &probe, Vec::new()).id;
        doc.materials = materials;
        // Every item must fit the ones before it, as it had to when it was added.
        for item in recipe.pmi.items {
            if let Some(problem) = doc.pmi.problem_with(&item, None) {
                return Err(FileError::Invalid(format!("{}: {problem}", item.id)));
            }
            doc.pmi.items.push(item);
        }
        doc.pmi.part = recipe.pmi.part;
        for (datum, name) in recipe.datum_names {
            doc.rename_datum(datum, &name).map_err(|e| FileError::Invalid(e.to_string()))?;
        }
        crate::configs::check_properties(&recipe.properties).map_err(FileError::Invalid)?;
        recipe.values.keep_file_rule(recipe.configurations.holds_formulas());
        let resolved =
            recipe.configurations.validate(&mut recipe.values, &recipe.features).map_err(FileError::Invalid)?;
        // Solving a variant also fills solver caches that saved sketches omit.
        let consistent = if recipe.configurations.is_empty() {
            resolved == recipe.features
        } else {
            serde_json::to_value(&resolved)? == serde_json::to_value(&recipe.features)?
        };
        if !consistent {
            return Err(FileError::Invalid("driven inputs disagree with their expressions".into()));
        }
        doc.values = recipe.values;
        doc.configurations = recipe.configurations;
        doc.properties = recipe.properties;
        doc.features = recipe.features;
        doc.rollback = recipe.rollback;
        doc.next_id = recipe.next_feature;
        doc.next_body = recipe.next_body.max(doc.features.iter().map(|f| f.body.saturating_add(1)).max().unwrap_or(1));
        Ok(doc)
    }

    /// Opens a part file and rebuilds it on `kernel`.
    pub fn open(path: impl AsRef<Path>, kernel: Arc<dyn Kernel>) -> Result<Document, FileError> {
        Self::from_bytes(&std::fs::read(path)?, kernel)
    }
}

fn uses_graded_walls(features: &[Feature]) -> bool {
    features.iter().any(|f| match &f.kind {
        crate::FeatureKind::Shell(wall) => wall.thickness.constant().is_none(),
        crate::FeatureKind::Thicken(wall) => wall.thickness.constant().is_none(),
        _ => false,
    })
}

fn feature_options_after(features: &[Feature], version: u32) -> Option<&'static str> {
    if version < HOLE_WIZARD_VERSION
        && features.iter().any(|f| matches!(&f.kind, crate::FeatureKind::Hole(h) if h.uses_wizard_format()))
    {
        Some(HOLE_WIZARD_FORMAT)
    } else if version < OPTIONS_VERSION && uses_options(features) {
        Some(OPTIONS_FORMAT)
    } else {
        None
    }
}

/// True when a feature uses what only [`OPTIONS_FORMAT`] carries: an extrusion's options,
/// or a reference to a thin extrusion's wall or end, which can outlive the extrusion.
fn uses_options(features: &[Feature]) -> bool {
    fn thin_name(v: &Value) -> bool {
        match v {
            Value::Object(map) => {
                matches!(map.get("kind").and_then(Value::as_str), Some("thin_wall" | "thin_end"))
                    || map.values().any(thin_name)
            }
            Value::Array(items) => items.iter().any(thin_name),
            _ => false,
        }
    }
    features.iter().any(|f| match &f.kind {
        crate::FeatureKind::Extrude(e) if !e.options.is_default() => true,
        // Large sources and fields hold no face names; a sketch holds one, for its plane.
        crate::FeatureKind::ImportStep(_)
        | crate::FeatureKind::Scan(_)
        | crate::FeatureKind::Lattice(_)
        | crate::FeatureKind::Gyroid(_) => false,
        crate::FeatureKind::Sketch(s) => serde_json::to_value(&s.plane).is_ok_and(|v| thin_name(&v)),
        kind => serde_json::to_value(kind).is_ok_and(|v| thin_name(&v)),
    })
}

/// What a save adds to the part file or leaves out of it, beyond the truth.
#[derive(Clone, Copy, Debug, Default)]
pub struct Extras<'a> {
    /// A PNG of the part to store as `thumbnail.png`.
    pub thumbnail: Option<&'a [u8]>,
    /// Leave out the display meshes whatever the settings say, for a quick copy such as
    /// autosave's recovery copy. The part opens from it with a rebuild.
    pub no_display_data: bool,
}

fn part_manifest(
    id: DocumentId,
    backend: &BackendInfo,
    truth: &Value,
    settings: &Settings,
    view: &PartView,
) -> Manifest {
    let mut m = Manifest::new(id, DocKind::Part, manifest::semantic_hash(DocKind::Part, truth));
    m.kernel = Some(format!("{} {}", backend.name, backend.version));
    m.format = truth.get("format").and_then(Value::as_str).unwrap_or(FORMAT).to_owned();
    m.geometry_hash = Some(shape_hash(truth));
    m.sections = vec![Section::required(PART)];
    if !settings.is_default() {
        m.sections.push(Section::optional(settings::ENTRY));
    }
    if !view.is_empty() {
        m.sections.push(Section::optional(view::ENTRY));
    }
    if !view.saved.is_empty() {
        m.sections.push(Section::optional(saved_view::ENTRY));
    }
    m
}

/// The settings and view entries, each only when it holds something.
fn optional_entries<'a>(
    settings: &Settings,
    view: &PartView,
) -> Result<impl Iterator<Item = Entry<'a>> + use<'a>, FileError> {
    let settings = (!settings.is_default()).then(|| Entry::new(settings::ENTRY, settings.to_json().into_bytes()));
    let saved_views = &view.saved;
    let view = (!view.is_empty()).then(|| Entry::new(view::ENTRY, view.to_json().into_bytes()));
    let saved_views = if saved_views.is_empty() {
        None
    } else {
        Some(Entry::new(saved_view::ENTRY, saved_view::encode(saved_views)?))
    };
    Ok(settings.into_iter().chain(view).chain(saved_views))
}

fn has_extended_styles(materials: &PartMaterials) -> bool {
    !materials.display_states.is_empty()
        || materials.material.iter().any(|assignment| assignment.body.is_some())
        || materials
            .appearances
            .iter()
            .any(|assignment| matches!(assignment.target, crate::AppearanceTarget::Body { .. }))
        || materials
            .snapshots
            .materials
            .iter()
            .any(|material| !material.description.is_empty() || material.damping_ratio.is_some())
}

fn shape_hash(truth: &Value) -> ContentHash {
    let mut truth = soliddraft_document::hash::without_format(truth);
    if let Value::Object(map) = &mut truth {
        for key in [
            "name",
            "datum_names",
            "material",
            "appearances",
            "snapshots",
            "pmi",
            "values",
            "configurations",
            "properties",
            "next_body",
            "display_states",
        ] {
            map.remove(key);
        }
        if let Some(Value::Array(features)) = map.get_mut("features") {
            // Preserve the historical hash encoding without including the id counter kept by undo.
            let next = features.iter().filter_map(|f| f.get("id").and_then(Value::as_u64)).max().unwrap_or(0) + 1;
            for f in features.iter_mut().filter_map(Value::as_object_mut) {
                f.remove("name");
                f.remove("via_assistant");
            }
            map.insert("next_feature".into(), next.into());
        }
    }
    strip_labels(&mut truth);
    ContentHash::of_entries([(PART, &truth)])
}

/// Removes every `label` key: the words a reference keeps for repair prompts.
fn strip_labels(v: &mut Value) {
    match v {
        Value::Object(map) => {
            map.remove("label");
            map.values_mut().for_each(strip_labels);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_labels),
        _ => {}
    }
}

impl Document {
    /// Restores a journal's starting part without reading any user file.
    pub fn from_journal_state(state: &serde_json::Value, kernel: Arc<dyn Kernel>) -> Result<Self, String> {
        if state["backend"] != serde_json::to_value(kernel.backend()).map_err(|e| e.to_string())? {
            return Err("journal kernel backend or version differs from this build".into());
        }
        let mut doc = Self::from_part_json_unbuilt(&state["part"].to_string(), kernel).map_err(|e| e.to_string())?;
        doc.id = serde_json::from_value(state["id"].clone()).map_err(|e| e.to_string())?;
        doc.settings = serde_json::from_value(state["settings"].clone()).map_err(|e| e.to_string())?;
        doc.undo = serde_json::from_value(state["undo"].clone()).map_err(|e| e.to_string())?;
        doc.redo = serde_json::from_value(state["redo"].clone()).map_err(|e| e.to_string())?;
        doc.rebuild();
        Ok(doc)
    }

    /// Authored state and history for a portable journal baseline or file input.
    /// No display caches, kernel handles or measurements are included.
    pub fn journal_state(&self) -> serde_json::Value {
        serde_json::json!({"part":self.truth(), "id":self.id, "settings":self.settings,
            "undo":self.undo, "redo":self.redo, "backend":self.kernel.backend()})
    }
}
