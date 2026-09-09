//! Small bounded HTTP fixture; only canonical numeric loopback is authorized.

use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub(super) struct Gateway {
    pub address: SocketAddr,
    pub inference: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl Gateway {
    pub fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let inference = Arc::new(AtomicUsize::new(0));
        let stopped = Arc::clone(&stop);
        let requests = Arc::clone(&inference);
        let worker = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut count = 0;
            while !stopped.load(Ordering::Acquire) {
                if Instant::now() >= deadline || count > 16 {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                match listener.accept() {
                    Ok((mut connection, peer)) => {
                        assert!(peer.ip().is_loopback());
                        count += 1;
                        serve(&mut connection, &requests)?;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        });
        Self {
            address,
            inference,
            stop,
            worker: Some(worker),
        }
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

fn serve(connection: &mut TcpStream, inference: &AtomicUsize) -> io::Result<()> {
    connection.set_read_timeout(Some(Duration::from_millis(100)))?;
    connection.set_write_timeout(Some(Duration::from_secs(1)))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut request = Vec::new();
    let header_end = loop {
        if let Some(offset) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break offset + 4;
        }
        if request.len() > 8192 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        read_chunk(connection, &mut request, deadline)?;
    };
    let headers =
        std::str::from_utf8(&request[..header_end]).map_err(|_| io::ErrorKind::InvalidData)?;
    let line = headers
        .lines()
        .next()
        .ok_or(io::ErrorKind::InvalidData)?
        .to_owned();
    let body_len = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>())
        })
        .transpose()
        .map_err(|_| io::ErrorKind::InvalidData)?
        .unwrap_or(0);
    if body_len > 64 * 1024 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    while request.len() < header_end + body_len {
        read_chunk(connection, &mut request, deadline)?;
    }
    let (content_type, body) = if line.starts_with("GET /catalog ") {
        (
            "application/json",
            "{\"data\":[{\"id\":\"zai/glm-5.2\",\"type\":\"language\"}]}",
        )
    } else if line.starts_with("POST /inference ") {
        let _: serde_json::Value =
            serde_json::from_slice(&request[header_end..header_end + body_len])
                .map_err(|_| io::ErrorKind::InvalidData)?;
        inference.fetch_add(1, Ordering::Release);
        (
            "text/event-stream",
            concat!(
                "data: {\"type\":\"text-delta\",\"id\":\"answer\",\"delta\":\"local fixture answer\"}\n\n",
                "data: {\"type\":\"finish\",\"finishReason\":{\"unified\":\"stop\"}}\n\n",
            ),
        )
    } else {
        return Err(io::ErrorKind::InvalidData.into());
    };
    write!(
        connection,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    connection.flush()
}

fn read_chunk(
    connection: &mut TcpStream,
    request: &mut Vec<u8>,
    deadline: Instant,
) -> io::Result<()> {
    if Instant::now() >= deadline {
        return Err(io::ErrorKind::TimedOut.into());
    }
    let mut bytes = [0_u8; 4096];
    match connection.read(&mut bytes) {
        Ok(0) => Err(io::ErrorKind::UnexpectedEof.into()),
        Ok(count) if request.len() + count <= 72 * 1024 => {
            request.extend_from_slice(&bytes[..count]);
            Ok(())
        }
        Ok(_) => Err(io::ErrorKind::InvalidData.into()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}
