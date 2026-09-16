use super::*;
use crate::mcp::{
    lifetime::McpPeerLifetime, protocol::WireLimits, stdio_startup::NativeMcpStdioStartup,
};
use crate::reference_host::mcp::{
    NativeReferenceHostMcpEphemeralStartupOptions,
    tests::fixture::{Clock, Directory},
};
use std::{fs::File, sync::atomic::Ordering, time::Instant};

#[test]
fn parent_transport_reselection_preserves_captured_inputs_and_leaves_both_seeds_unchanged() {
    let directory = Directory::new();
    let clock = Arc::new(Clock::default());
    let environment = vec![("CAPTURED".into(), "original".into())];
    let stdio = Arc::new(
        NativeMcpStdioStartup::new(
            "/explicit-unexecuted-helper".into(),
            vec![],
            environment.clone(),
            Arc::new(File::open(&directory.0).unwrap()),
            WireLimits::default(),
        )
        .unwrap(),
    );
    let cancellation = CancellationToken::new();
    let options =
        NativeReferenceHostMcpOptions::new(Arc::new(NativeMcpContexts::new()), clock.clone())
            .with_ephemeral_startup(NativeReferenceHostMcpEphemeralStartupOptions {
                captured_environment: environment.clone(),
                stdio: Some(stdio.clone()),
                clock: clock.clone(),
                catalog_epoch: Instant::now(),
                owner_cancellation: cancellation.clone(),
                #[cfg(feature = "mcp-http")]
                network: None,
                peer_lifetime: McpPeerLifetime::OwnerControlled,
                max_retained_bytes: 1024 * 1024,
                max_retained_generations: 4,
            })
            .unwrap();
    let archive = Arc::new(NativeToolResultArchiveAdapter::new(Arc::new(
        crate::ToolResultArchive::from_root_descriptor(File::open(&directory.0).unwrap().into()),
    )));
    let (child, parent) = seeds(options, None, archive, None).unwrap();
    #[cfg(feature = "mcp-http")]
    let network = Arc::new(
        crate::mcp::network::NativeMcpNetwork::new(
            crate::mcp::network::McpResolverConfig::literal_only(),
            [3; 32],
            None,
            Arc::new(crate::mcp::clock::TokioMcpClock),
            CancellationToken::new(),
            4,
        )
        .unwrap(),
    );
    let selected = parent
        .select_ephemeral_network(
            #[cfg(feature = "mcp-http")]
            Some(network.clone()),
        )
        .unwrap();
    assert!(Arc::ptr_eq(&parent.0.inputs, &selected.0.inputs));
    let old = parent.0.options.ephemeral.as_ref().unwrap();
    let new = selected.0.options.ephemeral.as_ref().unwrap();
    assert!(Arc::ptr_eq(
        old.stdio.as_ref().unwrap(),
        new.stdio.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(&old.clock, &new.clock));
    assert_eq!(new.captured_environment, environment);
    assert_eq!(old.catalog_epoch, new.catalog_epoch);
    cancellation.cancel();
    assert!(old.owner_cancellation.is_cancelled());
    assert!(!new.owner_cancellation.is_cancelled());
    assert!(child.0.options.ephemeral.is_none());
    #[cfg(feature = "mcp-http")]
    {
        assert!(old.network.is_none());
        assert!(Arc::ptr_eq(new.network.as_ref().unwrap(), &network));
        let cleared = selected.select_ephemeral_network(None).unwrap();
        assert!(
            cleared
                .0
                .options
                .ephemeral
                .as_ref()
                .unwrap()
                .network
                .is_none()
        );
        assert!(
            selected
                .0
                .options
                .ephemeral
                .as_ref()
                .unwrap()
                .network
                .is_some()
        );
    }
    assert_eq!(clock.0.load(Ordering::Relaxed), 0);
}
