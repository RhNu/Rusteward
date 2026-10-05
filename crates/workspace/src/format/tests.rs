use std::path::Path;

use rusteward_core::Edition;

use super::{Options, command, compare, diff, first_difference};
use crate::{config::FormatSettings, discovery::Source};

#[test]
fn final_difference_retains_original_coordinates_and_texts() {
    let source = Source {
        path: Path::new("project/src/lib.rs").into(),
        edition: Edition::Edition2024,
    };
    let result = compare(
        &source,
        Path::new("project"),
        "fn café() {}\nfn b() {}\n".into(),
        "fn café() {}\n\nfn b() {}\n".into(),
        false,
        Options {
            check: true,
            diff: true,
        },
    );
    let diagnostic = result.diagnostic.unwrap();
    assert_eq!(diagnostic.path.as_deref(), Some(Path::new("src/lib.rs")));
    assert_eq!((diagnostic.line, diagnostic.column), (Some(2), Some(1)));
    assert_eq!(diagnostic.snippet.as_deref(), Some("fn b() {}"));
    assert!(result.diff.unwrap().contains("--- a/src/lib.rs"));
    let change = result.change.unwrap();
    assert_eq!(change.path, source.path);
    assert_eq!(change.original, "fn café() {}\nfn b() {}\n");
    assert_eq!(change.formatted, "fn café() {}\n\nfn b() {}\n");
}

#[test]
fn unchanged_skipped_spacing_produces_no_pending_change() {
    let source = Source {
        path: "src/lib.rs".into(),
        edition: Edition::Edition2024,
    };
    let result = compare(
        &source,
        Path::new("."),
        "fn a() {}\n".into(),
        "fn a() {}\n".into(),
        true,
        Options {
            check: true,
            diff: true,
        },
    );
    assert!(result.skipped);
    assert!(result.change.is_none());
    assert!(result.diagnostic.is_none());
    assert!(result.diff.is_none());
}

#[test]
fn apply_mode_retains_changes_even_when_spacing_was_skipped() {
    let source = Source {
        path: "external/lib.rs".into(),
        edition: Edition::Edition2024,
    };
    let result = compare(
        &source,
        Path::new("project"),
        "fn a(){}\n".into(),
        "fn a() {}\n".into(),
        true,
        Options::default(),
    );
    assert!(result.skipped);
    assert!(result.change.is_some());
    assert!(result.diagnostic.is_none());
    assert!(result.diff.is_none());
}

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
