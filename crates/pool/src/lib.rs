//! Utilities for working with hyper.
//!
//! This crate is less-stable than [`hyper`](https://docs.rs/hyper). However,
//! does respect Rust's semantic version regarding breaking changes.

mod common;
pub mod connect;
pub mod pool;
pub mod rt;

#[cfg(test)]
mod blackbox_tests;
mod client;
pub mod service;
pub use client::{Builder, Client, Error, ResponseFuture};
