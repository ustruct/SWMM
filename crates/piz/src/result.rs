use camino::Utf8PathBuf;
use thiserror::Error;

pub type ZipResult<T> = Result<T, ZipError>;

#[derive(Debug, Error)]
pub enum ZipError {

    #[error("I/O Error")]
    Io(#[from] std::io::Error),

    #[error("Invalid Zip archive: {0}")]
    InvalidArchive(&'static str),

    #[error("Invalid UTF-8")]
    Encoding(#[from] std::str::Utf8Error),

    #[error("Unsupported Zip archive: {0}")]
    UnsupportedArchive(String),

    #[error("Archive prepended with {0} unknown bytes")]
    PrependedWithUnknownBytes(usize),

    #[error("Archive contained strange a strange file hierarchy: {0}")]
    Hierarchy(String),

    #[error("No file in the archive with the path {0}")]
    NoSuchFile(Utf8PathBuf),

    #[error("Invalid path")]
    InvalidPath(String),

    #[error("Zip archive too large for address space")]
    InsufficientAddressSpace,
}
