use crossterm::event::{KeyCode, KeyModifiers};

use super::super::render::render;
use super::super::update::{message_for_key, update};
use super::*;

#[test]
fn a_opens_and_closes_the_age_report_modal() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = DiffModel::new(ReportDate::parse("2026-08-04")?);
    let legacy_commit = PromotionCommit {
        id: "222222222222".to_owned(),
        short_id: "2222222".to_owned(),
        subject: "Legacy".to_owned(),
        author: "Pat".to_owned(),
        date: "2019-12-31".to_owned(),
    };
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec!["feature/legacy".to_owned()],
        })),
    )?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Measured(PromotionBranch {
            branch: "feature/legacy".to_owned(),
            started: "2019-12-31".to_owned(),
            last: "2019-12-31".to_owned(),
            ahead: 1,
            last_author: "Pat".to_owned(),
            commits: vec![legacy_commit.clone()],
            merged_environments: Vec::new(),
            jira: JiraIssueState::NoTicket,
        }))),
    )?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Inventory(EnvironmentInventory {
            ahead: vec![legacy_commit],
            behind_main: Vec::new(),
        }))),
    )?;
    update(&mut model, Message::Scan(Box::new(DiffUpdate::Finished)))?;

    let message = message_for_key(&model, KeyCode::Char('a'), KeyModifiers::NONE)
        .ok_or("a did not map to the age report")?;
    update(&mut model, message)?;
    let mut terminal = Terminal::new(TestBackend::new(110, 48))?;
    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();

    assert!(rendered.contains("The age of unshipped work"));
    assert!(rendered.contains("2019"));
    assert!(!rendered.contains("Before 2020"));
    assert!(rendered.contains("Will not ship without a decision"));
    assert!(rendered.contains("feature/legacy"));

    let mut short_terminal = Terminal::new(TestBackend::new(110, 18))?;
    short_terminal.draw(|frame| render(frame, &mut model))?;
    let short_rendered = short_terminal.backend().to_string();
    assert!(short_rendered.contains("The age of unshipped work"));
    assert!(short_rendered.contains("Will not ship without a decision"));

    let close = message_for_key(&model, KeyCode::Char('a'), KeyModifiers::NONE)
        .ok_or("a did not close the age report")?;
    update(&mut model, close)?;
    assert!(model.age_report.is_none());
    Ok(())
}

#[test]
fn age_report_waits_for_a_complete_scan() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;

    update(&mut model, Message::OpenAgeReport)?;

    assert!(model.age_report.is_none());
    assert_eq!(
        model.warning.as_deref(),
        Some("The age report is available when the scan completes.")
    );
    Ok(())
}

#[test]
fn age_report_scrolls_through_every_authored_year() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    model.finished = true;
    model.environment = "qa".to_owned();
    model.main = "main".to_owned();
    let commits = (2000..=2026)
        .rev()
        .map(|year| PromotionCommit {
            id: format!("commit-{year}"),
            short_id: year.to_string(),
            subject: format!("Work from {year}"),
            author: "Pat".to_owned(),
            date: format!("{year}-01-01"),
        })
        .collect::<Vec<_>>();
    model.inventory.ahead = commits.clone();
    model.rows.push(BranchRow {
        branch: "feature/history".to_owned(),
        report: Some(PromotionBranch {
            branch: "feature/history".to_owned(),
            started: "2000-01-01".to_owned(),
            last: "2026-01-01".to_owned(),
            ahead: 27,
            last_author: "Pat".to_owned(),
            commits,
            merged_environments: Vec::new(),
            jira: JiraIssueState::NoTicket,
        }),
    });
    update(&mut model, Message::OpenAgeReport)?;

    let scroll = message_for_key(&model, KeyCode::Down, KeyModifiers::NONE)
        .ok_or("down did not map to age-report scrolling")?;
    update(&mut model, scroll)?;
    for _ in 1..28 {
        update(&mut model, Message::ScrollAgeDown)?;
    }
    let mut terminal = Terminal::new(TestBackend::new(110, 32))?;
    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();

    assert_eq!(model.age_selected, 28);
    assert!(rendered.contains("Older than one year"));
    assert!(rendered.contains("2000"));
    Ok(())
}
