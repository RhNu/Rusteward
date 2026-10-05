use super::worker_count;

#[test]
fn automatic_workers_follow_available_cpus_and_file_count() {
    assert_eq!(worker_count(0, 8, 20), 8);
    assert_eq!(worker_count(0, 8, 3), 3);
    assert_eq!(worker_count(0, 0, 20), 1);
}

#[test]
fn explicit_workers_override_cpu_count_with_a_file_limit() {
    assert_eq!(worker_count(4, 2, 20), 4);
    assert_eq!(worker_count(20, 8, 3), 3);
    assert_eq!(worker_count(1, 8, 20), 1);
}

#[test]
fn empty_and_single_file_inputs_need_no_parallel_workers() {
    assert_eq!(worker_count(0, 8, 0), 0);
    assert_eq!(worker_count(4, 8, 0), 0);
    assert_eq!(worker_count(0, 8, 1), 1);
    assert_eq!(worker_count(4, 8, 1), 1);
}
