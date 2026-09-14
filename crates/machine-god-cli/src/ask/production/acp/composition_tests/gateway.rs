//! Bounded, joined loopback HTTP peer for the actual native HTTP transports.
use super::support::KEY;
use serde_json::Value;
use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(super) struct Request {
    pub method: String,
    pub authorized: bool,
    pub body: Value,
}
pub(super) struct Gateway {
    pub address: SocketAddr,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<io::Result<()>>>,
}
impl Gateway {
    pub fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !stopped.load(Ordering::Acquire) {
                if Instant::now() >= deadline {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                match listener.accept() {
                    Ok((mut stream, peer)) => {
                        if !peer.ip().is_loopback() || captured.lock().unwrap().len() >= 4 {
                            return Err(io::ErrorKind::InvalidData.into());
                        }
                        serve(&mut stream, &captured, deadline)?;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        });
        Self {
            address,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
    pub fn finish(mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap().unwrap();
    }
}
impl Drop for Gateway {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn serve(
    stream: &mut TcpStream,
    captured: &Mutex<Vec<Request>>,
    outer_deadline: Instant,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_millis(100)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    let deadline = outer_deadline.min(Instant::now() + Duration::from_secs(5));
    let mut bytes = Vec::new();
    let end = loop {
        if let Some(offset) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            break offset + 4;
        }
        if bytes.len() > 8192 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        read(stream, &mut bytes, deadline)?;
    };
    let headers = std::str::from_utf8(&bytes[..end]).map_err(|_| io::ErrorKind::InvalidData)?;
    let first = headers
        .lines()
        .next()
        .ok_or(io::ErrorKind::InvalidData)?
        .to_owned();
    let authorized =
        headers
            .lines()
            .filter_map(|line| line.split_once(':'))
            .any(|(name, value)| {
                name.eq_ignore_ascii_case("authorization")
                    && value.trim() == format!("Bearer {KEY}")
            });
    let length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>())
        })
        .transpose()
        .map_err(|_| io::ErrorKind::InvalidData)?
        .unwrap_or(0);
    if length > 64 * 1024 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    while bytes.len() < end + length {
        read(stream, &mut bytes, deadline)?;
    }
    let (method, body, content_type, response) = if first.starts_with("GET /catalog ") {
        (
            "catalog",
            Value::Null,
            "application/json",
            r#"{"data":[{"id":"zai/glm-5.2","type":"language"}]}"#,
        )
    } else if first.starts_with("POST /inference ") {
        let body = serde_json::from_slice(&bytes[end..end + length])
            .map_err(|_| io::ErrorKind::InvalidData)?;
        (
            "inference",
            body,
            "text/event-stream",
            concat!(
                "data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"local fixture answer\"}\n\n",
                "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n",
            ),
        )
    } else {
        return Err(io::ErrorKind::InvalidData.into());
    };
    // Keep only the authorization observation, never the raw request headers.
    captured.lock().unwrap().push(Request {
        method: method.into(),
        authorized,
        body,
    });
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
        response.len()
    )?;
    stream.flush()
}

fn read(stream: &mut TcpStream, bytes: &mut Vec<u8>, deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        return Err(io::ErrorKind::TimedOut.into());
    }
    let mut chunk = [0; 4096];
    match stream.read(&mut chunk) {
        Ok(0) => Err(io::ErrorKind::UnexpectedEof.into()),
        Ok(n) if bytes.len() + n <= 72 * 1024 => {
            bytes.extend_from_slice(&chunk[..n]);
            Ok(())
        }
        Ok(_) => Err(io::ErrorKind::InvalidData.into()),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::Interrupted | io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) =>
        {
            Ok(())
        }
        Err(e) => Err(e),
    }
}
