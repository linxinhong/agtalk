//! 测试工具：仅 #[cfg(test)] 编译，供各领域模块测试复用。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

/// 最小 mock HTTP server：按序返回预置响应；每个请求的「请求行 + 空行 + 正文」
/// 经 channel 回报给测试断言。返回 (base_url, 请求接收端, 线程句柄)。
pub fn mock_http_server(
    responses: Vec<String>,
) -> (
    String,
    std::sync::mpsc::Receiver<String>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        for body in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let trimmed = line.trim_end();
                if let Some(v) = trimmed
                    .to_ascii_lowercase()
                    .strip_prefix("content-length: ")
                {
                    content_length = v.trim().parse().unwrap();
                }
                if trimmed.is_empty() {
                    break;
                }
            }
            let mut req_body = String::new();
            if content_length > 0 {
                let mut buf = vec![0u8; content_length];
                reader.read_exact(&mut buf).unwrap();
                req_body = String::from_utf8_lossy(&buf).into_owned();
            }
            tx.send(format!("{}\n{}", request_line.trim_end(), req_body))
                .unwrap();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).unwrap();
        }
    });
    (format!("http://127.0.0.1:{}", port), rx, handle)
}
