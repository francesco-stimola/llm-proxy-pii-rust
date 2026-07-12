//! `llm-proxy-pii-rust` — a privacy-preserving LLM proxy.
//!
//! The library exposes the building blocks (config, PII detection, pipeline,
//! proxy, server) so both the binary (`main.rs`) and integration tests can use
//! them. See `docs/ARCHITECTURE.md` for the design.

pub mod config;
pub mod pii;
pub mod pipeline;
pub mod proxy;
pub mod server;
pub mod stream;
