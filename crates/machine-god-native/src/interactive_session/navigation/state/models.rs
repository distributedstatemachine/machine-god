//! Child-local menu state and one explicitly authorized, owner-driven cache load.
use super::{Error, NativeInteractiveSession, Navigation, Route};
use crate::{
    NativeInteractiveSessionOptions, NativeManagedEditorIdentity, NativeManagedFormKind,
    NativeModelCatalogCacheError, NativeModelCatalogCacheSnapshot,
    NativeModelCatalogCacheState as State, NativeModelPicker, NativeModelPickerView,
};
use machine_god_core::{BoxFuture, CancellationToken, ManagedInspectSection};
use std::{
    task::{Context, Poll},
    time::Instant,
};

#[derive(Debug)]
pub struct NativeManagedModelsView<'a> {
    pub picker: NativeModelPickerView<'a>,
    pub state: State,
}

type Loading =
    BoxFuture<'static, Result<NativeModelCatalogCacheSnapshot, NativeModelCatalogCacheError>>;
pub(super) struct Models {
    pub(super) picker: NativeModelPicker,
    pub(super) chosen: Option<String>,
    state: State,
    loading: Option<Loading>,
    cancellation: CancellationToken,
    origin: Option<(Instant, u64)>,
}
impl Default for Models {
    fn default() -> Self {
        Self {
            picker: NativeModelPicker::unloaded(),
            chosen: None,
            state: State::Idle,
            loading: None,
            cancellation: CancellationToken::new(),
            origin: None,
        }
    }
}
impl Models {
    pub(super) fn view(&self) -> NativeManagedModelsView<'_> {
        NativeManagedModelsView {
            picker: self.picker.view(),
            state: self.state,
        }
    }
    pub(super) fn load(&mut self, options: &NativeInteractiveSessionOptions, force: bool) {
        let cache = options.catalog_cache.clone();
        let snapshot = cache.as_ref().map(|cache| cache.snapshot());
        if let Some(catalog) = snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.catalog.clone())
            .or_else(|| options.catalog.clone())
        {
            self.picker.replace_catalog(catalog);
            self.state = State::Ready;
        }
        if self.loading.is_some() {
            self.state = State::Loading;
            return;
        }
        let Some(cache) = cache else {
            return;
        };
        let snapshot = snapshot.expect("snapshot accompanies cache");
        // Continue from the cache's captured monotonic lower bound; a new menu
        // never invents elapsed time to bypass its failed-load cooldown.
        let (origin, base) = self
            .origin
            .get_or_insert_with(|| (Instant::now(), snapshot.last_attempt_ms.unwrap_or(0)));
        let now_ms =
            base.saturating_add(u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX));
        self.cancellation = CancellationToken::new();
        let cancellation = self.cancellation.clone();
        self.loading = Some(Box::pin(async move {
            if force {
                cache.refresh(now_ms, cancellation).await
            } else {
                cache.load(now_ms, cancellation).await
            }
        }));
        self.state = State::Loading;
    }
    pub(super) fn poll(&mut self, cx: &mut Context<'_>) -> bool {
        let Some(future) = &mut self.loading else {
            return false;
        };
        let Poll::Ready(result) = future.as_mut().poll(cx) else {
            return false;
        };
        self.loading = None;
        match result {
            Ok(snapshot) => {
                self.state = snapshot.state;
                if let Some(catalog) = snapshot.catalog {
                    self.picker.replace_catalog(catalog);
                }
            }
            Err(_) => self.state = State::Failed,
        }
        cx.waker().wake_by_ref();
        true
    }
}
impl Drop for Models {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

impl Navigation {
    pub(in crate::interactive_session) fn start_models(
        &mut self,
        options: &NativeInteractiveSessionOptions,
    ) {
        self.models.load(options, false);
    }

    pub(super) fn hydrate_catalog(owner: &mut NativeInteractiveSession) {
        let Some(catalog) = owner
            .options
            .catalog_cache
            .as_ref()
            .and_then(|cache| cache.snapshot().catalog)
        else {
            return;
        };
        owner.options.catalog = Some(catalog.clone());
        if owner
            .current
            .model_catalog()
            .as_ref()
            .is_none_or(|current| !std::sync::Arc::ptr_eq(current, &catalog))
        {
            // Retirement can reject this observation; retain it in options for
            // the next foreground and retry only while the owner is driven.
            let _ = owner.current.set_model_catalog(catalog);
        }
    }

    pub(super) fn open_models(&mut self, owner: &NativeInteractiveSession) -> Result<(), Error> {
        self.target()?;
        self.models.picker = NativeModelPicker::unloaded();
        self.models.chosen = None;
        self.models.load(&owner.options, false);
        self.result = None;
        self.route = Route::Models;
        Ok(())
    }
    pub(super) fn select_model(
        &mut self,
        owner: &mut NativeInteractiveSession,
    ) -> Result<(), Error> {
        let model = self
            .models
            .picker
            .selected()
            .ok_or(Error::NoSelection)?
            .model()
            .id()
            .to_owned();
        self.models.chosen = Some(model);
        self.route = Route::Form(NativeManagedFormKind::Configure);
        self.form = None;
        self.inspect(owner, ManagedInspectSection::Configuration, None)
    }
    pub(in crate::interactive_session) fn edit_models(
        &mut self,
        editor: &NativeManagedEditorIdentity,
        query: &str,
        cursor: usize,
    ) -> Result<(), Error> {
        if !self.open || *editor != self.frame().editor {
            return Err(Error::StaleFrame);
        }
        if self.route != Route::Models {
            return Err(Error::InvalidAction);
        }
        let result = self
            .models
            .picker
            .edit(query, cursor)
            .map_err(|_| Error::InvalidAction);
        self.change(false)?;
        self.error = result.as_ref().err().copied();
        result
    }
    pub(in crate::interactive_session) fn cancel_models(&self) {
        self.models.cancellation.cancel();
    }
    pub(in crate::interactive_session) fn models_pending(&self) -> bool {
        self.models.loading.is_some()
    }
}
