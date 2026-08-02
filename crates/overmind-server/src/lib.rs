pub mod api;
pub mod audit;
pub mod ceo;
pub mod db;
pub mod domain;
pub mod files;
pub mod governance;
pub mod i18n;
pub mod mcp;
pub mod meeting;
pub mod notify;
pub mod org;
pub mod runner;
pub mod scheduler;
pub mod ws;

pub use api::app;
pub use db::{AppState, Config, init, init_with};
