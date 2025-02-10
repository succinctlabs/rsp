#![warn(unused_crate_dependencies)]

/// Client program input data types.
pub mod io;
#[macro_use]
mod utils;
pub mod custom;
pub mod error;
pub mod executor;

mod from_any;
pub use from_any::FromAny;

/// Chain ID for Ethereum Mainnet.
pub const CHAIN_ID_ETH_MAINNET: u64 = 0x1;

/// Chain ID for OP Mainnet.
pub const CHAIN_ID_OP_MAINNET: u64 = 0xa;

/// Chain ID for Linea Mainnet.
pub const CHAIN_ID_LINEA_MAINNET: u64 = 0xe708;

/// Chain ID for Sepolia.
pub const CHAIN_ID_SEPOLIA: u64 = 0xaa36a7;
