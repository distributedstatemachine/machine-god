use super::*;

#[test]
fn client_secret_methods_bind_encoded_secret_only_to_token_request() {
    for method in ["client_secret_basic", "client_secret_post", "none"] {
        let registration = Registration {
            id: Secret::new(b"client:id").unwrap(),
            secret: Some(Secret::new(b"secret space").unwrap()),
            method: method.into(),
        };
        let Request::Token(bytes, authorization) =
            authenticated(&registration, "grant_type=refresh_token".into()).unwrap()
        else {
            panic!()
        };
        let body = String::from_utf8(bytes).unwrap();
        match method {
            "client_secret_basic" => {
                assert!(!body.contains("secret"));
                assert_eq!(
                    authorization.unwrap(),
                    format!("Basic {}", STANDARD.encode(b"client%3Aid:secret%20space"))
                );
            }
            "client_secret_post" => {
                assert!(authorization.is_none());
                assert!(body.contains("client_secret=secret%20space"));
            }
            _ => {
                assert!(authorization.is_none());
                assert!(!body.contains("secret"));
                assert!(body.contains("client_id=client%3Aid"));
            }
        }
    }
}
