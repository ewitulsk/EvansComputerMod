//! A scripted network and disk (tests and the session recorder).

use ecm_web::fetch::{FetchError, Transport};
use std::collections::HashMap;

/// Pages by `host:port path`, files by path; every request is logged.
#[derive(Default, Clone)]
pub struct Net {
    pub http: HashMap<String, Vec<u8>>,
    pub files: HashMap<String, Vec<u8>>,
    pub log: Vec<String>,
}

impl Net {
    pub fn page(mut self, key: &str, status: &str, ct: &str, body: &str) -> Self {
        self.http.insert(key.into(), format!("HTTP/1.0 {status}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes());
        self
    }
    pub fn file(mut self, path: &str, body: &str) -> Self {
        self.files.insert(path.into(), body.as_bytes().to_vec());
        self
    }
}

impl Transport for Net {
    fn exchange(&mut self, host: &str, port: u16, request: &[u8]) -> Result<Vec<u8>, FetchError> {
        let req = String::from_utf8_lossy(request).into_owned();
        let path = req.split(' ').nth(1).unwrap_or("").to_string();
        let key = format!("{host}:{port} {path}");
        self.log.push(key.clone());
        self.http.get(&key).cloned().ok_or(FetchError::Connect(format!("{host}:{port}")))
    }
    fn read_file(&mut self, path: &str) -> Result<Vec<u8>, FetchError> {
        self.log.push(format!("file {path}"));
        self.files.get(path).cloned().ok_or(FetchError::NoFile(path.into()))
    }
    fn list_dir(&mut self, path: &str) -> Option<Vec<(String, bool)>> {
        let prefix = if path.ends_with('/') { path.to_string() } else { format!("{path}/") };
        let mut v: Vec<(String, bool)> = self.files.keys().filter_map(|k| k.strip_prefix(&prefix)).map(|n| (n.to_string(), false)).collect();
        v.sort();
        (!v.is_empty()).then_some(v)
    }
}

