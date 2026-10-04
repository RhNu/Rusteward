use std::path::Path;

use rusteward_core::Edition;

use super::{excluded, exclusions, parse_edition};

#[test]
fn globs_match_files_and_prune_subtrees() {
    let globs = exclusions(&[
        "generated/**".into(),
        "**/vendor/**".into(),
        "src/legacy.rs".into(),
    ])
    .unwrap();
    let root = Path::new("project");
    assert!(excluded(Path::new("project/generated"), true, root, &globs));
    assert!(excluded(
        Path::new("project/crate/vendor/a.rs"),
        false,
        root,
        &globs
    ));
    assert!(excluded(
        Path::new("project/src/legacy.rs"),
        false,
        root,
        &globs
    ));
    assert!(!excluded(
        Path::new("project/src/current.rs"),
        false,
        root,
        &globs
    ));
    assert!(exclusions(&["[".into()]).is_err());
}

#[test]
fn editions_are_explicit() {
    assert_eq!(parse_edition("2024").unwrap(), Edition::Edition2024);
    assert!(parse_edition("2099").is_err());
}

#[test]
fn outside_members_use_workspace_relative_exclusions() {
    let globs = exclusions(&["../shared/**".into()]).unwrap();
    assert!(excluded(
        Path::new("project/shared/src/lib.rs"),
        false,
        Path::new("project/workspace"),
        &globs,
    ));
}
