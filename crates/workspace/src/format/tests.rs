use std::path::Path;

use rusteward_core::Edition;

use super::{Options, command, compare, diff, difference_message, first_difference};
use crate::{config::FormatSettings, discovery::Source, report::DiagnosticLevel};

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

#[test]
fn newline_only_mismatches_remain_format_errors_with_original_text_and_diff() {
    let source = Source {
        path: "src/lib.rs".into(),
        edition: Edition::Edition2024,
    };
    let result = compare(
        &source,
        Path::new("."),
        "fn café() {}\r\n".into(),
        "fn café() {}\n".into(),
        false,
        Options {
            check: true,
            diff: true,
        },
    );
    let diagnostic = result.diagnostic.unwrap();
    assert_eq!(diagnostic.rule.as_deref(), Some("format"));
    assert_eq!(diagnostic.severity, DiagnosticLevel::Error);
    assert_eq!((diagnostic.line, diagnostic.column), (Some(1), Some(13)));
    assert!(diagnostic.message.contains("CRLF -> LF"));
    assert!(result.diff.is_some());
    let change = result.change.unwrap();
    assert_eq!(change.original, "fn café() {}\r\n");
    assert_eq!(change.formatted, "fn café() {}\n");
}

#[test]
fn newline_diagnostics_describe_mixed_input_and_configured_output_style() {
    assert!(difference_message("a\r\nb\n", "a\nb\n").contains("mixed LF/CRLF -> LF"));
    assert!(difference_message("a\nb\n", "a\r\nb\r\n").contains("LF -> CRLF"));
}

#[test]
fn content_changes_lone_cr_and_final_newlines_are_not_newline_style_only_changes() {
    for (original, formatted) in [
        ("fn a(){}\r\n", "fn a() {}\n"),
        ("a\r\n", "a"),
        ("a", "a\n"),
        ("a\rb\n", "ab\n"),
        ("a\r\r\n\n", "a\n\n"),
    ] {
        assert!(difference_message(original, formatted).contains("declaration-spacing result"));
    }
}
