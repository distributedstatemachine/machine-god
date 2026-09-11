use super::*;

fn parsed(bytes: &[u8]) -> Result<Framing> {
    let mut slots = [httparse::EMPTY_HEADER; 16];
    let mut response = httparse::Response::new(&mut slots);
    response.parse(bytes).map_err(|_| McpHttpError::Protocol)?;
    framing(
        response.code.ok_or(McpHttpError::Protocol)?,
        response.version,
        response.headers,
        1024,
    )
}

#[test]
fn framing_accepts_identical_lengths_but_rejects_ambiguity_and_compression() {
    assert!(matches!(
        parsed(b"HTTP/1.1 200 OK\r\nContent-Length: 2, 2\r\nContent-Length: 2\r\n\r\n"),
        Ok(Framing::Length(2))
    ));
    for headers in [
        "Content-Length: 2, 3",
        "Content-Length: +2",
        "Content-Length: 18446744073709551616",
        "Content-Length: 2\r\nTransfer-Encoding: chunked",
        "Transfer-Encoding: gzip, chunked",
        "Transfer-Encoding: chunked\r\nTransfer-Encoding: chunked",
        "Content-Encoding: gzip",
    ] {
        assert!(parsed(format!("HTTP/1.1 200 OK\r\n{headers}\r\n\r\n").as_bytes()).is_err());
    }
    assert!(parsed(b"HTTP/1.0 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n").is_err());
    assert!(matches!(
        parsed(b"HTTP/1.1 200 OK\r\nContent-Length: 1025\r\n\r\n"),
        Err(McpHttpError::Limit)
    ));
}

#[test]
fn no_content_and_not_modified_do_not_wait_for_eof_bodies() {
    assert!(matches!(
        parsed(b"HTTP/1.1 204 No Content\r\n\r\n"),
        Ok(Framing::Done)
    ));
    assert!(matches!(
        parsed(b"HTTP/1.1 304 Not Modified\r\nContent-Length: 9000\r\n\r\n"),
        Ok(Framing::Done)
    ));
    assert!(parsed(b"HTTP/1.1 204 No Content\r\nContent-Length: 1\r\n\r\n").is_err());
}
