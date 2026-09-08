use super::{
    NativeSessionCatalogEntry, NativeSessionCatalogError as Error,
    NativeSessionCatalogErrorKind as Kind, NativeSessionCatalogPage, NativeSessionCatalogQuery,
    exact_from_store, list_from_store,
};
use crate::{
    NativeEnvironment,
    root_selection::{ExistingSessionStoreError, open_existing_session_store},
    state_environment::{ProcessStateEnvironmentReader, capture_state_environment},
};
use machine_god_core::{BoxFuture, SessionId};

/// No-create state-only observation. An absent hierarchy returns an empty page.
/// All native work occurs synchronously on first poll, never at construction.
#[must_use]
pub fn list_native_session_catalog(
    environment: NativeEnvironment,
    query: NativeSessionCatalogQuery,
) -> BoxFuture<'static, Result<NativeSessionCatalogPage, Error>> {
    Box::pin(async move { list(&environment, &query) })
}
/// Captures only `XDG_STATE_HOME`, falling back to HOME only when missing/empty,
/// on first poll. It never reads config or discovers a workspace association.
#[must_use]
pub fn list_process_session_catalog(
    query: NativeSessionCatalogQuery,
) -> BoxFuture<'static, Result<NativeSessionCatalogPage, Error>> {
    Box::pin(async move {
        let environment = capture_state_environment(&mut ProcessStateEnvironmentReader);
        list(&environment, &query)
    })
}

/// Resolves the current directory on first poll and applies its canonical
/// workspace spelling before state-only listing. Never creates state roots.
#[must_use]
pub fn list_process_current_workspace_session_catalog(
    query: NativeSessionCatalogQuery,
) -> BoxFuture<'static, Result<NativeSessionCatalogPage, Error>> {
    Box::pin(async move {
        let workspace = std::fs::canonicalize(".").map_err(|_| Error::new(Kind::Unavailable))?;
        let query = query
            .with_workspace(&workspace)
            .map_err(|_| Error::new(Kind::Unavailable))?;
        let environment = capture_state_environment(&mut ProcessStateEnvironmentReader);
        list(&environment, &query)
    })
}
/// Exact-ID metadata projection without enumeration or creating missing state.
#[must_use]
pub fn inspect_native_session_catalog_entry(
    environment: NativeEnvironment,
    id: SessionId,
) -> BoxFuture<'static, Result<Option<NativeSessionCatalogEntry>, Error>> {
    Box::pin(async move { exact(&environment, &id) })
}
/// State-only process capture and exact-ID projection, inert before first poll.
#[must_use]
pub fn inspect_process_session_catalog_entry(
    id: SessionId,
) -> BoxFuture<'static, Result<Option<NativeSessionCatalogEntry>, Error>> {
    Box::pin(async move {
        let environment = capture_state_environment(&mut ProcessStateEnvironmentReader);
        exact(&environment, &id)
    })
}
fn list(
    environment: &NativeEnvironment,
    query: &NativeSessionCatalogQuery,
) -> Result<NativeSessionCatalogPage, Error> {
    let store = open_existing_session_store(environment).map_err(map_root_error)?;
    store.as_ref().map_or_else(
        || Ok(NativeSessionCatalogPage::empty()),
        |store| list_from_store(store, query),
    )
}
fn exact(
    environment: &NativeEnvironment,
    id: &SessionId,
) -> Result<Option<NativeSessionCatalogEntry>, Error> {
    let store = open_existing_session_store(environment).map_err(map_root_error)?;
    store
        .as_ref()
        .map_or(Ok(None), |store| exact_from_store(store, id))
}
fn map_root_error(error: ExistingSessionStoreError) -> Error {
    Error::new(match error {
        ExistingSessionStoreError::InvalidEnvironment => Kind::InvalidEnvironment,
        ExistingSessionStoreError::UnsafeStateRoot => Kind::UnsafeStateRoot,
        ExistingSessionStoreError::Unavailable => Kind::Unavailable,
    })
}
