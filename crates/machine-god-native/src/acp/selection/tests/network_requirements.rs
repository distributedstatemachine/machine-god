use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct CaptureFactory(Mutex<Vec<NativeMcpNetworkRequirement>>);

impl NativeAcpHostFactory for CaptureFactory {
    fn prepare(
        &self,
        _: PathBuf,
        network: NativeMcpNetworkRequirement,
        _: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeAcpPreparedHost, AcpSessionError>> {
        self.0.lock().unwrap().push(network);
        Box::pin(async { Err(AcpSessionError::Unavailable) })
    }

    fn list(
        &self,
        _: Option<PathBuf>,
        _: Option<NativeSessionCatalogCursor>,
        _: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeSessionCatalogPage, NativeSessionCatalogReadError>> {
        panic!("selection must not consult a catalog");
    }
}

#[test]
fn new_load_and_resume_pass_only_the_admitted_network_requirement_when_polled() {
    let cases: [(Option<&[u8]>, NativeMcpNetworkRequirement); 3] = [
        (None, NativeMcpNetworkRequirement::None),
        (
            Some(br#"[{"type":"http","name":"local","url":"http://127.0.0.1:8111/mcp","headers":[]}]"#),
            NativeMcpNetworkRequirement::LiteralOnly,
        ),
        (
            Some(br#"[{"type":"http","name":"remote","url":"https://mcp.example.test/","headers":[]}]"#),
            NativeMcpNetworkRequirement::SystemDns,
        ),
    ];
    run(async {
        for selection in [
            NativeAcpSessionSelection::New,
            NativeAcpSessionSelection::Load(SessionId::new("saved-session").unwrap()),
            NativeAcpSessionSelection::Resume(SessionId::new("saved-session").unwrap()),
        ] {
            for (raw, expected) in cases {
                let factory = Arc::new(CaptureFactory::default());
                let mut owner = NativeAcpSelectionOwner::new(factory.clone());
                owner
                    .request(
                        selection.clone(),
                        "/workspace".into(),
                        NativeMcpEphemeralConfiguration::decode(raw).unwrap(),
                        1,
                    )
                    .unwrap();
                assert!(factory.0.lock().unwrap().is_empty());
                assert!(matches!(
                    outcome(&mut owner).await,
                    NativeAcpSelectionOutcome::Rejected {
                        error: AcpSessionError::Unavailable,
                        candidate_may_have_persisted: false,
                        ..
                    }
                ));
                assert_eq!(*factory.0.lock().unwrap(), [expected]);
                owner.request_shutdown();
                assert!(owner.is_closed());
            }
        }
    });
}
