use super::SourceText;

#[test]
fn preserves_lf_lone_cr_and_literal_escape_sequences() {
    for source in ["", "汉字\ntext\n", "a\rb", "\\r\\n"] {
        let input = SourceText::new(source);
        assert_eq!(input.text, source);
        for offset in 0..=source.len() {
            assert_eq!(input.original_offset(offset), offset);
        }
    }
}

#[test]
fn normalizes_crlf_once_without_collapsing_new_pairs() {
    let input = SourceText::new("a\r\r\n\nb\r\n");
    assert_eq!(input.text, "a\r\n\nb\n");
    assert_eq!(input.original_offset(2), 2);
    assert_eq!(input.original_offset(3), 4);
    assert_eq!(input.original_offset(6), 8);
}

#[test]
fn maps_unicode_mixed_newlines_and_end_of_file_to_original_boundaries() {
    let source = "汉\r\n字\n末\r\n";
    let input = SourceText::new(source);
    assert_eq!(input.text, "汉\n字\n末\n");
    for (normalized, original) in [(0, 0), (3, 3), (4, 5), (7, 8), (8, 9), (11, 12), (12, 14)] {
        assert_eq!(input.original_offset(normalized), original);
        assert!(source.is_char_boundary(original));
    }
    assert_eq!(
        &source[input.original_offset(3)..input.original_offset(4)],
        "\r\n"
    );
}

#[test]
fn maps_consecutive_crlf_boundaries_independently() {
    let input = SourceText::new("\r\n\r\n");
    assert_eq!(input.text, "\n\n");
    assert_eq!(input.original_offset(0), 0);
    assert_eq!(input.original_offset(1), 2);
    assert_eq!(input.original_offset(2), 4);
}
