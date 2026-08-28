use super::super::interactive::{
    finish_interactive, interactive_error, interactive_resume_requested, InteractiveOutcome,
};
use super::super::machine_output::machine_failure;
use super::*;
use crate::cli::RestackArgs;

#[test]
fn interactive_completion_restores_before_ordinary_output() -> Result<(), Box<dyn std::error::Error>>
{
    let terminal = Rc::new(RefCell::new(Terminal::new(TestBackend::new(40, 10))?));
    terminal.borrow_mut().hide_cursor()?;
    let restored = Rc::new(RefCell::new(false));
    let restore_terminal = Rc::clone(&terminal);
    let restore_state = Rc::clone(&restored);
    let write_state = Rc::clone(&restored);

    finish_interactive(
        Ok(InteractiveOutcome::Cancelled("qa".to_owned())),
        move || {
            let _ = restore_terminal.borrow_mut().show_cursor();
            *restore_state.borrow_mut() = true;
            Ok(())
        },
        move |_| {
            assert!(*write_state.borrow());
            Ok(())
        },
    )?;
    Ok(())
}

#[test]
fn interactive_failure_still_restores_and_skips_ordinary_output(
) -> Result<(), Box<dyn std::error::Error>> {
    let terminal = Rc::new(RefCell::new(Terminal::new(TestBackend::new(40, 10))?));
    terminal.borrow_mut().hide_cursor()?;
    let restored = Rc::new(RefCell::new(false));
    let wrote = Rc::new(RefCell::new(false));
    let restore_terminal = Rc::clone(&terminal);
    let restore_state = Rc::clone(&restored);
    let write_state = Rc::clone(&wrote);

    let result = finish_interactive(
        Err(machine_failure("fetch_failed", "fetch failed", json!({}))),
        move || {
            let _ = restore_terminal.borrow_mut().show_cursor();
            *restore_state.borrow_mut() = true;
            Ok(())
        },
        move |_| {
            *write_state.borrow_mut() = true;
            Ok(())
        },
    );

    assert!(matches!(result, Err(CliError::Restack(message)) if message == "fetch failed"));
    assert!(*restored.borrow());
    assert!(!*wrote.borrow());
    Ok(())
}

#[test]
fn interactive_error_keeps_structured_details() {
    let error = interactive_error(machine_failure(
        "unsupported_history",
        "the environment history cannot be reconstructed without guessing",
        json!({"kind": "ambiguousFeatureRefs", "mergeCommit": "886faef"}),
    ));
    assert_eq!(
        error.to_string(),
        "restack failed: the environment history cannot be reconstructed without guessing ({\"kind\":\"ambiguousFeatureRefs\",\"mergeCommit\":\"886faef\"})"
    );
}

fn resume_args() -> RestackArgs {
    RestackArgs {
        environment: "qa".to_owned(),
        main: None,
        remote: None,
        params: None,
        dry_run: false,
        apply: false,
        resume: Some("v1.token".to_owned()),
        abort: false,
    }
}

#[test]
fn bare_resume_from_a_terminal_reviews_interactively() {
    assert!(interactive_resume_requested(&resume_args(), true));
}

#[test]
fn resume_stays_machine_readable_without_a_terminal_or_with_apply_or_abort() {
    assert!(!interactive_resume_requested(&resume_args(), false));
    let apply = RestackArgs {
        apply: true,
        ..resume_args()
    };
    assert!(!interactive_resume_requested(&apply, true));
    let abort = RestackArgs {
        abort: true,
        ..resume_args()
    };
    assert!(!interactive_resume_requested(&abort, true));
    let fresh = RestackArgs {
        resume: None,
        ..resume_args()
    };
    assert!(!interactive_resume_requested(&fresh, true));
}
