use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SmoothError {
    #[error("not yet implemented")]
    NotImplemented,
}
