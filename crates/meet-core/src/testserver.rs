//! Servidor HTTP mínimo para os testes (respostas pré-definidas, uma por conexão).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self { status: 200, headers: vec![], body: body.into() }
    }
    pub fn status(status: u16, body: &str) -> Self {
        Self { status, headers: vec![], body: body.as_bytes().to_vec() }
    }
}

pub struct Seen {
    pub request_line: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Atende `replies.len()` conexões em ordem e devolve o que recebeu.
pub fn serve(replies: Vec<Reply>) -> (String, JoinHandle<Vec<Seen>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let h = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for reply in replies {
            let (mut sock, _) = listener.accept().unwrap();
            let mut r = BufReader::new(sock.try_clone().unwrap());
            let mut request_line = String::new();
            r.read_line(&mut request_line).unwrap();
            let mut headers = Vec::new();
            let mut len = 0usize;
            loop {
                let mut l = String::new();
                r.read_line(&mut l).unwrap();
                let l = l.trim_end().to_string();
                if l.is_empty() {
                    break;
                }
                if let Some((k, v)) = l.split_once(':') {
                    if k.eq_ignore_ascii_case("content-length") {
                        len = v.trim().parse().unwrap();
                    }
                    headers.push((k.to_string(), v.trim().to_string()));
                }
            }
            let mut body = vec![0u8; len];
            r.read_exact(&mut body).unwrap();
            seen.push(Seen { request_line: request_line.trim_end().to_string(), headers, body });
            let mut head = format!("HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n", reply.status, reply.body.len());
            for (k, v) in &reply.headers {
                head.push_str(&format!("{k}: {v}\r\n"));
            }
            head.push_str("\r\n");
            sock.write_all(head.as_bytes()).unwrap();
            sock.write_all(&reply.body).unwrap();
        }
        seen
    });
    (base, h)
}
