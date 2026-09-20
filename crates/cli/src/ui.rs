use console::style;
use dialoguer::theme::ColorfulTheme;

pub fn theme() -> ColorfulTheme {
    ColorfulTheme::default()
}

pub fn outcome(msg: impl std::fmt::Display) {
    println!("{}", style(msg).dim());
}
