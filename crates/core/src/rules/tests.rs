use std::path::Path;

use super::{Rules, inspect, location};
use crate::{Edition, diagnostic::Severity};

fn inspect_text(path: &str, text: &str, rules: &Rules) -> Vec<crate::diagnostic::Diagnostic> {
    inspect(Path::new(path), text, Edition::Edition2024, rules)
}

#[test]
fn flags_mod_rs_and_only_inline_test_modules() {
    let source = "#[cfg(test)]\nmod tests;\nmod ordinary {}\nmod tests { fn a() {} }\n";
    let found = inspect_text("src/mod.rs", source, &Rules::default());
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].rule, "mod-rs");
    assert_eq!(found[0].severity, Severity::Error);
    assert_eq!(found[1].rule, "inline-tests");
    assert_eq!(found[1].severity, Severity::Warning);
    assert_eq!(found[1].line, 4);
}

#[test]
fn comments_strings_and_macros_do_not_create_modules() {
    let source = "// mod tests {}\nconst TEXT: &str = \"mod tests {}\";\nmacro_rules! m { () => { mod tests {} }; }\n";
    assert!(inspect_text("src/lib.rs", source, &Rules::default()).is_empty());
}

#[test]
fn disabled_rules_and_generated_files_do_not_report() {
    let rules = Rules {
        mod_rs: Severity::Off,
        inline_tests: Severity::Off,
        ..Rules::default()
    };
    assert!(inspect_text("src/mod.rs", "mod tests {}", &rules).is_empty());
    assert!(
        inspect_text(
            "src/mod.rs",
            "// @generated\ninvalid source",
            &Rules::default()
        )
        .is_empty()
    );
}

#[test]
fn line_tiers_use_custom_strict_limits_and_disabled_tier_falls_back() {
    let mut rules = Rules::default();
    rules.lines.warn = 2;
    rules.lines.error = 3;
    assert!(inspect_text("a.rs", "const A: u8 = 0;\n".repeat(2).as_str(), &rules).is_empty());
    let source = "const A: u8 = 0;\n".repeat(4);
    assert_eq!(
        inspect_text("a.rs", &source, &rules)[0].severity,
        Severity::Error
    );
    rules.lines.error_level = Severity::Off;
    assert_eq!(
        inspect_text("a.rs", &source, &rules)[0].severity,
        Severity::Warning
    );
}

#[test]
fn test_gated_modules_with_other_names_are_reported() {
    let found = inspect_text("a.rs", "#[cfg(test)]\nmod unit {}\n", &Rules::default());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].rule, "inline-tests");
    assert_eq!(location("汉字\n  fn x() {}", 11), (2, 5));
}
