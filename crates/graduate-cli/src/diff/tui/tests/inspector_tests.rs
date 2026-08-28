use super::super::render::render;
use super::super::update::update;
use super::*;

#[test]
fn loaded_jira_details_are_visible() -> Result<(), Box<dyn std::error::Error>> {
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
            ahead: 2,
            last_author: "Pat".to_owned(),
            commits: vec![test_commit("Add login"), test_commit("Add login tests")],
            merged_environments: Vec::new(),
            tip: String::new(),
            tip_in_environment: true,
            unmerged_ahead: 0,
            absorbed_environment_merges: 0,
            merge_onto_main: None,
            jira: JiraIssueState::Loaded(graduate::promotion::JiraIssueSummary {
                key: "PROJ-123".to_owned(),
                api_url: "https://example.atlassian.net/rest/api/3/issue/10001".to_owned(),
                summary: "Add login".to_owned(),
                status: "Ready for QA".to_owned(),
                status_category: None,
                assignee: Some("Pat".to_owned()),
                fix_versions: vec!["1.2".to_owned()],
                url: "https://example.atlassian.net/browse/PROJ-123".to_owned(),
            }),
        }))),
    )?;
    let mut terminal = Terminal::new(TestBackend::new(110, 48))?;
    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();

    assert!(rendered.contains("Ready for QA"));
    assert!(rendered.contains("Add login"));
    assert!(rendered.contains("Versions  1.2"));
    assert!(rendered.contains("feature/PROJ-123-login"));
    assert!(rendered.contains("1 of 1"));
    assert!(rendered.contains("Author  Pat"));
    assert!(rendered.contains("Commits  2"));
    assert!(rendered.contains("Updated  2024-01-02"));
    Ok(())
}

#[test]
fn inspector_separates_jira_status_from_branch_metadata() -> Result<(), Box<dyn std::error::Error>>
{
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
            ahead: 2,
            last_author: "Pat".to_owned(),
            commits: Vec::new(),
            merged_environments: Vec::new(),
            tip: String::new(),
            tip_in_environment: true,
            unmerged_ahead: 0,
            absorbed_environment_merges: 0,
            merge_onto_main: None,
            jira: JiraIssueState::NotConfigured {
                key: "PROJ-123".to_owned(),
            },
        }))),
    )?;
    let mut terminal = Terminal::new(TestBackend::new(110, 32))?;

    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();
    let lines = rendered.lines().collect::<Vec<_>>();
    let jira_row = lines
        .iter()
        .position(|line| line.contains("Jira  PROJ-123"))
        .ok_or("Jira status was not rendered")?;
    let author_row = lines
        .iter()
        .position(|line| line.contains("Author  Pat"))
        .ok_or("branch metadata was not rendered")?;

    assert!(author_row > jira_row);
    assert!(lines[jira_row..author_row]
        .iter()
        .any(|line| line.contains("────")));
    assert!(rendered.contains("Next  gd auth setup jira"));
    Ok(())
}

#[test]
fn very_short_inspector_keeps_branch_metadata_visible() -> Result<(), Box<dyn std::error::Error>> {
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
            ahead: 2,
            last_author: "Pat".to_owned(),
            commits: Vec::new(),
            merged_environments: Vec::new(),
            tip: String::new(),
            tip_in_environment: true,
            unmerged_ahead: 0,
            absorbed_environment_merges: 0,
            merge_onto_main: None,
            jira: JiraIssueState::NotConfigured {
                key: "PROJ-123".to_owned(),
            },
        }))),
    )?;
    let mut terminal = Terminal::new(TestBackend::new(110, 18))?;

    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();

    assert!(rendered.contains("Author  Pat"));
    assert!(rendered.contains("Commits  2"));
    assert!(rendered.contains("Updated  2024-01-02"));
    Ok(())
}

#[test]
fn environment_merged_branch_renders_red_with_a_footer_warning(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec![
                "feature/PROJ-123-login".to_owned(),
                "feature/PROJ-124-clean".to_owned(),
            ],
        })),
    )?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Measured(PromotionBranch {
            branch: "feature/PROJ-123-login".to_owned(),
            started: "2021-12-09".to_owned(),
            last: "2024-01-02".to_owned(),
            ahead: 3675,
            last_author: "Pat".to_owned(),
            commits: Vec::new(),
            merged_environments: vec!["qa".to_owned()],
            tip: String::new(),
            tip_in_environment: true,
            unmerged_ahead: 0,
            absorbed_environment_merges: 0,
            merge_onto_main: None,
            jira: JiraIssueState::NoTicket,
        }))),
    )?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Measured(PromotionBranch {
            branch: "feature/PROJ-124-clean".to_owned(),
            started: "2024-01-01".to_owned(),
            last: "2024-01-02".to_owned(),
            ahead: 2,
            last_author: "Pat".to_owned(),
            commits: Vec::new(),
            merged_environments: Vec::new(),
            tip: String::new(),
            tip_in_environment: true,
            unmerged_ahead: 0,
            absorbed_environment_merges: 0,
            merge_onto_main: None,
            jira: JiraIssueState::NoTicket,
        }))),
    )?;
    let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();
    let red_cells = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .any(|cell| cell.style().fg == Some(ratatui::style::Color::Red));

    assert!(rendered.contains("⚠ qa has been merged into this branch"));
    assert!(rendered.contains("feature/PROJ-123-login ⚠"));
    assert!(red_cells);

    update(&mut model, Message::MoveDown)?;
    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();

    assert!(!rendered.contains("has been merged into this branch"));
    assert!(rendered.contains("open Jira"));
    Ok(())
}

#[test]
fn unvalidated_jira_keys_leave_the_ticket_column_blank() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec!["CLAIMS-9".to_owned()],
        })),
    )?;
    update(
        &mut model,
        Message::Scan(Box::new(measured(
            "CLAIMS-9",
            "2024-01-01",
            "2024-01-02",
            2,
            JiraIssueState::NotFound {
                key: "CLAIMS-9".to_owned(),
            },
        ))),
    )?;
    let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();
    let row = rendered
        .lines()
        .find(|line| line.contains("2024-01-01"))
        .ok_or("table row was not rendered")?;

    assert_eq!(row.matches("CLAIMS-9").count(), 1);
    assert!(row.contains("not found"));
    Ok(())
}

#[test]
fn mostly_not_found_lookups_show_a_configuration_hint() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec!["AA-1".to_owned(), "BB-2".to_owned(), "CC-3".to_owned()],
        })),
    )?;
    for branch in ["AA-1", "BB-2", "CC-3"] {
        update(
            &mut model,
            Message::Scan(Box::new(measured(
                branch,
                "2024-01-01",
                "2024-01-02",
                1,
                JiraIssueState::NotFound {
                    key: branch.to_owned(),
                },
            ))),
        )?;
    }
    update(&mut model, Message::Scan(Box::new(DiffUpdate::Finished)))?;
    let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();

    assert!(rendered.contains("3 of 3 ticket lookups returned not found"));
    Ok(())
}

#[test]
fn done_jira_statuses_render_in_the_success_color() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec![
                "aaa-selected".to_owned(),
                "feature/PROJ-123-login".to_owned(),
            ],
        })),
    )?;
    update(
        &mut model,
        Message::Scan(Box::new(measured(
            "feature/PROJ-123-login",
            "2024-01-01",
            "2024-01-02",
            2,
            JiraIssueState::Loaded(graduate::promotion::JiraIssueSummary {
                key: "PROJ-123".to_owned(),
                api_url: "https://example.atlassian.net/rest/api/3/issue/10001".to_owned(),
                summary: "Add login".to_owned(),
                status: "Done".to_owned(),
                status_category: None,
                assignee: None,
                fix_versions: Vec::new(),
                url: "https://example.atlassian.net/browse/PROJ-123".to_owned(),
            }),
        ))),
    )?;
    let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

    terminal.draw(|frame| render(frame, &mut model))?;
    let green_cells = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .any(|cell| cell.style().fg == Some(ratatui::style::Color::Green));

    assert!(green_cells);
    Ok(())
}

#[test]
fn unmerged_work_is_visible_in_the_table_and_inspector() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec!["feature/extended".to_owned()],
        })),
    )?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Measured(PromotionBranch {
            branch: "feature/extended".to_owned(),
            started: "2024-01-01".to_owned(),
            last: "2024-01-02".to_owned(),
            ahead: 3,
            last_author: "Pat".to_owned(),
            commits: Vec::new(),
            merged_environments: Vec::new(),
            tip: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            tip_in_environment: false,
            unmerged_ahead: 2,
            absorbed_environment_merges: 0,
            merge_onto_main: None,
            jira: JiraIssueState::NoTicket,
        }))),
    )?;
    let mut terminal = Terminal::new(TestBackend::new(110, 48))?;
    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();

    assert!(rendered.contains("UNMERGED"));
    assert!(rendered.contains("Tip in env  no"));
    assert!(rendered.contains("Unmerged  2"));
    Ok(())
}
