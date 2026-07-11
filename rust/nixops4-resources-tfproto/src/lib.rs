//! Generated gRPC bindings and adapter internals.
//!
//! Currently only the tfplugin protocol v6 bindings are exposed; v5 will be
//! added when a concrete target requires it.

/// Generated protobuf + tonic bindings for tfplugin v6.
pub mod tfplugin6 {
    tonic::include_proto!("tfplugin6");
}

pub mod outputs;
pub mod prior_state;
pub mod provider;
pub mod schema;
pub mod tf_provider_client;
