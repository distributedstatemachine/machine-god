use super::{
    MAX_NATIVE_SKILL_QUERY_BYTES, MAX_NATIVE_SKILL_QUERY_ROWS, NativeSkillCatalogError as Error,
    NativeSkillDiagnostic, NativeSkillEntry, NativeSkillSelection, NativeSkillSnapshot, Result,
};

impl NativeSkillSnapshot {
    #[must_use]
    pub fn entries(&self) -> &[NativeSkillEntry] {
        &self.entries
    }
    #[must_use]
    pub fn diagnostics(&self) -> &[NativeSkillDiagnostic] {
        &self.diagnostics
    }
    #[must_use]
    pub const fn complete(&self) -> bool {
        self.complete
    }
    #[must_use]
    pub const fn generation(&self) -> &[u8; 32] {
        &self.generation
    }

    /// Resolves a name without choosing between duplicate locations. Exact
    /// location selection remains possible when unrelated discovery is partial.
    ///
    /// # Errors
    /// Returns ambiguity, incomplete-discovery, mismatch or not-found errors.
    pub fn resolve(
        &self,
        name: &str,
        location: Option<&std::path::Path>,
    ) -> Result<NativeSkillSelection> {
        if name.len() > crate::skills_metadata::MAX_NATIVE_SKILL_METADATA_NAME_BYTES
            || location
                .is_some_and(|path| path.as_os_str().len() > super::MAX_NATIVE_SKILL_PATH_BYTES)
        {
            return Err(Error::InvalidQuery);
        }
        if let Some(location) = location {
            let entry = self
                .entries
                .iter()
                .find(|entry| entry.location() == location)
                .ok_or(Error::NotFound)?;
            if entry.metadata.name != name {
                return Err(Error::NameLocationMismatch);
            }
            return Ok(entry.selection());
        }
        if !self.complete {
            return Err(Error::IncompleteDiscovery);
        }
        let mut matches = self
            .entries
            .iter()
            .filter(|entry| entry.metadata.name == name);
        let first = matches.next().ok_or(Error::NotFound)?;
        if matches.next().is_some() {
            return Err(Error::AmbiguousName);
        }
        Ok(first.selection())
    }

    /// Bounded ASCII-case-insensitive substring query in stable discovery order.
    ///
    /// # Errors
    /// Rejects query/row bounds instead of silently widening them.
    pub fn query(&self, query: &str, limit: usize) -> Result<Vec<&NativeSkillEntry>> {
        if query.len() > MAX_NATIVE_SKILL_QUERY_BYTES || limit > MAX_NATIVE_SKILL_QUERY_ROWS {
            return Err(Error::InvalidQuery);
        }
        let query = query.trim().to_ascii_lowercase();
        Ok(self
            .entries
            .iter()
            .filter(|entry| {
                entry.metadata.name.to_ascii_lowercase().contains(&query)
                    || entry
                        .metadata
                        .description
                        .to_ascii_lowercase()
                        .contains(&query)
                    || entry
                        .location()
                        .to_str()
                        .is_some_and(|path| path.to_ascii_lowercase().contains(&query))
            })
            .take(limit)
            .collect())
    }
}
