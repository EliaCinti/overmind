pub mod api;
pub mod audit;
pub mod ceo;
pub mod claude_auth;
pub mod db;
pub mod domain;
pub mod economy;
pub mod files;
pub mod governance;
pub mod i18n;
/// The cage's second layer, where the kernel has one (ADR-0029). Linux only —
/// on anything else `sandbox` simply never asks for it.
#[cfg(target_os = "linux")]
pub mod landlock;
pub mod mcp;
pub mod mcp_server;
pub mod meeting;
pub mod model;
pub mod notify;
pub mod org;
pub mod runner;
pub mod sandbox;
pub mod scheduler;
pub mod ws;

pub use api::app;
pub use db::{AppState, Config, init, init_with};
