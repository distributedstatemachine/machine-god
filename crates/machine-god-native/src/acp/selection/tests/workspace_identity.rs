use super::*;
use crate::{NativeRootSelection, PreparedNativeRoots};
use std::{fs, os::unix::fs::symlink};

async fn retire(prepared: NativeAcpPreparedHost) {
    let NativeAcpPreparedHost { host, .. } = prepared;
    assert!(cleanup::retire(host, None, false, 200).await.complete);
}

#[test]
fn prepared_host_rejects_foreign_primary_even_with_identical_canonical_spelling() {
    run(async {
        let factory = Factory::new();
        let mut first = factory
            .prepare(
                factory.workspace.clone(),
                NativeMcpNetworkRequirement::None,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let mut second = factory
            .prepare(
                factory.workspace.clone(),
                NativeMcpNetworkRequirement::None,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(first.validate().is_ok());
        assert!(second.validate().is_ok());
        std::mem::swap(&mut first.workspace, &mut second.workspace);
        assert!(matches!(
            first.validate(),
            Err(AcpSessionError::InvalidConfiguration)
        ));
        assert!(matches!(
            second.validate(),
            Err(AcpSessionError::InvalidConfiguration)
        ));
        std::mem::swap(&mut first.workspace, &mut second.workspace);
        retire(first).await;
        retire(second).await;
    });
}

#[test]
fn capture_rejects_replaced_same_path_descriptor_identity() {
    let factory = Factory::new();
    let environment = crate::NativeEnvironment::new(
        None,
        Some(
            factory
                .workspace
                .parent()
                .unwrap()
                .join("state")
                .into_os_string(),
        ),
        None,
    );
    let roots = PreparedNativeRoots::prepare(
        NativeRootSelection::from_environment(&environment, &factory.workspace).unwrap(),
    )
    .unwrap();
    let moved = factory.workspace.with_file_name("original-workspace");
    fs::rename(&factory.workspace, &moved).unwrap();
    fs::create_dir(&factory.workspace).unwrap();
    let replacement = PreparedNativeRoots::prepare(
        NativeRootSelection::from_environment(&environment, &factory.workspace).unwrap(),
    )
    .unwrap();
    let foreign = fixture::workspace_authority(&replacement);
    assert!(matches!(
        NativeAcpWorkspaceIdentity::capture(&roots, &foreign),
        Err(AcpSessionError::InvalidConfiguration)
    ));
}

struct SubstituteFactory(Arc<Factory>);
impl NativeAcpHostFactory for SubstituteFactory {
    fn prepare(
        &self,
        _: PathBuf,
        network: NativeMcpNetworkRequirement,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeAcpPreparedHost, AcpSessionError>> {
        self.0.prepare(self.0.workspace.clone(), network, cancel)
    }
    fn list(
        &self,
        workspace: Option<PathBuf>,
        cursor: Option<NativeSessionCatalogCursor>,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<NativeSessionCatalogPage, NativeSessionCatalogReadError>> {
        self.0.list(workspace, cursor, cancel)
    }
}

#[test]
fn selection_rejects_substituted_request_even_when_both_paths_name_the_same_root() {
    run(async {
        let factory = Arc::new(Factory::new());
        let alias = factory.workspace.parent().unwrap().join("alias");
        symlink(factory.workspace.parent().unwrap(), &alias).unwrap();
        let mut owner = NativeAcpSelectionOwner::new(Arc::new(SubstituteFactory(factory.clone())));
        owner
            .request(
                NativeAcpSessionSelection::New,
                alias.join("workspace"),
                empty(),
                1,
            )
            .unwrap();
        assert!(matches!(
            outcome(&mut owner).await,
            NativeAcpSelectionOutcome::Rejected {
                error: AcpSessionError::InvalidConfiguration,
                candidate_may_have_persisted: false,
                ..
            }
        ));
        assert!(owner.current().is_none());
        owner.request_shutdown();
        assert!(owner.is_closed());
    });
}

#[test]
fn prepared_alias_binding_survives_source_retarget_without_changing_authority() {
    run(async {
        let factory = Factory::new();
        let parent = factory.workspace.parent().unwrap();
        let alias = parent.join("alias");
        symlink(parent, &alias).unwrap();
        let prepared = factory
            .prepare(
                alias.join("workspace"),
                NativeMcpNetworkRequirement::None,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(prepared.workspace.requested, alias.join("workspace"));
        assert_eq!(prepared.host.workspace_root(), factory.workspace);
        let replacement = parent.join("replacement");
        fs::create_dir(&replacement).unwrap();
        fs::create_dir(replacement.join("workspace")).unwrap();
        fs::remove_file(&alias).unwrap();
        symlink(&replacement, &alias).unwrap();
        assert!(prepared.validate().is_ok());
        assert_eq!(prepared.host.workspace_root(), factory.workspace);
        retire(prepared).await;
    });
}
