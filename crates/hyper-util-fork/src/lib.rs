//! Utilities for working with hyper.
//!
//! This crate is less-stable than [`hyper`](https://docs.rs/hyper). However,
//! does respect Rust's semantic version regarding breaking changes.

pub mod client;
mod common;
pub mod rt;

pub mod service;
