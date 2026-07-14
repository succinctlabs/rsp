//! Library surface for the `ethproofs` package.
//!
//! This exposes a typed, request/response client for the ethproofs HTTP API, used by the
//! `ethproofs-cli` binary. The long-running proving service (`main.rs`) does not use this: it
//! has its own fire-and-forget submission client ([`crate::ethproofs`] in that binary) whose
//! background, best-effort semantics are deliberately different from what a CLI needs.

pub mod api;
