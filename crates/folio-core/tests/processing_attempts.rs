use folio_core::models::{AttemptOutcome, ProcessingAttempt, StageOutcome};

#[test]
fn retryable_failure_does_not_become_terminal_failure() {
    let mut stage = ProcessingAttempt::default();
    assert!(stage.start(1));
    assert!(stage.finish(1, AttemptOutcome::Retry));
    assert_eq!(stage.outcome(), StageOutcome::WaitingToRetry);
    assert!(stage.start(2));
    assert!(stage.finish(2, AttemptOutcome::Completed));
    assert_eq!(stage.outcome(), StageOutcome::Completed);
}

#[test]
fn late_attempt_cannot_complete_its_replacement() {
    let mut stage = ProcessingAttempt::default();
    stage.start(1);
    stage.finish(1, AttemptOutcome::Retry);
    stage.start(2);
    assert!(!stage.finish(1, AttemptOutcome::Completed));
    assert_eq!(stage.outcome(), StageOutcome::Running);
}

#[test]
fn duplicate_completion_and_restart_do_not_change_terminal_run() {
    let mut stage = ProcessingAttempt::default();
    stage.start(1);
    stage.finish(1, AttemptOutcome::Completed);
    assert!(!stage.finish(1, AttemptOutcome::Completed));
    assert!(!stage.start(2));
    assert_eq!(stage.outcome(), StageOutcome::Completed);
}

#[test]
fn exhausted_failure_remains_terminal() {
    let mut stage = ProcessingAttempt::default();
    stage.start(1);
    assert!(stage.finish(1, AttemptOutcome::Failed));
    assert!(!stage.start(2));
    assert_eq!(stage.outcome(), StageOutcome::Failed);
}

#[test]
fn skipped_stage_cannot_later_report_completion() {
    let mut stage = ProcessingAttempt::default();
    stage.start(1);
    stage.finish(1, AttemptOutcome::Skipped);
    assert!(!stage.finish(1, AttemptOutcome::Completed));
    assert_eq!(stage.outcome(), StageOutcome::Skipped);
}

#[test]
fn unstarted_and_out_of_order_attempts_are_rejected() {
    let mut stage = ProcessingAttempt::default();
    assert!(!stage.finish(0, AttemptOutcome::Completed));
    assert!(!stage.start(0));
    assert!(!stage.start(2));
    assert!(stage.start(1));
    assert!(!stage.start(2));
}

#[test]
fn restored_retry_state_continues_without_replaying_old_attempts() {
    let mut stage = ProcessingAttempt::restore(4, StageOutcome::WaitingToRetry).unwrap();
    assert!(stage.start(5));
    assert!(!stage.finish(4, AttemptOutcome::Completed));
    assert!(stage.finish(5, AttemptOutcome::Completed));
    assert_eq!(stage.number(), 5);
}

#[test]
fn inconsistent_persisted_attempt_states_are_rejected() {
    assert!(ProcessingAttempt::restore(0, StageOutcome::Running).is_none());
    assert!(ProcessingAttempt::restore(1, StageOutcome::Pending).is_none());
}

#[test]
fn exhausted_attempt_numbers_cannot_wrap_and_restart() {
    let mut stage = ProcessingAttempt::restore(u32::MAX, StageOutcome::WaitingToRetry).unwrap();
    assert!(!stage.start(0));
    assert_eq!(stage.number(), u32::MAX);
    assert_eq!(stage.outcome(), StageOutcome::WaitingToRetry);
}
