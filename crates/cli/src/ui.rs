use console::style;
use dialoguer::theme::ColorfulTheme;

/// The theme applied to every interactive prompt, so questions and their
/// answers render in color (bold prompt, green checkmark) and stand apart
/// from the plain, dimmed outcome lines printed via [`outcome`].
pub fn theme() -> ColorfulTheme {
    ColorfulTheme::default()
}

/// Prints a line reporting what happened, dimmed so it reads as secondary
/// to the colorful prompts above it - answering "what did this do?" at a
/// glance, distinct from "what did it ask?".
pub fn outcome(msg: impl std::fmt::Display) {
    println!("{}", style(msg).dim());
}
