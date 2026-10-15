//! Strictly opt-in, anonymous telemetry for the compatibility database.
//!
//! - Off by default: nothing is recorded until the user runs `cross telemetry enable`.
//! - Local only: events go to a queue file in the data directory. There is no
//!   endpoint and no network code here; nothing is ever sent (see `docs/telemetry.md`).
//! - Anonymous: macOS `major.minor`, chip family, built-in engine id, graphics
//!   backend and launch outcome. No ids, paths, names or timestamps ([`event`]).
//! - `DO_NOT_TRACK=1` suppresses recording even when enabled; disabling deletes the queue.

mod error;
pub mod event;
pub mod store;

pub use error::{Error, Result};
pub use event::{Event, Outcome};
pub use store::{record_launch, Settings, Telemetry};
