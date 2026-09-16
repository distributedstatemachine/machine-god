use super::*;
use crate::{
    AiGatewayModelCatalogAccessMode, AiGatewayModelCatalogProvider,
    AiGatewayModelCatalogRequestAccess, AiGatewayModelCatalogTransport,
    AiGatewayModelCatalogTransportError, AiGatewayModelCatalogTransportResponse,
};
use machine_god_core::{BoxFuture, CancellationToken};
use std::time::Instant;

struct Transport(Vec<u8>);
impl AiGatewayModelCatalogTransport for Transport {
    fn get(
        &self,
        _: AiGatewayModelCatalogRequestAccess,
        _: Instant,
        _: CancellationToken,
    ) -> BoxFuture<
        '_,
        Result<AiGatewayModelCatalogTransportResponse, AiGatewayModelCatalogTransportError>,
    > {
        Box::pin(async {
            Ok(AiGatewayModelCatalogTransportResponse::new(
                200,
                self.0.clone(),
            ))
        })
    }
    fn wait_until(&self, _: Instant) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
fn catalog(ids: &[&str]) -> Arc<NativeModelCatalog> {
    // The real catalog orders these otherwise-equivalent providers by release
    // date, not response order. Deliberately establish the desired source order.
    let entries: Vec<_> = ids.iter().enumerate().map(|(index, id)| {
        serde_json::json!({"id":id,"type":"language","tags":["tool-use"],"released":ids.len() - index})
    }).collect();
    let body = serde_json::to_vec(&serde_json::json!({"data": entries})).unwrap();
    let provider = AiGatewayModelCatalogProvider::new(
        AiGatewayModelCatalogAccessMode::PublicOnly,
        Arc::new(Transport(body)),
    );
    Arc::new(
        futures_executor::block_on(provider.list_model_details(CancellationToken::new())).unwrap(),
    )
}
fn ids(picker: &NativeModelPicker) -> Vec<String> {
    picker
        .view()
        .rows()
        .map(|entry| entry.model().id().to_owned())
        .collect()
}

#[test]
fn query_entered_before_catalog_arrival_is_retained_and_applied() {
    let mut picker = NativeModelPicker::unloaded();
    picker.edit("second", 3).unwrap();
    assert_eq!(picker.view().rows().len(), 0);
    picker.replace_catalog(catalog(&["vendor/first", "vendor/second"]));
    assert_eq!(picker.view().query, "second");
    assert_eq!(picker.view().cursor, 3);
    assert_eq!(picker.selected().unwrap().model().id(), "vendor/second");
}

#[test]
fn query_ranking_reuses_exact_and_fuzzy_semantics_with_stable_ties() {
    let mut picker = NativeModelPicker::new(catalog(&[
        "vendor/model-extra",
        "vendor/MODEL",
        "other/model",
    ]));
    assert_eq!(ids(&picker).len(), 3);
    picker.edit("model", 5).unwrap();
    assert_eq!(
        ids(&picker),
        ["vendor/MODEL", "other/model", "vendor/model-extra"]
    );
    picker.edit("VENDOR/MODEL", 12).unwrap();
    assert_eq!(picker.selected().unwrap().model().id(), "vendor/MODEL");
    picker.edit("unmatchable-value", 17).unwrap();
    assert!(picker.view().rows().next().is_none());
    picker.move_selection(false);
    assert!(picker.selected().is_none());
}

#[test]
fn exact_id_survives_catalog_reordering_but_not_removal_or_query_change() {
    let mut picker = NativeModelPicker::new(catalog(&["one/a", "two/b", "three/c"]));
    picker.move_selection(false);
    assert_eq!(picker.selected().unwrap().model().id(), "two/b");
    picker.edit("", 0).unwrap();
    assert_eq!(picker.view().selected, Some(1));
    picker.replace_catalog(catalog(&["two/b", "three/c", "one/a"]));
    assert_eq!(picker.selected().unwrap().model().id(), "two/b");
    assert_eq!(picker.view().selected, Some(0));
    picker.replace_catalog(catalog(&["three/c", "one/a"]));
    assert_eq!(picker.selected().unwrap().model().id(), "three/c");
    picker.edit("one", 3).unwrap();
    assert_eq!(picker.selected().unwrap().model().id(), "one/a");
    picker.replace_catalog(catalog(&["three/c"]));
    assert_eq!(picker.view().query, "one");
    assert!(picker.selected().is_none());
}

#[test]
fn rejected_utf8_cursor_or_oversized_query_preserves_selection_and_text() {
    let mut picker = NativeModelPicker::new(catalog(&["one/a", "two/b"]));
    picker.move_selection(false);
    for (query, cursor, error) in [
        ("α🙂", 1, NativeModelPickerError::InvalidCursor),
        ("query", 99, NativeModelPickerError::InvalidCursor),
        ("\0", 0, NativeModelPickerError::InvalidQuery),
    ] {
        assert_eq!(picker.edit(query, cursor), Err(error));
        assert_eq!(picker.view().query, "");
        assert_eq!(picker.selected().unwrap().model().id(), "two/b");
    }
    assert_eq!(
        picker.edit(&"x".repeat(MAX_NATIVE_MODEL_PICKER_QUERY_BYTES + 1), 0),
        Err(NativeModelPickerError::InvalidQuery)
    );
    picker.edit("private-query", 13).unwrap();
    assert!(!format!("{picker:?}").contains("private-query"));
    assert!(!format!("{:?}", picker.view()).contains("private-query"));
}

#[test]
fn selection_clamps_and_query_accepts_its_exact_byte_limit() {
    let mut picker = NativeModelPicker::new(catalog(&["one/a", "two/b"]));
    picker.move_selection(true);
    assert_eq!(picker.view().selected, Some(0));
    picker.move_selection(false);
    picker.move_selection(false);
    assert_eq!(picker.view().selected, Some(1));
    let query = "α".repeat(MAX_NATIVE_MODEL_PICKER_QUERY_BYTES / 2);
    picker.edit(&query, query.len()).unwrap();
    assert_eq!(
        picker.view().query.len(),
        MAX_NATIVE_MODEL_PICKER_QUERY_BYTES
    );
    picker.edit(&query, 2).unwrap();
    assert_eq!(picker.view().cursor, 2);
    picker.edit("", 0).unwrap();
    assert_eq!(picker.view().selected, Some(0));
}
