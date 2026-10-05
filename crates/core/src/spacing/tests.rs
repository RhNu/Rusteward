use ra_ap_syntax::Edition;

use super::separate_declarations;

fn format(source: &str) -> String {
    separate_declarations(source, Edition::Edition2024)
        .unwrap()
        .text
}

#[test]
fn separates_selected_declarations() {
    let source = "fn a() {}\nstruct S;\nimpl S {}\ntrait T {}\nenum E {}\n";
    assert_eq!(
        format(source),
        "fn a() {}\n\nstruct S;\n\nimpl S {}\n\ntrait T {}\n\nenum E {}\n"
    );
}

#[test]
fn separates_a_selected_item_from_either_ignored_neighbor() {
    let source = "const A: u32 = 1;\nfn f() {}\ntype Alias = u32;\nstruct S;\nenum E {}\n";
    let expected =
        "const A: u32 = 1;\n\nfn f() {}\n\ntype Alias = u32;\n\nstruct S;\n\nenum E {}\n";
    assert_eq!(format(source), expected);
}

#[test]
fn separates_imports_from_selected_declarations_in_both_directions() {
    let source = "use a::B;\nuse c::{D, E};\nfn f() {}\nuse x::Y;\nuse z::W;\n";
    let expected = "use a::B;\nuse c::{D, E};\n\nfn f() {}\n\nuse x::Y;\nuse z::W;\n";
    assert_eq!(format(source), expected);
}

#[test]
fn leaves_other_adjacent_items_unchanged() {
    let source = "use a::B;\nconst A: u32 = 1;\nconst B: u32 = 2;\ntype Alias = u32;\nmod m {}\n";
    assert_eq!(format(source), source);
}

#[test]
fn keeps_fields_variants_and_local_statements_compact() {
    let source = "struct S {\n    a: u32,\n    b: u32,\n}\n\nenum E {\n    A,\n    B,\n}\n\nfn f() {\n    let a = 1;\n    let b = 2;\n    println!(\"{}\", a + b);\n}\n";
    assert_eq!(format(source), source);
}

#[test]
fn separates_enums_from_imports_and_constants_without_spacing_variants() {
    let source = "use a::B;\nenum E {\n    A,\n    B,\n}\nconst X: u32 = 1;\nconst Y: u32 = 2;\n";
    let expected =
        "use a::B;\n\nenum E {\n    A,\n    B,\n}\n\nconst X: u32 = 1;\nconst Y: u32 = 2;\n";
    assert_eq!(format(source), expected);
}

#[test]
fn separates_module_items_and_impl_and_trait_methods() {
    let source = "mod m {\n    const A: u32 = 1;\n    fn a() {}\n    fn b() {}\n}\nimpl S {\n    type A = u32;\n    const B: u32 = 1;\n    fn c() {}\n}\ntrait T {\n    type A;\n    const B: u32;\n    fn c();\n    fn d();\n}\n";
    let expected = "mod m {\n    const A: u32 = 1;\n\n    fn a() {}\n\n    fn b() {}\n}\n\nimpl S {\n    type A = u32;\n    const B: u32 = 1;\n\n    fn c() {}\n}\n\ntrait T {\n    type A;\n    const B: u32;\n\n    fn c();\n\n    fn d();\n}\n";
    assert_eq!(format(source), expected);
}

#[test]
fn keeps_doc_comments_and_stacked_attributes_with_their_declaration() {
    let source = "fn a() {}\n/// Doc.\n#[must_use]\n#[cfg(any())]\nfn b() {}\n// Leading comment.\nfn c() {}\n";
    let expected = "fn a() {}\n\n/// Doc.\n#[must_use]\n#[cfg(any())]\nfn b() {}\n\n// Leading comment.\nfn c() {}\n";
    assert_eq!(format(source), expected);
}

#[test]
fn preserves_trailing_comments_and_multiline_raw_strings() {
    let source = "const A: &str = r#\"}\nfn fake() {}\"#; // trailing\n// leading\nfn b() {}\n";
    let expected = "const A: &str = r#\"}\nfn fake() {}\"#; // trailing\n\n// leading\nfn b() {}\n";
    assert_eq!(format(source), expected);
}

#[test]
fn preserves_nested_block_comments_and_their_attachment() {
    let source =
        "fn a() {} /* trailing /* nested */ comment */\n/* leading\n   comment */\nfn b() {}\n";
    let expected =
        "fn a() {} /* trailing /* nested */ comment */\n\n/* leading\n   comment */\nfn b() {}\n";
    assert_eq!(format(source), expected);
}

#[test]
fn leaves_macro_token_trees_opaque() {
    let source = "macro_rules! m {\n    () => {\n        fn a() {}\n        fn b() {}\n    };\n}\nfn c() {}\n";
    let expected = "macro_rules! m {\n    () => {\n        fn a() {}\n        fn b() {}\n    };\n}\n\nfn c() {}\n";
    assert_eq!(format(source), expected);
}

#[test]
fn preserves_newline_style_and_reports_original_lines() {
    let source = "fn a() {}\r\nfn b() {}\r\n";
    let result = separate_declarations(source, Edition::Edition2024).unwrap();
    assert_eq!(result.text, "fn a() {}\r\n\r\nfn b() {}\r\n");
    assert_eq!(result.missing_lines, [2]);
}

#[test]
fn handles_utf8_offsets() {
    assert_eq!(
        format("fn café() {}\nfn b() {}\n"),
        "fn café() {}\n\nfn b() {}\n"
    );
}

#[test]
fn preserves_existing_blank_lines_and_is_idempotent() {
    let source = "fn a() {}\n\n\nfn b() {}\nfn c() {}\n";
    let output = format(source);
    assert_eq!(output, "fn a() {}\n\n\nfn b() {}\n\nfn c() {}\n");
    assert_eq!(format(&output), output);
    assert_eq!(
        separate_declarations(&output, Edition::Edition2024)
            .unwrap()
            .missing_lines,
        [] as [usize; 0]
    );
}

#[test]
fn honors_file_skip() {
    let source = "#![rustfmt::skip]\nfn a() {}\nfn b() {}\n";
    let result = separate_declarations(source, Edition::Edition2024).unwrap();
    assert_eq!(result.text, source);
    assert!(result.skip_reason.is_some());
}

#[test]
fn preserves_skipped_containers_but_separates_their_neighbors() {
    let source = "#[rustfmt::skip]\nimpl S {\n    fn a() {}\n    fn b() {}\n}\nfn c() {}\n";
    let expected = "#[rustfmt::skip]\nimpl S {\n    fn a() {}\n    fn b() {}\n}\n\nfn c() {}\n";
    assert_eq!(format(source), expected);
}

#[test]
fn honors_inner_module_skip_and_conditional_skip() {
    let source = "mod m {\n    #![rustfmt::skip]\n    fn a() {}\n    fn b() {}\n}\n#[cfg_attr(any(), rustfmt::skip)]\nimpl S {\n    fn a() {}\n    fn b() {}\n}\n";
    let expected = "mod m {\n    #![rustfmt::skip]\n    fn a() {}\n    fn b() {}\n}\n\n#[cfg_attr(any(), rustfmt::skip)]\nimpl S {\n    fn a() {}\n    fn b() {}\n}\n";
    assert_eq!(format(source), expected);
}

#[test]
fn skips_generated_files_before_parsing() {
    let source = "// @generated\nnot valid Rust!\n";
    let result = separate_declarations(source, Edition::Edition2024).unwrap();
    assert_eq!(result.text, source);
    assert!(result.skip_reason.is_some());
}

#[test]
fn rejects_invalid_and_not_yet_formatted_inputs() {
    assert!(separate_declarations("fn broken( {", Edition::Edition2024).is_err());
    assert!(separate_declarations("fn a() {} fn b() {}", Edition::Edition2024).is_err());
}
