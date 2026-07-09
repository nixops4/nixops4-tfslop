//! Client for the tfplugin gRPC protocol.
//!
//! Speaks the go-plugin handshake to a child provider process and connects
//! to it over gRPC. Currently v6-only; the v5 case will be added when a
//! real target requires it.
//!
//! At this stage the module provides launch + disconnect only. Actual
//! provider RPCs (schema, configure, apply, ...) are added incrementally
//! in later commits as their fixture counterparts and Nix translations
//! land.

mod client;
mod handshake;

pub use client::ProviderClient;
