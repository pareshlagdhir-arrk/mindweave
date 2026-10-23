//! Errors of the command layer, worded so an agent can act on them.

use thiserror::Error;
use uuid::Uuid;

use crate::format::FormatError;

/// Why a command was refused or failed. Messages say what to do instead.
#[derive(Debug, Error)]
pub enum OpError {
    /// No object with this id on any page.
    #[error("No object with id {0} in the document. Call list_objects to see the ids.")]
    NotFound(Uuid),
    /// The object exists but is the wrong kind for the command.
    #[error("Object {id} is a {kind}; {expected}.")]
    WrongKind {
        id: Uuid,
        kind: &'static str,
        expected: String,
    },
    /// A bad argument; the message names it and the accepted values.
    #[error("{0}")]
    Invalid(String),
    /// The file could not be read or written.
    #[error(transparent)]
    Format(#[from] FormatError),
}

impl OpError {
    /// An [`OpError::Invalid`] with `message`.
    pub fn invalid(message: impl Into<String>) -> OpError {
        OpError::Invalid(message.into())
    }
}

/// Result of a command.
pub type OpResult<T> = Result<T, OpError>;
