use super::*;
use crate::mcp::ephemeral::{
    NativeMcpEphemeralConfiguration, NativeMcpEphemeralError, NativeMcpEphemeralOwner,
    tests::options,
};
use futures_executor::block_on;
use std::time::{Duration, Instant};

fn empty() -> NativeMcpEphemeralConfiguration {
    NativeMcpEphemeralConfiguration::decode(None).unwrap()
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}
fn owner() -> NativeMcpEphemeralOwner {
    let owner = NativeMcpEphemeralOwner::new(options().0).unwrap();
    block_on(owner.replace(empty(), CancellationToken::new(), deadline())).unwrap();
    owner
}

#[test]
fn exact_catalog_handoff_updates_readiness_and_next_replacement_checkpoint() {
    let owner = owner();
    let runtime = &owner.options.runtime;
    let expected = runtime.publication_checkpoint().unwrap();
    let candidate = runtime.prepare_candidate(vec![], &[]).unwrap();
    let prospective = candidate.publication_checkpoint();
    owner
        .catalog
        .sync_catalog_publication(&expected, &prospective, || {
            assert_eq!(owner.ready().unwrap_err(), NativeMcpEphemeralError::Busy);
            assert_eq!(
                block_on(owner.replace(empty(), CancellationToken::new(), deadline())).unwrap_err(),
                NativeMcpEphemeralError::Busy
            );
            runtime.publish_if(candidate, &expected)?;
            Ok(prospective.clone())
        })
        .unwrap();
    owner.ready().unwrap();
    owner.catalog.required_readiness(runtime).unwrap();
    block_on(owner.replace(empty(), CancellationToken::new(), deadline())).unwrap();
}

#[test]
fn failed_handoff_does_not_adopt_prospective_checkpoint_and_releases_reservation() {
    let owner = owner();
    let runtime = &owner.options.runtime;
    let expected = runtime.publication_checkpoint().unwrap();
    let candidate = runtime.prepare_candidate(vec![], &[]).unwrap();
    let prospective = candidate.publication_checkpoint();
    assert_eq!(
        owner
            .catalog
            .sync_catalog_publication(&expected, &prospective, || Err(Error::Cancelled))
            .unwrap_err(),
        Error::Cancelled
    );
    owner.ready().unwrap();
    assert!(
        runtime
            .publication_checkpoint()
            .unwrap()
            .same_selection(&expected)
    );
    block_on(owner.replace(empty(), CancellationToken::new(), deadline())).unwrap();
}

#[test]
fn close_after_catalog_commit_never_resurrects_active_generation() {
    let owner = owner();
    let runtime = &owner.options.runtime;
    let expected = runtime.publication_checkpoint().unwrap();
    let candidate = runtime.prepare_candidate(vec![], &[]).unwrap();
    let prospective = candidate.publication_checkpoint();
    owner
        .catalog
        .sync_catalog_publication(&expected, &prospective, || {
            runtime.publish_if(candidate, &expected)?;
            owner.close();
            Ok(prospective.clone())
        })
        .unwrap();
    assert!(owner.ready().is_err());
    assert!(lock(&owner.state).active.is_none());
    block_on(owner.settle(CancellationToken::new(), deadline())).unwrap();
}

#[test]
fn stale_or_foreign_catalog_handoff_does_not_call_commit() {
    let owner = owner();
    let foreign = self::owner();
    let expected = owner.options.runtime.publication_checkpoint().unwrap();
    let foreign_checkpoint = foreign.options.runtime.publication_checkpoint().unwrap();
    assert!(
        owner
            .catalog
            .sync_catalog_publication(&expected, &foreign_checkpoint, || panic!("foreign commit"))
            .is_err()
    );
    block_on(owner.replace(empty(), CancellationToken::new(), deadline())).unwrap();
    assert!(
        owner
            .catalog
            .sync_catalog_publication(&expected, &expected, || panic!("stale commit"))
            .is_err()
    );
    owner.ready().unwrap();
}
