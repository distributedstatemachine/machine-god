//! Pure, bounded native model preferences; encoding is not persistence.

use std::collections::BTreeMap;
use std::fmt;

use machine_god_core::validate_model_id;
use serde_json::{Value, json};

/// Reserved native record entry, independent of presentation and context metadata.
pub const NATIVE_MODEL_PREFERENCES_KEY: &str = "machine_god.model_preferences";
/// Pinned byte bound for a named reasoning effort.
pub const MAX_NATIVE_REASONING_EFFORT_BYTES: usize = 64;
/// Pinned maximum number of advertised named reasoning choices.
pub const MAX_NATIVE_REASONING_EFFORT_OPTIONS: usize = 16;

/// Fixed errors never retain or format rejected preference content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NativeModelPreferencesError {
    InvalidModel,
    InvalidEffort,
    InvalidCapabilities,
    Malformed,
    UnsupportedVersion,
}

impl fmt::Display for NativeModelPreferencesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidModel => "native model preference is invalid",
            Self::InvalidEffort => "native reasoning effort is invalid",
            Self::InvalidCapabilities => "native model capabilities are invalid",
            Self::Malformed => "native model preferences are malformed",
            Self::UnsupportedVersion => "native model preferences version is unsupported",
        })
    }
}

impl std::error::Error for NativeModelPreferencesError {}

/// Automatic reasoning or one validated opaque name; not a closed effort enum.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct NativeReasoningEffort {
    named: Option<String>,
}

impl NativeReasoningEffort {
    /// Normalizes only the ASCII-case-insensitive auto/adaptive/default aliases.
    ///
    /// # Errors
    /// Rejects empty, oversized, or non-ASCII-alphanumeric/`-_.` names.
    pub fn parse(raw: &str) -> Result<Self, NativeModelPreferencesError> {
        if raw.is_empty() || raw.len() > MAX_NATIVE_REASONING_EFFORT_BYTES {
            return Err(NativeModelPreferencesError::InvalidEffort);
        }
        if ["auto", "adaptive", "default"]
            .iter()
            .any(|alias| raw.eq_ignore_ascii_case(alias))
        {
            return Ok(Self::default());
        }
        if !raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(NativeModelPreferencesError::InvalidEffort);
        }
        Ok(Self {
            named: Some(raw.to_owned()),
        })
    }

    #[must_use]
    pub fn as_named(&self) -> Option<&str> {
        self.named.as_deref()
    }

    /// Returns the canonical storage label, not a terminal-escaped presentation.
    #[must_use]
    pub fn label(&self) -> &str {
        self.as_named().unwrap_or("auto")
    }
}

impl fmt::Debug for NativeReasoningEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeReasoningEffort")
            .field("automatic", &self.named.is_none())
            .finish_non_exhaustive()
    }
}

/// Explicit advertised controls. No model ID or tag implies these capabilities.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct NativeModelCapabilities {
    efforts: Vec<NativeReasoningEffort>,
    supports_fast: bool,
}

impl NativeModelCapabilities {
    /// Copies at most sixteen already validated named efforts in supplied order.
    /// Duplicates are preserved like the pinned catalog; automatic is implicit.
    ///
    /// # Errors
    /// Rejects excessive choices or an automatic entry in the named-choice list.
    pub fn new(
        efforts: &[NativeReasoningEffort],
        supports_fast: bool,
    ) -> Result<Self, NativeModelPreferencesError> {
        if efforts.len() > MAX_NATIVE_REASONING_EFFORT_OPTIONS
            || efforts.iter().any(|effort| effort.as_named().is_none())
        {
            return Err(NativeModelPreferencesError::InvalidCapabilities);
        }
        Ok(Self {
            efforts: efforts.to_vec(),
            supports_fast,
        })
    }

    #[must_use]
    pub fn reasoning_efforts(&self) -> &[NativeReasoningEffort] {
        &self.efforts
    }

    #[must_use]
    pub const fn supports_fast(&self) -> bool {
        self.supports_fast
    }
}

impl fmt::Debug for NativeModelCapabilities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeModelCapabilities")
            .field("effort_count", &self.efforts.len())
            .field("supports_fast", &self.supports_fast)
            .finish_non_exhaustive()
    }
}

/// Requested settings, which may differ from controls effective for a model.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeModelPreferences {
    model: String,
    effort: NativeReasoningEffort,
    requested_fast: bool,
}

impl Default for NativeModelPreferences {
    fn default() -> Self {
        Self {
            model: crate::AI_GATEWAY_DEFAULT_MODEL.to_owned(),
            effort: NativeReasoningEffort::default(),
            requested_fast: false,
        }
    }
}

impl fmt::Debug for NativeModelPreferences {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeModelPreferences")
            .finish_non_exhaustive()
    }
}

/// Outcome of the pinned fast toggle; unsupported is explicitly not a mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeFastModeChange {
    Enabled,
    Disabled,
    Unsupported,
}

/// Borrowed effective controls; resolving never changes requested preferences.
#[derive(Clone, Copy)]
pub struct NativeEffectiveModelPreferences<'a> {
    model: &'a str,
    effort: Option<&'a NativeReasoningEffort>,
    fast: bool,
}

impl NativeEffectiveModelPreferences<'_> {
    #[must_use]
    pub const fn model(&self) -> &str {
        self.model
    }

    #[must_use]
    pub const fn effort(&self) -> Option<&NativeReasoningEffort> {
        self.effort
    }

    #[must_use]
    pub const fn fast(&self) -> bool {
        self.fast
    }
}

impl fmt::Debug for NativeEffectiveModelPreferences<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeEffectiveModelPreferences")
            .finish_non_exhaustive()
    }
}

impl NativeModelPreferences {
    /// Validates before allocating the bounded model copy.
    ///
    /// # Errors
    /// Rejects models outside the shared core model-ID contract.
    pub fn new(
        model: &str,
        effort: NativeReasoningEffort,
        requested_fast: bool,
    ) -> Result<Self, NativeModelPreferencesError> {
        validate_model_id(model).map_err(|_| NativeModelPreferencesError::InvalidModel)?;
        Ok(Self {
            model: model.to_owned(),
            effort,
            requested_fast,
        })
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    #[must_use]
    pub const fn effort(&self) -> &NativeReasoningEffort {
        &self.effort
    }

    #[must_use]
    pub const fn requested_fast(&self) -> bool {
        self.requested_fast
    }

    /// Direct selection preserves requested effort and fast, including stale values.
    ///
    /// # Errors
    /// Invalid input leaves every preference unchanged.
    pub fn set_model(&mut self, model: &str) -> Result<(), NativeModelPreferencesError> {
        validate_model_id(model).map_err(|_| NativeModelPreferencesError::InvalidModel)?;
        model.clone_into(&mut self.model);
        Ok(())
    }

    pub fn set_effort(&mut self, effort: NativeReasoningEffort) {
        self.effort = effort;
    }

    pub fn toggle_fast(&mut self, capabilities: &NativeModelCapabilities) -> NativeFastModeChange {
        if self.requested_fast {
            self.requested_fast = false;
            NativeFastModeChange::Disabled
        } else if capabilities.supports_fast {
            self.requested_fast = true;
            NativeFastModeChange::Enabled
        } else {
            NativeFastModeChange::Unsupported
        }
    }

    /// Applies picker defaults only to supported controls: auto effort, fast on.
    /// Unsupported controls preserve the prior requested values, as in the pin.
    ///
    /// # Errors
    /// Invalid model input leaves all preferences unchanged.
    pub fn select_from_picker(
        &mut self,
        model: &str,
        capabilities: &NativeModelCapabilities,
    ) -> Result<(), NativeModelPreferencesError> {
        self.set_model(model)?;
        if !capabilities.efforts.is_empty() {
            self.effort = NativeReasoningEffort::default();
        }
        if capabilities.supports_fast {
            self.requested_fast = true;
        }
        Ok(())
    }

    #[must_use]
    pub fn effective(
        &self,
        capabilities: &NativeModelCapabilities,
    ) -> NativeEffectiveModelPreferences<'_> {
        NativeEffectiveModelPreferences {
            model: &self.model,
            effort: capabilities
                .efforts
                .contains(&self.effort)
                .then_some(&self.effort),
            fast: self.requested_fast && capabilities.supports_fast,
        }
    }

    /// Reads only the reserved entry; absence is distinct from explicit defaults.
    ///
    /// # Errors
    /// Rejects malformed present entries without traversing unrelated metadata.
    pub fn from_metadata(
        metadata: &BTreeMap<String, Value>,
    ) -> Result<Option<Self>, NativeModelPreferencesError> {
        metadata
            .get(NATIVE_MODEL_PREFERENCES_KEY)
            .map(Self::from_value)
            .transpose()
    }

    /// Decodes four required fields from a borrowed value without cloning its tree.
    ///
    /// # Errors
    /// Rejects unknown versions/fields, missing fields, wrong types and invalid values.
    pub fn from_value(value: &Value) -> Result<Self, NativeModelPreferencesError> {
        let object = value
            .as_object()
            .ok_or(NativeModelPreferencesError::Malformed)?;
        let version = object
            .get("schema_version")
            .and_then(Value::as_u64)
            .ok_or(NativeModelPreferencesError::Malformed)?;
        if version != 1 {
            return Err(NativeModelPreferencesError::UnsupportedVersion);
        }
        if object.len() != 4 {
            return Err(NativeModelPreferencesError::Malformed);
        }
        let model = object
            .get("model")
            .and_then(Value::as_str)
            .ok_or(NativeModelPreferencesError::Malformed)?;
        let effort = object
            .get("effort")
            .and_then(Value::as_str)
            .ok_or(NativeModelPreferencesError::Malformed)?;
        let fast = object
            .get("fast_mode")
            .and_then(Value::as_bool)
            .ok_or(NativeModelPreferencesError::Malformed)?;
        Self::new(model, NativeReasoningEffort::parse(effort)?, fast)
    }

    /// Returns a bounded value only; does not write settings or session records.
    #[must_use]
    pub fn to_value(&self) -> Value {
        json!({"schema_version": 1, "model": self.model, "effort": self.effort.label(), "fast_mode": self.requested_fast})
    }
}
