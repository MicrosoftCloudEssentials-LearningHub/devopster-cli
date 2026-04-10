use anyhow::{Context, Result};
use console::{style, Style};
use dialoguer::{theme::ColorfulTheme, Input, MultiSelect, Select};

fn theme() -> ColorfulTheme {
    ColorfulTheme {
        prompt_style: Style::new().cyan().bold(),
        values_style: Style::new().green(),
        active_item_style: Style::new().yellow().bold(),
        active_item_prefix: style("> ".to_string()).yellow().bold(),
        checked_item_prefix: style("[x] ".to_string()).yellow().bold(),
        unchecked_item_prefix: style("[ ] ".to_string()).dim(),
        picked_item_prefix: style("* ".to_string()).green().bold(),
        unpicked_item_prefix: style("- ".to_string()).dim(),
        success_prefix: style("ok: ".to_string()).green().bold(),
        error_prefix: style("error: ".to_string()).red().bold(),
        prompt_prefix: style("> ".to_string()).cyan().bold(),
        ..ColorfulTheme::default()
    }
}

pub fn header(title: &str) {
    let line = "=".repeat(title.len().max(24));
    println!("{}", style(line).cyan().bold());
    println!("{}", style(title).cyan().bold());
    println!("{}", style("=".repeat(title.len().max(24))).cyan().bold());
}

pub fn hero(title: &str, subtitle: &str) {
    header(title);
    println!("{}", style(subtitle).white().bold());
}

pub fn section(title: &str) {
    println!();
    println!("{} {}", style("--").yellow().bold(), style(title).bold());
}

pub fn info(message: &str) {
    println!("{} {}", style("info").cyan().bold(), message);
}

pub fn success(message: &str) {
    println!("{} {}", style("done").green().bold(), message);
}

pub fn warn(message: &str) {
    println!("{} {}", style("warn").yellow().bold(), message);
}

pub fn note(message: &str) {
    println!("{} {}", style("note").blue().bold(), style(message).dim());
}

pub fn item(message: &str) {
    println!("  {} {}", style("-").yellow().bold(), message);
}

pub fn key_value(label: &str, value: impl std::fmt::Display) {
    println!("  {} {}", style(format!("{label}:")).bold(), value);
}

pub fn error(message: &str) {
    eprintln!("{} {}", style("error").red().bold(), message);
}

pub fn prompt_input(prompt: &str) -> Result<String> {
    let theme = theme();
    Input::<String>::with_theme(&theme)
        .with_prompt(prompt)
        .allow_empty(true)
        .interact_text()
        .with_context(|| format!("failed to read input for '{prompt}'"))
}

pub fn prompt_confirm(prompt: &str, default_yes: bool) -> Result<bool> {
    let suffix = if default_yes { "[Y/n]" } else { "[y/N]" };

    loop {
        let answer = prompt_input(&format!("{prompt} {suffix}"))?;
        let trimmed = answer.trim();

        if trimmed.is_empty() {
            return Ok(default_yes);
        }

        if matches!(trimmed.to_ascii_lowercase().as_str(), "y" | "yes") {
            return Ok(true);
        }

        if matches!(trimmed.to_ascii_lowercase().as_str(), "n" | "no") {
            return Ok(false);
        }

        warn("Enter Y or N, or press Enter to accept the default.");
    }
}

pub fn select(prompt: &str, items: &[String], default: usize) -> Result<usize> {
    let theme = theme();
    Select::with_theme(&theme)
        .with_prompt(prompt)
        .default(default)
        .items(items)
        .interact()
        .with_context(|| format!("failed to read selection for '{prompt}'"))
}

pub fn multi_select(
    prompt: &str,
    items: &[String],
    defaults: Option<&[bool]>,
) -> Result<Vec<usize>> {
    let theme = theme();
    let mut multi = MultiSelect::with_theme(&theme);
    multi = multi.with_prompt(prompt).items(items);
    if let Some(defaults) = defaults {
        multi = multi.defaults(defaults);
    }

    multi
        .interact()
        .with_context(|| format!("failed to read selection for '{prompt}'"))
}
