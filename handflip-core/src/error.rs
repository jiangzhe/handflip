use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("IO error {0}")]
    IO(#[from] std::io::Error),
    #[error("HTTP error {0}")]
    Http(String),
    #[error("Parse error {0}")]
    Parse(String),
    #[error("Bad request {0}")]
    BadRequest(String),
    #[error("Server error {0}")]
    Server(String),
}

impl From<http_types::Error> for Error {
    fn from(src: http_types::Error) -> Error {
        Error::Http(src.to_string())
    }
}

impl From<std::num::ParseIntError> for Error {
    fn from(src: std::num::ParseIntError) -> Error {
        Error::Parse(src.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
