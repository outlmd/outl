//! # outl-core
//!
//! Tree CRDT, op log, storage trait, and domain models for the outl outliner.
//!
//! See `crates/outl-core/CLAUDE.md` and `docs/crdt.md` for the algorithm
//! specification and the invariants this crate must preserve.
//!
//! The four functions whose correctness underpins the entire project are
//! [`tree::Tree::do_op`], [`tree::Tree::undo_op`], [`tree::Tree::apply_op`], and
//! [`tree::Tree::creates_cycle`]. Each has a coverage requirement of 100%.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod block;
mod content;
pub mod device;
pub mod fractional;
pub mod hlc;
pub mod id;
pub mod journal;
pub mod lock;
pub mod log;
pub mod op;
pub mod page;
pub mod property;
pub mod snapshot;
pub mod storage;
pub mod tag;
pub mod tree;
pub mod workspace;
pub mod workspace_id;

pub use block::Block;
pub use device::{
    device_dir, ActorBinding, BindingVerdict, DeviceError, DeviceStore, MachineId,
    STALE_BINDING_TTL, STALE_SCRATCH_TTL,
};
pub use fractional::Fractional;
pub use hlc::Hlc;
pub use id::{ActorId, NodeId};
pub use journal::Journal;
pub use lock::{resolve_write_actor, ActorWriteLock, LockError, WorkspaceLock};
pub use log::OpLog;
pub use op::{LogOp, Op};
pub use page::Page;
pub use property::{PropValue, Property};
pub use snapshot::{SnapshotBody, SnapshotError, SCHEMA_VERSION};
pub use storage::{Storage, StorageError};
pub use tag::Tag;
pub use tree::Tree;
pub use workspace::{Workspace, WorkspaceBatch};
pub use workspace_id::{WorkspaceId, WorkspaceIdError};
