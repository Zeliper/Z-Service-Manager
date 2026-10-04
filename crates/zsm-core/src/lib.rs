pub mod buffer;
pub mod config;
mod launch;
pub mod logfile;
pub mod manager;
mod metrics;
pub mod output;
pub mod paths;
pub mod supervisor;
pub mod win;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
