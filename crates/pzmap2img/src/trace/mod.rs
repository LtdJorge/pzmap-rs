//! The log level option for the command-line tool.

use clap::ValueEnum;
use tracing::Level;

/// How much the command-line tool logs. Converts to a [`tracing::Level`].
#[derive(Copy, Clone, Debug, Default, ValueEnum)]
pub enum TraceLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl From<TraceLevel> for Level {
    fn from(value: TraceLevel) -> Self {
        match value {
            TraceLevel::Trace => Level::TRACE,
            TraceLevel::Debug => Level::DEBUG,
            TraceLevel::Info => Level::INFO,
            TraceLevel::Warn => Level::WARN,
            TraceLevel::Error => Level::ERROR,
        }
    }
}
