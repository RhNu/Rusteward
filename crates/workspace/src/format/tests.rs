use std::path::Path;

use rusteward_core::Edition;

use super::{command, diff, first_difference};
use crate::config::FormatSettings;

#[test]
fn mismatch_coordinates_handle_unicode_and_newline_changes() {
    assert_eq!(
        first_difference("fn café() {}\nfn b() {}\n", "fn café() {}\n\nfn b() {}\n"),
        (2, 1)
    );
    assert_eq!(first_difference("汉字 a", "汉字 b"), (1, 4));
    assert_eq!(first_difference("a\r\n", "a\n"), (1, 2));
}

#[test]
fn unified_diff_shows_final_text_change() {
    let output = diff(
        Path::new("src/lib.rs"),
        "fn a() {}\nfn b() {}\n",
        "fn a() {}\n\nfn b() {}\n",
    );
    assert!(output.contains("--- a/src/lib.rs"));
    assert!(output.contains("+++ b/src/lib.rs"));
    assert!(output.contains("\n+\n"));
}

#[test]
fn rustfmt_options_are_independent_arguments() {
    let mut settings = FormatSettings::default();
    settings
        .rustfmt
        .insert("max_width".into(), toml::Value::Integer(90));
    let command = command(
        Path::new("project"),
        Path::new("empty.toml"),
        Edition::Edition2024,
        &settings,
    )
    .unwrap();
    let args: Vec<_> = command
        .get_args()
        .map(|arg| arg.to_str().unwrap())
        .collect();
    assert!(args.windows(2).any(|pair| pair == ["--edition", "2024"]));
    assert!(
        args.windows(2)
            .any(|pair| pair == ["--config", "max_width=90"])
    );
    assert!(
        args.windows(2)
            .any(|pair| pair == ["--config", "skip_children=true"])
    );
}
