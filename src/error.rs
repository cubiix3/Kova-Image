use std::{fmt, io};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    NotFound,
    AccessDenied,
    Unsupported,
    Corrupted(String),
    TooLarge,
    Dimensions,
    MemoryBudget,
    Changed,
    Cancelled,
    Io(String),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "File not found"),
            Self::AccessDenied => write!(f, "Access denied"),
            Self::Unsupported => write!(f, "Unsupported image format"),
            Self::Corrupted(s) => write!(f, "Could not decode image: {s}"),
            Self::TooLarge => write!(f, "File exceeds the 128 MiB input limit"),
            Self::Dimensions => write!(f, "Image dimensions exceed the safety limit"),
            Self::MemoryBudget => write!(f, "Image exceeds the decode memory budget"),
            Self::Changed => write!(f, "File changed during load; open it again"),
            Self::Cancelled => write!(f, "Load cancelled"),
            Self::Io(s) => write!(f, "{s}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        match value.kind() {
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::PermissionDenied => Self::AccessDenied,
            _ => Self::Io(value.to_string()),
        }
    }
}
impl From<image::ImageError> for Error {
    fn from(value: image::ImageError) -> Self {
        match value {
            image::ImageError::Limits(_) => Self::MemoryBudget,
            image::ImageError::Unsupported(_) => Self::Unsupported,
            _ => Self::Corrupted(value.to_string()),
        }
    }
}
