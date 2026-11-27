//! Where package files live: a directory (`fs`) or an S3-compatible bucket (`s3`:
//! DigitalOcean Spaces, MinIO, AWS).
//!
//! The S3 client is deliberately small: four calls (PUT, GET, DELETE of an object
//! and HEAD of the bucket) signed with AWS Signature Version 4 and sent with
//! `reqwest` over rustls, so nothing links against a system TLS library.

use async_trait::async_trait;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[async_trait]
pub trait Storage: Send + Sync {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> Result<(), String>;
    /// `Ok(None)` when there is no such object.
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String>;
    /// Deleting an object that is not there is not an error.
    async fn delete(&self, key: &str) -> Result<(), String>;
    /// Can the store be reached? Checked at startup.
    async fn check(&self) -> Result<(), String>;
}

/// Keys are `/`-separated names; none of the parts may be empty, `.` or `..`.
fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 900
        && key.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
        && !key.bytes().any(|b| b < 0x20 || b == b'\\')
}

// ------------------------------------------------------------------ fs

pub struct FsStorage {
    root: PathBuf,
}

impl FsStorage {
    pub fn new(dir: &str) -> Result<FsStorage, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{dir}: {e}"))?;
        Ok(FsStorage { root: PathBuf::from(dir) })
    }

    fn path(&self, key: &str) -> Result<PathBuf, String> {
        if !valid_key(key) {
            return Err(format!("bad storage key '{key}'"));
        }
        Ok(key.split('/').fold(self.root.clone(), |p, part| p.join(part)))
    }
}

#[async_trait]
impl Storage for FsStorage {
    async fn put(&self, key: &str, bytes: Vec<u8>, _content_type: &str) -> Result<(), String> {
        let path = self.path(key)?;
        let dir = path.parent().ok_or("no parent directory")?.to_path_buf();
        tokio::fs::create_dir_all(&dir).await.map_err(|e| format!("{}: {e}", dir.display()))?;
        // Write beside the target and rename, so a reader never sees half a file.
        let tmp = dir.join(format!(".tmp-{}", backend_common::token::random_hex(8)));
        tokio::fs::write(&tmp, &bytes).await.map_err(|e| format!("{}: {e}", tmp.display()))?;
        tokio::fs::rename(&tmp, &path).await.map_err(|e| format!("{}: {e}", path.display()))
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let path = self.path(key)?;
        match tokio::fs::read(&path).await {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    async fn delete(&self, key: &str) -> Result<(), String> {
        let path = self.path(key)?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    async fn check(&self) -> Result<(), String> {
        let probe = format!(".probe-{}", backend_common::token::random_hex(6));
        self.put(&probe, b"ok".to_vec(), "text/plain").await?;
        self.delete(&probe).await
    }
}

// ------------------------------------------------------------------ s3

#[derive(Debug, Clone)]
pub struct S3Config {
    /// `https://nyc3.digitaloceanspaces.com`, `http://minio:9000`.
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    /// `Some(true)`: `endpoint/bucket/key`. `Some(false)`: `bucket.endpoint/key`. `None`: decide from the endpoint.
    pub path_style: Option<bool>,
}

pub struct S3Storage {
    cfg: S3Config,
    scheme: String,
    /// Host (with a non-default port) of the endpoint.
    authority: String,
    path_style: bool,
    http: reqwest::Client,
}

/// Virtual-hosted addressing needs a DNS name per bucket; that exists at AWS and
/// DigitalOcean. Anything else (MinIO, an address, localhost) gets path-style.
fn auto_path_style(host: &str) -> bool {
    !(host.ends_with(".amazonaws.com") || host.ends_with(".digitaloceanspaces.com"))
}

/// RFC 3986 encoding of one path segment, as SigV4 wants it.
fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("hmac key");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

fn hex(b: &[u8]) -> String {
    backend_common::hex(b)
}

/// The `Authorization` value for a request. `headers` are the signed headers:
/// lower-case names, sorted, with trimmed values. `amz_date` is `YYYYMMDDTHHMMSSZ`.
pub fn sigv4(
    access_key: &str,
    secret_key: &str,
    region: &str,
    method: &str,
    canonical_uri: &str,
    headers: &[(&str, &str)],
    payload_sha256: &str,
    amz_date: &str,
) -> String {
    let date = &amz_date[..8];
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed: Vec<&str> = headers.iter().map(|(k, _)| *k).collect();
    let signed = signed.join(";");
    let canonical = format!("{method}\n{canonical_uri}\n\n{canonical_headers}\n{signed}\n{payload_sha256}");
    let scope = format!("{date}/{region}/s3/aws4_request");
    let to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", hex(&Sha256::digest(canonical.as_bytes())));
    let mut key = hmac(format!("AWS4{secret_key}").as_bytes(), date.as_bytes());
    for part in [region, "s3", "aws4_request"] {
        key = hmac(&key, part.as_bytes());
    }
    let signature = hex(&hmac(&key, to_sign.as_bytes()));
    format!("AWS4-HMAC-SHA256 Credential={access_key}/{scope},SignedHeaders={signed},Signature={signature}")
}

fn amz_date_now() -> String {
    let t = time::OffsetDateTime::now_utc();
    format!("{:04}{:02}{:02}T{:02}{:02}{:02}Z", t.year(), t.month() as u8, t.day(), t.hour(), t.minute(), t.second())
}

impl S3Storage {
    pub fn new(cfg: S3Config) -> Result<S3Storage, String> {
        let (scheme, rest) = cfg.endpoint.split_once("://").ok_or("S3_ENDPOINT must start with http:// or https://")?;
        if scheme != "http" && scheme != "https" {
            return Err("S3_ENDPOINT must start with http:// or https://".into());
        }
        let authority = rest.trim_end_matches('/').to_string();
        if authority.is_empty() || authority.contains('/') {
            return Err("S3_ENDPOINT must be a host without a path (the bucket goes in S3_BUCKET)".into());
        }
        let host = authority.split(':').next().unwrap_or("").to_string();
        let path_style = cfg.path_style.unwrap_or_else(|| auto_path_style(&host));
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        Ok(S3Storage { scheme: scheme.to_string(), authority, path_style, http, cfg })
    }

    /// (Host header, path) of an object; an empty key addresses the bucket.
    fn address(&self, key: &str) -> (String, String) {
        let object: String = key.split('/').filter(|p| !p.is_empty()).map(|p| format!("/{}", encode_segment(p))).collect();
        if self.path_style {
            (self.authority.clone(), format!("/{}{}", encode_segment(&self.cfg.bucket), object))
        } else {
            (format!("{}.{}", self.cfg.bucket, self.authority), if object.is_empty() { "/".into() } else { object })
        }
    }

    async fn call(&self, method: reqwest::Method, key: &str, body: Option<(Vec<u8>, &str)>) -> Result<reqwest::Response, String> {
        let (host, path) = self.address(key);
        let payload_hash = hex(&Sha256::digest(body.as_ref().map(|b| b.0.as_slice()).unwrap_or(b"")));
        let date = amz_date_now();
        let headers = [("host", host.as_str()), ("x-amz-content-sha256", payload_hash.as_str()), ("x-amz-date", date.as_str())];
        let auth =
            sigv4(&self.cfg.access_key, &self.cfg.secret_key, &self.cfg.region, method.as_str(), &path, &headers, &payload_hash, &date);
        let mut req = self
            .http
            .request(method, format!("{}://{}{}", self.scheme, host, path))
            .header("x-amz-content-sha256", &payload_hash)
            .header("x-amz-date", &date)
            .header("authorization", auth);
        if let Some((bytes, content_type)) = body {
            req = req.header("content-type", content_type).body(bytes);
        }
        req.send().await.map_err(|e| format!("s3: {}", e.without_url()))
    }
}

async fn refused(what: &str, resp: reqwest::Response) -> String {
    let status = resp.status();
    let body: String = resp.text().await.unwrap_or_default().chars().take(300).collect();
    format!("s3 {what}: {status} {body}")
}

#[async_trait]
impl Storage for S3Storage {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> Result<(), String> {
        if !valid_key(key) {
            return Err(format!("bad storage key '{key}'"));
        }
        let resp = self.call(reqwest::Method::PUT, key, Some((bytes, content_type))).await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(refused("put", resp).await)
        }
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        if !valid_key(key) {
            return Err(format!("bad storage key '{key}'"));
        }
        let resp = self.call(reqwest::Method::GET, key, None).await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(refused("get", resp).await);
        }
        resp.bytes().await.map(|b| Some(b.to_vec())).map_err(|e| format!("s3 get: {}", e.without_url()))
    }

    async fn delete(&self, key: &str) -> Result<(), String> {
        if !valid_key(key) {
            return Err(format!("bad storage key '{key}'"));
        }
        let resp = self.call(reqwest::Method::DELETE, key, None).await?;
        if resp.status().is_success() || resp.status() == reqwest::StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(refused("delete", resp).await)
        }
    }

    async fn check(&self) -> Result<(), String> {
        let resp = self.call(reqwest::Method::HEAD, "", None).await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("s3 bucket '{}': {}", self.cfg.bucket, resp.status()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The "GET Object" example of the Amazon S3 API reference (Signature Version 4,
    /// "Authenticating Requests: Using the Authorization Header").
    #[test]
    fn sigv4_matches_the_aws_example() {
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let auth = sigv4(
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            "GET",
            "/test.txt",
            &[
                ("host", "examplebucket.s3.amazonaws.com"),
                ("range", "bytes=0-9"),
                ("x-amz-content-sha256", empty),
                ("x-amz-date", "20130524T000000Z"),
            ],
            empty,
            "20130524T000000Z",
        );
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,\
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date,\
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    fn cfg(endpoint: &str, path_style: Option<bool>) -> S3Config {
        S3Config {
            endpoint: endpoint.into(),
            region: "us-east-1".into(),
            bucket: "ecm".into(),
            access_key: "a".into(),
            secret_key: "s".into(),
            path_style,
        }
    }

    #[test]
    fn addressing() {
        let minio = S3Storage::new(cfg("http://minio:9000", None)).unwrap();
        assert_eq!(minio.address("packages/a.b/1.0.0+x/f g.ecmpkg"), ("minio:9000".into(), "/ecm/packages/a.b/1.0.0%2Bx/f%20g.ecmpkg".into()));
        assert_eq!(minio.address(""), ("minio:9000".into(), "/ecm".into()));
        let spaces = S3Storage::new(cfg("https://nyc3.digitaloceanspaces.com/", None)).unwrap();
        assert_eq!(spaces.address("k/v"), ("ecm.nyc3.digitaloceanspaces.com".into(), "/k/v".into()));
        assert_eq!(spaces.address(""), ("ecm.nyc3.digitaloceanspaces.com".into(), "/".into()));
        let forced = S3Storage::new(cfg("https://nyc3.digitaloceanspaces.com", Some(true))).unwrap();
        assert_eq!(forced.address("k").1, "/ecm/k");
        assert!(S3Storage::new(cfg("nyc3.digitaloceanspaces.com", None)).is_err());
        assert!(S3Storage::new(cfg("https://host/bucket", None)).is_err());
    }

    #[tokio::test]
    async fn fs_round_trip_and_key_checks() {
        let dir = tempfile::tempdir().unwrap();
        let s = FsStorage::new(dir.path().join("blobs").to_str().unwrap()).unwrap();
        s.check().await.unwrap();
        assert_eq!(s.get("packages/a.b/1.0.0/a.b-1.0.0.ecmpkg").await.unwrap(), None);
        s.put("packages/a.b/1.0.0/a.b-1.0.0.ecmpkg", vec![1, 2, 3], "application/octet-stream").await.unwrap();
        s.put("packages/a.b/1.0.0/a.b-1.0.0.ecmpkg", vec![4, 5], "application/octet-stream").await.unwrap();
        assert_eq!(s.get("packages/a.b/1.0.0/a.b-1.0.0.ecmpkg").await.unwrap(), Some(vec![4, 5]));
        assert!(dir.path().join("blobs/packages/a.b/1.0.0/a.b-1.0.0.ecmpkg").is_file());
        s.delete("packages/a.b/1.0.0/a.b-1.0.0.ecmpkg").await.unwrap();
        s.delete("packages/a.b/1.0.0/a.b-1.0.0.ecmpkg").await.unwrap();
        assert_eq!(s.get("packages/a.b/1.0.0/a.b-1.0.0.ecmpkg").await.unwrap(), None);
        for bad in ["../x", "a/../b", "/abs", "a//b", "", "a\\b"] {
            assert!(s.put(bad, vec![], "x").await.is_err(), "{bad}");
            assert!(s.get(bad).await.is_err(), "{bad}");
        }
    }
}
