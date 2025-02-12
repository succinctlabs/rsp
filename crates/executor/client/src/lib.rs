/// Client program input data types.
pub mod io;
#[macro_use]
mod utils;
pub mod custom;
pub mod error;
pub mod executor;

mod into_primitives;
pub use into_primitives::{FromInput, IntoInput, IntoPrimitives};
