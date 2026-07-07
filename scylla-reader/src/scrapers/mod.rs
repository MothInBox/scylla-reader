//! Scraper registry — plugin-based and built-in scrapers.

pub mod plugin_install;
pub mod services;
pub use services::ScraperRegistry;
