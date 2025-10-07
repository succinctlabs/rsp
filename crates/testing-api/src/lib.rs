//! RSP Testing API
//!
//! This crate provides a simplified API for working with RSP (Reth in SP1) in testing contexts.
//! It exposes two main functions:
//!
//! 1. `fetch_block_stdin` - Fetches and executes a block, generating stdin data
//! 2. `get_program_elf` - Retrieves the compiled ELF for RSP client programs
//!
//! # Features
//!
//! - `embedded-programs` - Embeds the compiled ELF programs at build time. This enables
//!   the `get_program_elf` function but significantly increases build time.
//!
//! # Example
//!
//! ```no_run
//! use rsp_testing_api::{fetch_block_stdin, get_program_elf, ProgramType};
//! use std::path::Path;
//!
//! # tokio_test::block_on(async {
//! // Fetch stdin for a block
//! let stdin_path = fetch_block_stdin(
//!     21000000,
//!     "https://eth-mainnet.g.alchemy.com/v2/YOUR_API_KEY",
//!     Path::new("./cache")
//! ).await?;
//!
//! println!("Stdin written to: {}", stdin_path.display());
//!
//! // Get the ETH program ELF (requires 'embedded-programs' feature)
//! # #[cfg(feature = "embedded-programs")]
//! let eth_elf = get_program_elf(ProgramType::Eth)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! # });
//! ```

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod error;
mod program;
mod stdin;

pub use error::TestingApiError;
pub use program::{get_program_elf, ProgramType};
pub use stdin::fetch_block_stdin;
