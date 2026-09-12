//! Private executable fixture exercising the real host-owned browser launcher.

use super::*;
use machine_god_native::NativeBackgroundUrlExecutable;
use std::collections::BTreeMap;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

pub(super) struct Browser {
    program: PathBuf,
    capture: PathBuf,
    ready: PathBuf,
}
impl Browser {
    pub(super) fn new(directory: &ScopedTestDirectory) -> Self {
        let program = directory.path().join("mock-browser");
        let capture = directory.path().join("browser-url");
        let ready = directory.path().join("browser-ready");
        fs::write(&program, b"#!/bin/sh\n[ \"$#\" -eq 1 ] || exit 81\n[ -z \"${HOME+x}\" ] || exit 82\nprintf '%s' \"$1\" > \"$MG_OAUTH_BROWSER_CAPTURE\" || exit 83\nprintf ready > \"$MG_OAUTH_BROWSER_READY\"\n").unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            program,
            capture,
            ready,
        }
    }

    pub(super) fn options(
        &self,
        options: NativeInteractiveSessionOptions,
    ) -> NativeInteractiveSessionOptions {
        options.with_background_url_opener(
            NativeBackgroundUrlExecutable::new(
                self.program.clone(),
                fs::File::open(&self.program).unwrap(),
            )
            .unwrap(),
            vec![
                (
                    "MG_OAUTH_BROWSER_CAPTURE".into(),
                    self.capture.clone().into_os_string(),
                ),
                (
                    "MG_OAUTH_BROWSER_READY".into(),
                    self.ready.clone().into_os_string(),
                ),
            ],
        )
    }

    pub(super) fn untouched(&self) -> bool {
        !self.capture.exists() && !self.ready.exists()
    }

    pub(super) async fn callback(
        &self,
        origin: &str,
        correct_state: bool,
    ) -> BTreeMap<String, String> {
        while !self.ready.exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let address = url::Url::parse(&fs::read_to_string(&self.capture).unwrap()).unwrap();
        assert_eq!(address.origin().ascii_serialization(), origin);
        assert_eq!(address.path(), "/authorize");
        let fields = address
            .query_pairs()
            .into_owned()
            .collect::<BTreeMap<_, _>>();
        assert_eq!(fields["client_id"], "local-client");
        assert_eq!(fields["response_type"], "code");
        assert_eq!(fields["code_challenge_method"], "S256");
        assert_eq!(fields["code_challenge"].len(), 43);
        assert_eq!(fields["resource"], format!("{origin}/mcp"));
        assert_eq!(fields["scope"], "read offline_access");
        let mut redirect = url::Url::parse(&fields["redirect_uri"]).unwrap();
        assert_eq!(redirect.host_str(), Some("127.0.0.1"));
        assert_eq!(redirect.path(), "/callback");
        redirect
            .query_pairs_mut()
            .append_pair("code", "local-code")
            .append_pair(
                "state",
                if correct_state {
                    &fields["state"]
                } else {
                    "wrong-state"
                },
            )
            .append_pair("iss", origin);
        let mut socket = TcpStream::connect((Ipv4Addr::LOCALHOST, redirect.port().unwrap()))
            .await
            .unwrap();
        socket
            .write_all(
                format!(
                    "GET {}?{} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
                    redirect.path(),
                    redirect.query().unwrap()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        socket.read_to_end(&mut response).await.unwrap();
        assert_eq!(response.starts_with(b"HTTP/1.1 200 OK"), correct_state);
        fields
    }
}
