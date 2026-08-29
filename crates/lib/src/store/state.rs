use std::collections::BTreeMap;

use crate::{
    Result,
    backend::{CacheScope, RecordView, StoreStateLifecycle, StoreStateRequest},
    entry::ID,
    instance::backend::Backend,
};

use super::ProjectionDescriptor;

/// Reserved key for the generic opaque whole-state projection.
pub const OPAQUE_STATE_KEY: &[u8] = &[0x00];

pub(crate) fn opaque_request(
    database: &ID,
    store: &str,
    descriptor: ProjectionDescriptor,
    source_key: Vec<u8>,
    scope: CacheScope,
) -> StoreStateRequest {
    StoreStateRequest {
        database: database.clone(),
        store: store.to_string(),
        lifecycle: StoreStateLifecycle::Derived,
        scope,
        projection: descriptor,
        source_key,
    }
}

pub(crate) async fn load_opaque(
    backend: &dyn Backend,
    view: &RecordView,
) -> Result<Option<Vec<u8>>> {
    backend.store_state_record_get(view, OPAQUE_STATE_KEY).await
}

pub(crate) async fn publish_opaque(
    backend: &dyn Backend,
    request: StoreStateRequest,
    bytes: Vec<u8>,
) -> Result<RecordView> {
    let token = backend.begin_store_state_staging(request).await?;
    let result = async {
        backend
            .stage_store_state_records(
                &token,
                BTreeMap::from([(OPAQUE_STATE_KEY.to_vec(), Some(bytes))]),
            )
            .await?;
        backend.publish_store_state(token.clone()).await
    }
    .await;
    if result.is_err() {
        let _ = backend.abort_store_state(token).await;
    }
    result
}
