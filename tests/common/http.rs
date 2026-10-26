use serde_json::Value;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

pub struct Reply {
    pub path: String,
    pub status: u16,
    pub body: String,
}

pub struct Server {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    pub fn start(replies: Vec<Reply>) -> Self {
        Self::start_gated(replies, None)
    }

    pub fn start_paused(reply: Reply) -> (Self, mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        (
            Self::start_gated(vec![reply], Some((started_tx, release_rx))),
            started_rx,
            release_tx,
        )
    }

    fn start_gated(
        replies: Vec<Reply>,
        mut gate: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(false).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let thread = thread::spawn(move || {
            for reply in replies {
                let (mut stream, _) = listener.accept().expect("expected HTTP request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let (path, body) = read_request(&mut stream);
                assert_eq!(path, reply.path, "unexpected test HTTP path");
                captured
                    .lock()
                    .unwrap()
                    .push(serde_json::json!({"path": path, "body": body}));
                if let Some((started, release)) = gate.take() {
                    started.send(()).unwrap();
                    release.recv().unwrap();
                }
                let reason = if reply.status == 200 { "OK" } else { "Error" };
                write!(
                    stream,
                    "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    reply.status,
                    reason,
                    reply.body.len(),
                    reply.body
                )
                .unwrap();
            }
        });
        Self {
            url,
            requests,
            thread: Some(thread),
        }
    }
    pub fn finish(mut self) {
        self.thread.take().unwrap().join().unwrap();
    }

    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Wake accept if a test exits before consuming every scripted reply.
        if let Some(thread) = self.thread.take() {
            let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
            let _ = thread.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> (String, Value) {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let header_end;
    loop {
        let count = stream.read(&mut buffer).expect("read HTTP request");
        assert_ne!(count, 0, "client closed before HTTP headers");
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = position + 4;
            break;
        }
    }
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let path = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned();
    let length: usize = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().unwrap())
        })
        .unwrap_or(0);
    while bytes.len() < header_end + length {
        let count = stream.read(&mut buffer).expect("read HTTP body");
        assert_ne!(count, 0, "client closed before HTTP body");
        bytes.extend_from_slice(&buffer[..count]);
    }
    let body = if length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes[header_end..header_end + length]).expect("JSON request body")
    };
    (path, body)
}
