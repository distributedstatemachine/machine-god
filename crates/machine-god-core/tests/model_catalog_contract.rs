use futures_executor::block_on;
use machine_god_core::{
    AvailableModel, BoxFuture, CancellationToken, InvalidModelIdReason, MAX_MODEL_ID_BYTES,
    ModelCatalog, ModelCatalogAccess, ModelCatalogProvider, ProviderError, ProviderErrorKind,
    PublicCatalogReason, validate_model_id,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

fn model(id: &str) -> AvailableModel {
    AvailableModel::new(id).expect("test model ID is valid")
}

#[test]
fn model_ids_accept_exact_utf8_byte_bounds_without_normalizing() {
    let one_byte = AvailableModel::new("!").unwrap();
    assert_eq!(one_byte.id(), "!");

    let exact_limit = "~".repeat(MAX_MODEL_ID_BYTES);
    let model = AvailableModel::new(exact_limit.clone()).unwrap();
    assert_eq!(model.id(), exact_limit);

    let punctuation = AvailableModel::new("provider/model-v1?preview=true").unwrap();
    assert_eq!(punctuation.id(), "provider/model-v1?preview=true");

    for id in [
        "provider/模型 version 2".to_owned(),
        "é".repeat(512),
        "🦀".repeat(256),
        "\u{85}model\u{a0}".to_owned(),
        "cafe\u{301}".to_owned(),
    ] {
        assert_eq!(validate_model_id(&id), Ok(()));
        assert_eq!(AvailableModel::new(id.clone()).unwrap().id(), id);
    }
}

#[test]
fn model_ids_reject_each_invalid_category_without_reflection() {
    let empty = AvailableModel::new("").unwrap_err();
    assert_eq!(empty.reason(), InvalidModelIdReason::Empty);
    assert_eq!(empty.to_string(), "invalid model ID: must not be empty");

    let oversized_input = "x".repeat(MAX_MODEL_ID_BYTES + 1);
    let oversized = AvailableModel::new(oversized_input.clone()).unwrap_err();
    assert_eq!(oversized.reason(), InvalidModelIdReason::TooLong);
    assert!(!format!("{oversized:?}").contains(&oversized_input));
    assert!(!oversized.to_string().contains(&oversized_input));

    for invalid in ["line\nbreak", "delete\u{7f}", "model\0name", "a\tb"] {
        let error = AvailableModel::new(invalid).unwrap_err();
        assert_eq!(error.reason(), InvalidModelIdReason::ControlCharacter);
        assert!(!format!("{error:?}").contains(invalid));
        assert!(!error.to_string().contains(invalid));
    }

    for edge in [' ', '\t', '\r', '\n'] {
        for invalid in [
            format!("{edge}private-provider-id"),
            format!("private-provider-id{edge}"),
        ] {
            let error = validate_model_id(&invalid).unwrap_err();
            assert_eq!(error.reason(), InvalidModelIdReason::EdgeWhitespace);
            assert!(!format!("{error:?}: {error}").contains(&invalid));
        }
    }
    for id in [format!("{}a", "é".repeat(512)), "🦀".repeat(257)] {
        assert_eq!(
            validate_model_id(&id).unwrap_err().reason(),
            InvalidModelIdReason::TooLong
        );
    }
}

#[test]
fn model_ids_reject_every_ascii_control_but_accept_utf8_c1() {
    for byte in (0_u8..=31).chain(std::iter::once(127)) {
        let invalid = format!("before{}after", char::from(byte));
        assert_eq!(
            validate_model_id(&invalid).unwrap_err().reason(),
            InvalidModelIdReason::ControlCharacter
        );
    }
    for codepoint in 0x80..=0x9f {
        assert!(
            validate_model_id(&format!("provider/{}", char::from_u32(codepoint).unwrap())).is_ok()
        );
    }
}

#[test]
fn catalog_owns_models_and_preserves_provider_order() {
    let catalog = ModelCatalog::new(
        vec![model("provider/zeta"), model("provider/alpha")],
        ModelCatalogAccess::Authenticated,
    );

    let ids = catalog
        .models()
        .iter()
        .map(AvailableModel::id)
        .collect::<Vec<_>>();
    assert_eq!(ids, ["provider/zeta", "provider/alpha"]);
    assert_eq!(catalog.access(), ModelCatalogAccess::Authenticated);

    let owned = catalog.into_models();
    assert_eq!(owned[0].id(), "provider/zeta");
    assert_eq!(owned[1].id(), "provider/alpha");
}

#[test]
fn public_catalog_access_retains_its_exact_reason() {
    for reason in [
        PublicCatalogReason::NoCredential,
        PublicCatalogReason::AuthenticatedCredentialRejected,
    ] {
        let access = ModelCatalogAccess::PublicOnly { reason };
        let ModelCatalogAccess::PublicOnly { reason: observed } = access else {
            panic!("public access changed variant");
        };
        assert_eq!(observed, reason);

        let catalog = ModelCatalog::new(Vec::new(), access);
        assert_eq!(catalog.access(), access);
        assert!(catalog.models().is_empty());
    }
}

#[derive(Debug)]
struct StaticCatalogProvider;

impl ModelCatalogProvider for StaticCatalogProvider {
    fn name(&self) -> &'static str {
        "static-catalog"
    }

    fn list_models(
        &self,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelCatalog, ProviderError>> {
        Box::pin(async {
            Ok(ModelCatalog::new(
                vec![model("provider/model")],
                ModelCatalogAccess::Authenticated,
            ))
        })
    }
}

fn boxed_provider(provider: impl ModelCatalogProvider) -> Box<dyn ModelCatalogProvider> {
    Box::new(provider)
}

#[test]
fn catalog_provider_is_object_safe() {
    let provider = boxed_provider(StaticCatalogProvider);
    assert_eq!(provider.name(), "static-catalog");

    let catalog = block_on(provider.list_models(CancellationToken::new())).unwrap();
    assert_eq!(catalog.models(), [model("provider/model")]);
}

#[derive(Debug)]
struct CancellationAwareProvider;

impl ModelCatalogProvider for CancellationAwareProvider {
    fn name(&self) -> &'static str {
        "cancellation-aware"
    }

    fn list_models(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelCatalog, ProviderError>> {
        Box::pin(async move {
            cancellation.cancelled().await;
            Err(ProviderError::new(
                ProviderErrorKind::Cancelled,
                "model_catalog_cancelled",
                "model catalog request cancelled",
                false,
            ))
        })
    }
}

#[derive(Debug, Default)]
struct WakeCounter(AtomicUsize);

impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn provider_future_observes_the_supplied_cancellation_token() {
    let provider: Box<dyn ModelCatalogProvider> = Box::new(CancellationAwareProvider);
    let cancellation = CancellationToken::new();
    let mut future = provider.list_models(cancellation.clone());
    let wake_counter = Arc::new(WakeCounter::default());
    let waker = Waker::from(Arc::clone(&wake_counter));
    let mut context = Context::from_waker(&waker);

    assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    assert_eq!(wake_counter.0.load(Ordering::Relaxed), 0);

    assert!(cancellation.cancel());
    assert_eq!(wake_counter.0.load(Ordering::Relaxed), 1);

    let Poll::Ready(Err(error)) = future.as_mut().poll(&mut context) else {
        panic!("catalog future did not return its cancellation error");
    };
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
    assert_eq!(error.code, "model_catalog_cancelled");
    assert!(!error.retryable);
}
