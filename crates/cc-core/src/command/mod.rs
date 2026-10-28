//! Command layer: one implementation per user/AI action, owning validation, defaults,
//! locks, consent gates and the canonical result/error envelope. Shells parse arguments
//! and print/serialize; they do not assemble command JSON themselves.

mod paths;
mod pptx;

pub mod deck;
pub mod generate;
pub mod person;
pub mod progress;
pub mod project;
pub mod render;
pub mod script;
pub mod voice;

pub use person::video_set;
pub use progress::{ProgressEvent, ProgressSink};
pub use project::{project_create, project_delete, project_list, project_status};

use crate::error::Error;
use serde_json::{json, Value};

/// Explicit acknowledgement for an irreversible command (artifact cleanup, project
/// deletion). Shells map their `--confirm` / `confirm:true` flag into this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmation {
    Confirmed,
    Unconfirmed,
}

impl From<bool> for Confirmation {
    fn from(confirmed: bool) -> Self {
        if confirmed {
            Confirmation::Confirmed
        } else {
            Confirmation::Unconfirmed
        }
    }
}

/// Structured error payload, identical on every shell: `error.code` is stable and
/// machine-readable, `message` is user-readable.
pub fn error_json(err: &Error) -> Value {
    json!({
        "ok": false,
        "error": { "code": err.code(), "message": err.to_string() }
    })
}
