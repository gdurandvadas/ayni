use std::env;
use std::io::{self, IsTerminal};

pub mod fallback;
pub mod layout;
pub mod md_report;
pub mod report;
pub mod runner;

pub const PASS_RGB: (u8, u8, u8) = (0x06, 0xd6, 0xa0);
pub const WARN_RGB: (u8, u8, u8) = (0xff, 0xd1, 0x66);
pub const FAIL_RGB: (u8, u8, u8) = (0xef, 0x47, 0x6f);

#[must_use]
pub fn is_interactive_stdout() -> bool {
    io::stdout().is_terminal()
}

#[must_use]
pub fn color_enabled() -> bool {
    env::var_os("NO_COLOR").is_none()
}
