//! Core business logic and application state.
//!
//! This crate glues together protocol, network, storage, and UI layers.

pub mod app_state;
pub mod handlers;

pub use app_state::AppState;
