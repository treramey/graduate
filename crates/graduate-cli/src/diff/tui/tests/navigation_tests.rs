use crossterm::event::{KeyCode, KeyModifiers};

use super::super::render::render;
use super::super::update::{finish_after_update_channel_closes, message_for_key, update};
use super::*;

#[test]
fn moving_to_another_branch_clears_the_open_ticket_warning(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec![
                "feature/no-ticket".to_owned(),
                "feature/PROJ-123".to_owned(),
            ],
        })),
    )?;
    update(&mut model, Message::OpenTicket)?;

    update(&mut model, Message::MoveDown)?;

    assert_eq!(model.selected, 1);
    assert!(model.warning.is_none());
    Ok(())
}

#[test]
fn moving_up_after_scrolling_moves_the_selection_within_the_viewport(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: (0..20).map(|index| format!("branch-{index:02}")).collect(),
        })),
    )?;
    let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

    update(&mut model, Message::SelectLast)?;
    for _ in 0..4 {
        update(&mut model, Message::MoveUp)?;
    }
    terminal.draw(|frame| render(frame, &mut model))?;
    let before = terminal.backend().to_string();
    update(&mut model, Message::MoveUp)?;
    terminal.draw(|frame| render(frame, &mut model))?;
    let after = terminal.backend().to_string();

    assert_eq!(first_visible_branch(&before), first_visible_branch(&after));
    Ok(())
}

#[test]
fn update_channel_must_not_close_before_finished() -> Result<(), Box<dyn std::error::Error>> {
    let result = finish_after_update_channel_closes(test_model()?);

    assert!(
        matches!(result, Err(CliError::Git(message)) if message.contains("before the scan completed"))
    );
    Ok(())
}

#[test]
fn table_columns_stay_compact_and_ahead_counts_right_align(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec!["aa".to_owned(), "bb".to_owned()],
        })),
    )?;
    update(
        &mut model,
        Message::Scan(Box::new(measured(
            "aa",
            "2011-01-01",
            "2011-01-02",
            3,
            JiraIssueState::NoTicket,
        ))),
    )?;
    update(
        &mut model,
        Message::Scan(Box::new(measured(
            "bb",
            "2011-01-01",
            "2011-01-02",
            28,
            JiraIssueState::NoTicket,
        ))),
    )?;
    let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();
    let lines = rendered.lines().collect::<Vec<_>>();
    let header = lines
        .iter()
        .find(|line| line.contains("BRANCH") && line.contains("AHEAD"))
        .ok_or("table heading was not rendered")?;
    let branch_column =
        character_position(header, "BRANCH").ok_or("BRANCH heading was not rendered")?;
    let started_column =
        character_position(header, "STARTED").ok_or("STARTED heading was not rendered")?;
    let ahead_end =
        character_position(header, "AHEAD").ok_or("AHEAD heading was not rendered")? + 4;
    let row_a = lines
        .iter()
        .find(|line| line.contains("aa") && line.contains("2011-01-01"))
        .ok_or("row aa was not rendered")?;
    let row_b = lines
        .iter()
        .find(|line| line.contains("bb") && line.contains("2011-01-01"))
        .ok_or("row bb was not rendered")?;

    assert_eq!(started_column - branch_column, 26);
    assert!(rendered.contains("2011-01-01  2011-01-02"));
    assert!(row_a.contains("not found"));
    assert_eq!(row_a.chars().nth(ahead_end), Some('3'));
    assert_eq!(row_b.chars().nth(ahead_end), Some('8'));
    Ok(())
}

#[test]
fn s_cycles_the_sort_and_selection_follows_the_branch() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec!["aa".to_owned(), "bb".to_owned()],
        })),
    )?;
    update(
        &mut model,
        Message::Scan(Box::new(measured(
            "aa",
            "2024-05-01",
            "2024-05-02",
            1,
            JiraIssueState::NoTicket,
        ))),
    )?;
    update(
        &mut model,
        Message::Scan(Box::new(measured(
            "bb",
            "2022-01-01",
            "2022-01-02",
            9,
            JiraIssueState::NoTicket,
        ))),
    )?;
    let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

    let message = message_for_key(&model, KeyCode::Char('s'), KeyModifiers::NONE)
        .ok_or("s did not map to a sort message")?;
    update(&mut model, message)?;
    terminal.draw(|frame| render(frame, &mut model))?;
    let by_started = terminal.backend().to_string();

    assert_eq!(model.rows[0].branch, "bb");
    assert_eq!(model.selected, 1);
    assert!(by_started.contains("STARTED ▲"));

    update(&mut model, Message::CycleSort)?;
    update(&mut model, Message::CycleSort)?;
    terminal.draw(|frame| render(frame, &mut model))?;
    let by_ahead = terminal.backend().to_string();

    assert_eq!(model.rows[0].branch, "bb");
    assert!(by_ahead.contains("AHEAD ▼"));

    let report = model.completed_report();
    assert_eq!(report.branches[0].branch, "aa");
    Ok(())
}
