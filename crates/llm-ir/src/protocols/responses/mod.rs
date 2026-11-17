#![doc = include_str!("README.md")]

pub mod decoder;
pub(crate) mod parse;
pub(crate) mod render;
mod terminal_scan;
mod wire;
pub mod writer;
