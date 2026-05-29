use reqwest::StatusCode;

#[derive(Debug, thiserror::Error)]
pub enum TusError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid url: {0}")]
    InvalidUrl(#[from] url::ParseError),

    #[error("unexpected status: {0}")]
    UnexpectedStatus(StatusCode),

    #[error("missing or invalid header: {0}")]
    BadHeader(&'static str),
}
