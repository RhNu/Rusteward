use super::writer_panic;

#[test]
fn thread_panic_errors_preserve_borrowed_and_owned_messages() {
    let borrowed = "synthetic writer failure";
    let owned = String::from("another synthetic writer failure");
    assert!(writer_panic(&borrowed).to_string().contains(borrowed));
    assert!(writer_panic(&owned).to_string().contains(&owned));
}

#[test]
fn opaque_panic_payloads_still_report_the_failing_operation() {
    let error = writer_panic(&42_u32);
    assert!(error.to_string().contains("writer panicked"));
}
