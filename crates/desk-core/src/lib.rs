//! Desk backend core. M1 speaks ACP only; print/headless `-p` is out of scope.

pub mod catalog;
pub mod client;
pub mod codec;
pub mod prompt;
pub mod scheduler;
pub mod service;
pub mod store;
pub mod types;

pub use client::{
    AcpClient, ApprovalDecision, ApprovalHandle, CancelHandle, ClientEvent, EventAction,
    PromptOutcome,
};
pub use codec::{CodecError, DecodeItem, NdjsonCodec};
pub use prompt::encode_prompt;
pub use scheduler::{Job, Scheduler};
pub use service::{
    clock_in_with_ctx, probe_discovered, read_image_data_url, App, Ctx, ModelOption, Provider,
    ProviderStatus, SessionView, Snapshot, SubmitResult,
};
pub use store::{Attachment, CrashRecovery, Message, Notification, Run, Session, Team};
pub use types::RpcMessage;
