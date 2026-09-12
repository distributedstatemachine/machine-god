use super::*;

#[test]
fn dropped_ready_batch_releases_identity_without_dropping_historical_startup() {
    run(async {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let mut selected = http_options(listener.local_addr().unwrap());
        let credentials = Credentials::new(
            selected.network.as_ref().unwrap().clone(),
            selected.workers.clone(),
        );
        selected
            .authentication
            .push(authentication(NativeMcpStartupAuthSource::Stored(
                credentials.service.clone(),
            )));
        let startup = NativeMcpStartup::new(selected).unwrap();
        credentials.seed(&startup.authentication_config("remote").unwrap(), i64::MAX);
        let (batch, _) = join(
            startup.build(
                NativeMcpStartupPhase::All,
                CancellationToken::new(),
                deadline(),
            ),
            reply(&listener, &discover(false)),
        )
        .await;
        assert!(batch.receipt().required_ready());
        let receipt = batch.receipt().clone();
        let generation = batch.servers()[0].authority_cancellations[3].clone();
        assert!(!generation.is_cancelled());
        drop(batch);
        assert!(generation.is_cancelled());
        assert!(receipt.cleanup_complete());
        assert!(startup.authentication_identities.lock().unwrap().is_empty());
        assert!(credentials.store.path().exists());
    });
}

#[test]
fn unpolled_startup_has_no_selected_identity_or_credential_effect() {
    let mut selected = http_options("127.0.0.1:34567".parse().unwrap());
    let credentials = Credentials::new(
        selected.network.as_ref().unwrap().clone(),
        selected.workers.clone(),
    );
    selected
        .authentication
        .push(authentication(NativeMcpStartupAuthSource::Stored(
            credentials.service.clone(),
        )));
    let startup = NativeMcpStartup::new(selected).unwrap();
    drop(startup.build(
        NativeMcpStartupPhase::All,
        CancellationToken::new(),
        deadline(),
    ));
    assert!(startup.authentication_identities.lock().unwrap().is_empty());
    assert!(!credentials.store.path().exists());
}
