//! Shared real-turn fixture for the two scoped enumeration adapters.

use crate::{
    NATIVE_SESSION_METADATA_KEY, NativeConversation, NativeConversationTurn, NativeSessionMetadata,
    NativeWorkspaceAuthority, NativeWorkspaceContexts, NativeWorkspaceEntrySpec,
    NativeWorkspaceSource,
};
use futures_executor::block_on;
use machine_god_core::{
    BoxFuture, Capability, Engine, ModelEvent, PermissionDecision, PermissionError,
    PermissionGrantScope, PermissionHandler, PermissionRequest, SessionId, SessionIncarnationId,
    SessionRecord, SessionRevision, StopReason, Tool, ToolCall, ToolCallId, ToolContext,
};
use machine_god_testkit::{InMemorySessionStore, ModelProviderStep, ScriptedModelProvider};
use rustix::fs::{Mode, OFlags};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub struct Fixture {
    pub base: PathBuf,
    pub primary: PathBuf,
    pub additional: PathBuf,
    pub authority: NativeWorkspaceAuthority,
    pub contexts: Arc<NativeWorkspaceContexts>,
}

impl Fixture {
    pub fn new(label: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "machine-god-workspace-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&base).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        let primary = base.join("primary");
        let additional = base.join("additional");
        let state = base.join("state");
        for path in [&primary, &additional, &state] {
            std::fs::create_dir(path).unwrap();
        }
        std::fs::write(primary.join("primary.txt"), "primary").unwrap();
        std::fs::write(additional.join("additional.txt"), "additional").unwrap();
        let open = |path: &Path| {
            rustix::fs::open(
                path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW,
                Mode::empty(),
            )
            .unwrap()
        };
        let authority = NativeWorkspaceAuthority::open_blocking(
            open(&primary),
            primary.clone(),
            Some(open(&state)),
            state,
            vec![
                NativeWorkspaceEntrySpec::new(
                    NativeWorkspaceSource::new(additional.clone(), additional.clone(), true)
                        .unwrap(),
                    true,
                    false,
                )
                .unwrap(),
            ],
            false,
        )
        .unwrap();
        Self {
            base,
            primary,
            additional,
            authority,
            contexts: Arc::new(NativeWorkspaceContexts::new()),
        }
    }

    pub fn conversation(
        &self,
        tool: impl Tool + 'static,
        calls: Vec<ToolCall>,
        policy: Arc<Allow>,
    ) -> NativeConversation {
        let mut record = SessionRecord::empty(
            SessionId::new("session").unwrap(),
            SessionIncarnationId::new("incarnation").unwrap(),
        );
        record.revision = SessionRevision(1);
        record.metadata.insert(
            NATIVE_SESSION_METADATA_KEY.to_owned(),
            NativeSessionMetadata::default().to_value(),
        );
        let mut steps = Vec::new();
        if !calls.is_empty() {
            let mut events = calls
                .into_iter()
                .map(|call| ModelEvent::ToolCall { call })
                .collect::<Vec<_>>();
            events.push(ModelEvent::Stop {
                reason: StopReason::ToolCalls,
            });
            steps.push(ModelProviderStep::events(events));
        }
        for _ in 0..2 {
            steps.push(ModelProviderStep::events([ModelEvent::Stop {
                reason: StopReason::Completed,
            }]));
        }
        let engine = Engine::builder()
            .session_store(InMemorySessionStore::from_records(BTreeMap::from([(
                record.id.clone(),
                record,
            )])))
            .provider(ScriptedModelProvider::new("test", steps))
            .shared_permission_handler(policy)
            .tool(tool)
            .build()
            .unwrap();
        let session = block_on(engine.load_session(SessionId::new("session").unwrap()))
            .unwrap()
            .unwrap();
        NativeConversation::from_session(session)
            .unwrap()
            .with_workspace_contexts(self.authority.clone(), &self.contexts)
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.base).unwrap();
    }
}

#[derive(Default)]
pub struct Allow(pub Mutex<Vec<Capability>>);

impl PermissionHandler for Allow {
    fn authorize(
        &self,
        request: PermissionRequest,
    ) -> BoxFuture<'_, Result<PermissionDecision, PermissionError>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(request.capability);
            Ok(PermissionDecision::Allow {
                scope: PermissionGrantScope::Once,
            })
        })
    }
}

pub fn context(conversation: &NativeConversation, turn: &NativeConversationTurn) -> ToolContext {
    ToolContext {
        session_id: conversation.id(),
        session_incarnation_id: conversation.incarnation_id(),
        turn_id: turn.handle().id().clone(),
        call_id: ToolCallId::new("enumerate").unwrap(),
    }
}
