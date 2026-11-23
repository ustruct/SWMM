use std::{
    io::{BufRead, BufReader, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    time::Duration,
};

use anyhow_ext::{Context, Result};
use serde_json::Value;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);
const REPLY_TIMEOUT: Duration = Duration::from_secs(8);

pub struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

#[derive(Debug)]
pub struct CommandError(pub String);

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CommandError {}

impl Client {
    pub fn connect(port: u16) -> Result<Self> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .with_context(|| format!("Savage Wind isn't answering on port {port}"))?;
        stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
        stream.set_write_timeout(Some(REPLY_TIMEOUT))?;
        stream.set_nodelay(true)?;
        let writer = stream.try_clone()?;
        Ok(Self {
            reader: BufReader::new(stream),
            writer,
        })
    }

    pub fn call(&mut self, command: &str) -> Result<Value> {
        let line = command.replace(['\r', '\n'], " ");
        self.writer.write_all(line.trim().as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        let mut reply = String::new();
        if self.reader.read_line(&mut reply)? == 0 {
            anyhow_ext::bail!("Savage Wind closed the connection");
        }
        parse_reply(&reply)
    }
}

pub fn parse_reply(reply: &str) -> Result<Value> {
    let value: Value = serde_json::from_str(reply.trim()).context("Savage Wind sent a reply SWMM can't read")?;
    if value.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(value.get("data").cloned().unwrap_or(Value::Null));
    }
    let error = value
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("Savage Wind refused the command")
        .to_string();
    Err(CommandError(error).into())
}

pub fn quote(text: &str) -> String {
    let text = text.trim();
    if !text.is_empty() && !text.chars().any(|c| c.is_whitespace() || c == '"' || c == '\\') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

pub fn describe(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| format!("{k}: {}", describe(v)))
            .collect::<Vec<_>>()
            .join(", "),
        Value::Array(items) => items.iter().map(describe).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn replies() {
        assert_eq!(parse_reply("{\"ok\":true,\"data\":{\"pong\":true}}\n").unwrap()["pong"], Value::Bool(true));
        let err = parse_reply("{\"ok\":false,\"error\":\"Unknown command: x\"}").unwrap_err();
        assert_eq!(err.to_string(), "Unknown command: x");
        assert!(parse_reply("nope").is_err());
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("apple"), "apple");
        assert_eq!(quote(" stable wetland "), "\"stable wetland\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote(""), "\"\"");
    }

    #[test]
    fn round_trip() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut writer = stream;
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line, "set god_mode 1\n");
            writer
                .write_all(b"{\"ok\":true,\"data\":{\"god_mode\":true,\"applied\":\"live\"}}\n")
                .unwrap();
        });
        let mut client = Client::connect(port).unwrap();
        let data = client.call("set god_mode 1").unwrap();
        assert_eq!(data["applied"], "live");
        server.join().unwrap();
    }
}
