// Copyright (c) 2026-present VexaHub and collaborators
// SPDX-License-Identifier: MIT

use std::path::Path;

use reqwest::header::HeaderMap;
use reqwest::{Client as HttpClient, StatusCode};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

mod error;

pub use error::TusError;
pub use reqwest::Url;

const TUS_RESUMABLE: &str = "1.0.0";
const CONTENT_TYPE: &str = "application/offset+octet-stream";

/// Server capabilities returned by OPTIONS.
#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub version: Box<[Box<str>]>,
    pub extensions: Box<[Box<str>]>,
    pub max_size: Option<u64>,
}

/// Upload state returned by HEAD.
#[derive(Debug, Clone)]
pub struct UploadInfo {
    pub offset: u64,
    pub length: Option<u64>,
    pub expires: Option<Box<str>>,
}

/// Tus resumable upload client.
#[derive(Debug, Clone)]
pub struct TusClient {
    http: HttpClient,
    endpoint: Url,
    chunk_size: Option<u64>,
    headers: HeaderMap,
}

impl ServerInfo {
    pub fn has_extension(&self, name: &str) -> bool {
        self.extensions.iter().any(|e| e.as_ref() == name)
    }
}

impl TusClient {
    /// Create a new client pointing at the tus endpoint.
    pub fn new(endpoint: &str) -> Result<Self, TusError> {
        let endpoint = Url::parse(endpoint)?;
        let http = HttpClient::new();

        Ok(Self {
            http,
            endpoint,
            chunk_size: None,
            headers: HeaderMap::new(),
        })
    }

    /// Use a custom reqwest client (timeouts, proxy, etc.).
    pub fn with_http_client(mut self, client: HttpClient) -> Self {
        self.http = client;

        self
    }

    /// Set chunk size for splitting PATCH requests.
    /// If unset, the entire remaining file is sent in one PATCH.
    pub fn with_chunk_size(mut self, size: u64) -> Self {
        self.chunk_size = Some(size);

        self
    }

    /// Add custom headers sent with every request (auth, etc.).
    pub fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;

        self
    }

    /// OPTIONS: discover server capabilities.
    pub async fn server_info(&self) -> Result<ServerInfo, TusError> {
        let resp = self
            .http
            .request(reqwest::Method::OPTIONS, self.endpoint.clone())
            .headers(self.headers.clone())
            .send()
            .await?;

        let status = resp.status();

        if status != StatusCode::OK && status != StatusCode::NO_CONTENT {
            return Err(TusError::UnexpectedStatus(status));
        }

        let version = parse_comma_list(resp.headers(), "tus-version");
        let extensions = parse_comma_list(resp.headers(), "tus-extension");
        let max_size = optional_header::<u64>(resp.headers(), "tus-max-size");

        Ok(ServerInfo {
            version,
            extensions,
            max_size,
        })
    }

    /// POST: create a new upload, returns the upload URL.
    pub async fn create(
        &self,
        length: u64,
        metadata: Option<&[(&str, &str)]>,
    ) -> Result<Url, TusError> {
        let mut req = self
            .http
            .post(self.endpoint.clone())
            .headers(self.headers.clone())
            .header("tus-resumable", TUS_RESUMABLE)
            .header("upload-length", length.to_string())
            .header("content-length", "0");

        if let Some(meta) = metadata {
            req = req.header("upload-metadata", encode_metadata(meta));
        }

        let resp = req.send().await?;

        if resp.status() != StatusCode::CREATED {
            return Err(TusError::UnexpectedStatus(resp.status()));
        }

        let location = required_header_str(resp.headers(), "location")?;

        resolve_url(&self.endpoint, location)
    }

    /// HEAD: get current upload offset and length.
    pub async fn get_offset(&self, upload_url: &Url) -> Result<UploadInfo, TusError> {
        let resp = self
            .http
            .head(upload_url.clone())
            .headers(self.headers.clone())
            .header("tus-resumable", TUS_RESUMABLE)
            .send()
            .await?;

        let status = resp.status();

        if status != StatusCode::OK && status != StatusCode::NO_CONTENT {
            return Err(TusError::UnexpectedStatus(status));
        }

        let offset = required_header::<u64>(resp.headers(), "upload-offset")?;
        let length = optional_header::<u64>(resp.headers(), "upload-length");
        let expires = optional_header_boxed(resp.headers(), "upload-expires");

        Ok(UploadInfo {
            offset,
            length,
            expires,
        })
    }

    /// PATCH: upload file bytes, resuming from the current offset.
    pub async fn upload(&self, upload_url: &Url, path: &Path) -> Result<(), TusError> {
        let mut file = File::open(path).await?;
        let file_len = file.metadata().await?.len();
        let info = self.get_offset(upload_url).await?;

        if info.offset >= file_len {
            tracing::debug!(offset = info.offset, len = file_len, "already complete");
            return Ok(());
        }

        let mut offset = info.offset;

        if offset > 0 {
            file.seek(std::io::SeekFrom::Start(offset)).await?;
        }

        while offset < file_len {
            let remaining = file_len - offset;

            let chunk_len = match self.chunk_size {
                Some(cs) => remaining.min(cs),
                None => remaining,
            };

            let chunk = read_chunk(&mut file, chunk_len).await?;

            let resp = self
                .http
                .patch(upload_url.clone())
                .headers(self.headers.clone())
                .header("tus-resumable", TUS_RESUMABLE)
                .header("upload-offset", offset.to_string())
                .header("content-length", chunk_len.to_string())
                .header("content-type", CONTENT_TYPE)
                .body(chunk)
                .send()
                .await?;

            if resp.status() != StatusCode::NO_CONTENT {
                return Err(TusError::UnexpectedStatus(resp.status()));
            }

            offset = required_header::<u64>(resp.headers(), "upload-offset")?;

            tracing::debug!(offset, total = file_len, "chunk uploaded");
        }

        Ok(())
    }

    /// DELETE: terminate an upload and free server resources.
    pub async fn delete(&self, upload_url: &Url) -> Result<(), TusError> {
        let resp = self
            .http
            .delete(upload_url.clone())
            .headers(self.headers.clone())
            .header("tus-resumable", TUS_RESUMABLE)
            .header("content-length", "0")
            .send()
            .await?;

        if resp.status() != StatusCode::NO_CONTENT {
            return Err(TusError::UnexpectedStatus(resp.status()));
        }

        Ok(())
    }
}

/// Parse a required header into a typed value.
fn required_header<T: std::str::FromStr>(
    headers: &HeaderMap,
    name: &'static str,
) -> Result<T, TusError> {
    headers
        .get(name)
        .ok_or(TusError::BadHeader(name))?
        .to_str()
        .map_err(|_| TusError::BadHeader(name))?
        .parse::<T>()
        .map_err(|_| TusError::BadHeader(name))
}

/// Parse a required header as a string slice.
fn required_header_str<'a>(
    headers: &'a HeaderMap,
    name: &'static str,
) -> Result<&'a str, TusError> {
    headers
        .get(name)
        .ok_or(TusError::BadHeader(name))?
        .to_str()
        .map_err(|_| TusError::BadHeader(name))
}

/// Try to parse an optional header into a typed value.
fn optional_header<T: std::str::FromStr>(headers: &HeaderMap, name: &str) -> Option<T> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
}

/// Try to read an optional header as a Box<str>.
fn optional_header_boxed(headers: &HeaderMap, name: &str) -> Option<Box<str>> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.into())
}

/// Parse a comma separated header into boxed strings.
fn parse_comma_list(headers: &HeaderMap, name: &str) -> Box<[Box<str>]> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().into())
                .collect::<Vec<Box<str>>>()
                .into_boxed_slice()
        })
        .unwrap_or_default()
}

/// Encode metadata as per tus spec: `key base64val,key base64val`.
fn encode_metadata(metadata: &[(&str, &str)]) -> String {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    metadata
        .iter()
        .map(|(k, v)| {
            if v.is_empty() {
                (*k).to_owned()
            } else {
                format!("{} {}", k, STANDARD.encode(v))
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Resolve a possibly relative Location header against the endpoint.
fn resolve_url(base: &Url, location: &str) -> Result<Url, TusError> {
    Ok(base.join(location)?)
}

/// Read the next chunk from an already opened and seeked file handle.
async fn read_chunk(file: &mut File, len: u64) -> Result<Vec<u8>, TusError> {
    let len = usize::try_from(len).map_err(|_| {
        TusError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "chunk size exceeds platform pointer width",
        ))
    })?;

    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf).await?;

    Ok(buf)
}
