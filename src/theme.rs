//! Terminal color theme for diagnostic output.
//!
//! All color choices live here — change a function to restyle the whole CLI.
//! A future config file can override these by calling `colored::control::set_override`.

use colored::Colorize;

pub fn severity_error(s: &str) -> String {
    s.red().bold().to_string()
}

pub fn severity_warning(s: &str) -> String {
    s.yellow().bold().to_string()
}

pub fn locator(s: &str) -> String {
    s.cyan().dimmed().to_string()
}

pub fn message(s: &str) -> String {
    s.white().bold().to_string()
}
