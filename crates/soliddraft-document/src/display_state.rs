//! Named viewport settings that are independent of the geometry configuration.

use serde::{Deserialize, Serialize};

use crate::{ConfigurationId, DisplayStateId, DocumentId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    #[default]
    ShadedWithEdges,
    Shaded,
    Wireframe,
}

impl DisplayMode {
    pub const ALL: [Self; 3] = [Self::ShadedWithEdges, Self::Shaded, Self::Wireframe];

    pub fn label(self) -> &'static str {
        match self {
            Self::ShadedWithEdges => "Shaded with edges",
            Self::Shaded => "Shaded",
            Self::Wireframe => "Wireframe",
        }
    }
}

/// Every configuration, or chosen configurations. A null entry names Default.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", content = "configurations", rename_all = "snake_case")]
pub enum DisplayScope {
    #[default]
    All,
    Only(Vec<Option<ConfigurationId>>),
}

impl DisplayScope {
    pub fn includes(&self, config: Option<&ConfigurationId>) -> bool {
        match self {
            Self::All => true,
            Self::Only(configs) => configs.iter().any(|id| id.as_ref() == config),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayState<T> {
    pub id: DisplayStateId,
    pub name: String,
    #[serde(flatten)]
    pub scope: DisplayScope,
    pub mode: DisplayMode,
    pub view: T,
}

impl<T: Default> DisplayState<T> {
    pub fn new(name: String, scope: DisplayScope, mode: DisplayMode) -> Self {
        Self { id: DisplayStateId(DocumentId::new().to_string()), name, scope, mode, view: T::default() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
pub struct DisplayStates<T> {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    states: Vec<DisplayState<T>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    active: Vec<(Option<ConfigurationId>, DisplayStateId)>,
}

impl<T> Default for DisplayStates<T> {
    fn default() -> Self {
        Self { states: Vec::new(), active: Vec::new() }
    }
}

impl<T> DisplayStates<T> {
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    pub fn states(&self) -> &[DisplayState<T>] {
        &self.states
    }

    pub fn selected(&self, config: Option<&ConfigurationId>) -> Option<&DisplayState<T>> {
        let (_, id) = self.active.iter().find(|(saved, _)| saved.as_ref() == config)?;
        self.states.iter().find(|s| s.id == *id && s.scope.includes(config))
    }

    pub fn select(&mut self, config: Option<&ConfigurationId>, id: Option<DisplayStateId>) -> Result<(), String> {
        if let Some(id) = &id
            && !self.states.iter().any(|s| s.id == *id && s.scope.includes(config))
        {
            return Err("This display state is unavailable in the current configuration".into());
        }
        self.active.retain(|(c, _)| c.as_ref() != config);
        if let Some(id) = id {
            self.active.push((config.cloned(), id));
            self.active.sort_by(|a, b| a.0.cmp(&b.0));
        }
        Ok(())
    }

    pub fn put(&mut self, mut state: DisplayState<T>) -> Result<(), String> {
        state.name = state.name.trim().to_owned();
        self.check_state(&state)?;
        self.active.retain(|(config, id)| *id != state.id || state.scope.includes(config.as_ref()));
        match self.states.iter_mut().find(|s| s.id == state.id) {
            Some(old) => *old = state,
            None => self.states.push(state),
        }
        Ok(())
    }

    pub fn remove(&mut self, id: &DisplayStateId) {
        self.active.retain(|(_, active)| active != id);
        self.states.retain(|state| &state.id != id);
    }

    fn check_state(&self, state: &DisplayState<T>) -> Result<(), String> {
        if state.id.0.is_empty()
            || state.name.trim().is_empty()
            || state.name.len() > 128
            || state.name.eq_ignore_ascii_case("Default")
        {
            return Err("Enter a display state name other than Default, up to 128 bytes".into());
        }
        if self.states.iter().any(|s| s.id != state.id && s.name.eq_ignore_ascii_case(&state.name)) {
            return Err(format!("A display state named {} already exists", state.name));
        }
        if let DisplayScope::Only(configs) = &state.scope
            && (configs.is_empty() || configs.iter().enumerate().any(|(i, config)| configs[..i].contains(config)))
        {
            return Err("Choose one or more distinct configurations".into());
        }
        Ok(())
    }

    pub fn check(&self) -> Result<(), String> {
        for (i, state) in self.states.iter().enumerate() {
            self.check_state(state)?;
            if self.states[..i].iter().any(|s| s.id == state.id) {
                return Err("Duplicate display state ID".into());
            }
        }
        for (i, (config, id)) in self.active.iter().enumerate() {
            if self.active[..i].iter().any(|(c, _)| c == config)
                || !self.states.iter().any(|s| s.id == *id && s.scope.includes(config.as_ref()))
            {
                return Err("Invalid display state selection".into());
            }
        }
        Ok(())
    }
}
