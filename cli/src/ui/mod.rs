use std::env;
use std::io::{self, IsTerminal};

pub mod fallback;
pub mod layout;
pub mod md_report;
pub mod report;
pub mod runner;

#[must_use]
pub fn is_interactive_stdout() -> bool {
    io::stdout().is_terminal()
}

#[must_use]
pub fn color_enabled() -> bool {
    env::var_os("NO_COLOR").is_none()
}
