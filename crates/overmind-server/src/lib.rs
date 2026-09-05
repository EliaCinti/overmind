pub mod api;
pub mod audit;
pub mod auth;
pub mod backup;
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
pub mod provider;
pub mod runner;
/// Test-only re-exports: integration tests exercise the narration parsers
/// without making them public API.
pub mod runner_test_hooks {
    pub use crate::runner::draft_reply;

    /// Through the trait rather than at the function behind it: since
    /// ADR-0048 the path a run actually takes goes via `Provider`, and a test
    /// that skipped it would stop proving the thing it is named for.
    pub fn text_delta_in(line: &str) -> Option<String> {
        crate::provider::current().text_delta(line)
    }
}
pub mod sandbox;
pub mod scheduler;
pub mod ws;

pub use api::app;
pub use db::{AppState, Config, init, init_with};
