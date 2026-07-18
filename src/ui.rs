//! Terminal output styling. All human-facing rendering lives here so the
//! command logic stays output-format agnostic.

use console::style;

pub fn header(text: &str) {
    println!("\n{}", style(text).bold().cyan());
}

pub fn kv(key: &str, value: &str) {
    println!("  {:<24} {value}", style(key).dim());
}

pub fn ok(msg: &str) {
    println!("{} {msg}", style("✓").green().bold());
}

pub fn warn(msg: &str) {
    println!("{} {msg}", style("!").yellow().bold());
}

pub fn error(msg: &str) {
    eprintln!("{} {msg}", style("✗").red().bold());
}
