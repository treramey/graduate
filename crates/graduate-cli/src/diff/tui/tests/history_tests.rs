use crossterm::event::{KeyCode, KeyModifiers};

use super::super::render::render;
use super::super::update::{message_for_key, update};
use super::*;

#[test]
fn history_list_scrolls_to_any_number_of_commits() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec!["feature/PROJ-123-login".to_owned()],
        })),
    )?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Measured(PromotionBranch {
            branch: "feature/PROJ-123-login".to_owned(),
            started: "2024-01-01".to_owned(),
            last: "2024-01-02".to_owned(),
            ahead: 50,
            last_author: "Pat".to_owned(),
            commits: (1..=50)
                .map(|index| test_commit(&format!("Commit {index}")))
                .collect(),
            merged_environments: Vec::new(),
            jira: JiraIssueState::NoTicket,
        }))),
    )?;
    update(&mut model, Message::OpenHistory)?;
    for _ in 1..50 {
        update(&mut model, Message::ScrollHistoryDown)?;
    }
    let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();

    assert_eq!(model.history_selected, 49);
    assert!(rendered.contains("Commit 50"));
    Ok(())
}

#[test]
fn history_sheet_uses_the_wide_modal_and_explains_the_comparison(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    model.main = "main".to_owned();
    model.rows.push(BranchRow {
        branch: "feature/PROJ-123-login".to_owned(),
        report: Some(PromotionBranch {
            branch: "feature/PROJ-123-login".to_owned(),
            started: "2024-01-01".to_owned(),
            last: "2024-01-02".to_owned(),
            ahead: 1,
            last_author: "Pat".to_owned(),
            commits: vec![test_commit("DEMO-101 Add authentication")],
            merged_environments: Vec::new(),
            jira: JiraIssueState::NoTicket,
        }),
    });
    update(&mut model, Message::OpenHistory)?;
    let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();
    let lines = rendered.lines().collect::<Vec<_>>();
    let top = lines
        .iter()
        .position(|line| line.contains("Commits ahead of main"))
        .ok_or("history title was not rendered")?;
    let bottom = lines
        .iter()
        .position(|line| line.contains("Esc/h") && line.contains("close history"))
        .ok_or("history footer was not rendered")?;
    let headings = lines
        .iter()
        .find(|line| line.contains("SHA") && line.contains("SUBJECT"))
        .ok_or("history headings were not rendered")?;
    let commit = lines
        .iter()
        .find(|line| line.contains("a1b2c3d"))
        .ok_or("history commit was not rendered")?;
    let character_position = |line: &str, needle: &str| {
        line.find(needle)
            .map(|byte_index| line[..byte_index].chars().count())
    };

    assert!(rendered.contains("feature/PROJ-123-login  ·  1 commit  ·  newest first"));
    assert!(!lines[top + 1].contains("feature/PROJ-123-login"));
    assert!(lines[top + 2].contains("feature/PROJ-123-login"));
    assert!(!lines[top + 3].contains("feature/PROJ-123-login"));
    assert!(rendered.contains("SHA"));
    assert!(rendered.contains("SUBJECT"));
    assert!(rendered.contains("AUTHOR"));
    assert!(rendered.contains("DATE"));
    assert!(rendered.contains("a1b2c3d"));
    assert!(rendered.contains("DEMO-101 Add authentication"));
    assert!(rendered.contains("Pat"));
    assert!(rendered.contains("2024-01-02"));
    assert!(rendered.contains("1 of 1"));
    assert_eq!(
        character_position(headings, "SHA"),
        character_position(commit, "a1b2c3d")
    );
    assert_eq!(
        character_position(headings, "SUBJECT"),
        character_position(commit, "DEMO-101 Add authentication")
    );
    assert!(!lines[top].contains("2024-01-02"));
    assert!(bottom.saturating_sub(top) >= 8);
    assert!(bottom.saturating_sub(top) < 12);
    Ok(())
}

#[test]
fn h_closes_the_open_history_sheet() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    model.history_open = true;

    let message = message_for_key(&model, KeyCode::Char('h'), KeyModifiers::NONE);
    if let Some(message) = message {
        update(&mut model, message)?;
    }

    assert!(!model.history_open);
    Ok(())
}
