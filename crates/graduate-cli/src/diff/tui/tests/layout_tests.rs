use ratatui::layout::Rect;

use super::super::render::{modal_area, render};
use super::super::update::update;
use super::*;
use crate::shared::theme;

#[test]
fn every_modal_uses_the_wide_content_viewport() {
    assert_eq!(modal_area(Rect::new(0, 0, 160, 48), 34).width, 115);
    assert_eq!(modal_area(Rect::new(0, 0, 90, 48), 24).width, 90);
}

#[test]
fn renders_skeleton_rows_before_measurements_finish() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    update(
        &mut model,
        Message::Scan(Box::new(DiffUpdate::Skeleton {
            environment: "qa".to_owned(),
            main: "main".to_owned(),
            branches: vec!["feature/PROJ-123-login".to_owned()],
        })),
    )?;
    let mut terminal = Terminal::new(TestBackend::new(110, 48))?;
    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();

    assert!(rendered.contains("In qa but not main"));
    assert!(rendered.contains("feature/PROJ-123-login"));
    assert!(rendered.contains("measuring"));
    Ok(())
}

#[test]
fn completed_report_calls_out_commits_behind_main() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    model.environment = "qa".to_owned();
    model.main = "main".to_owned();
    model.finished = true;
    model
        .inventory
        .behind_main
        .push(test_commit("Main-only work"));
    let mut terminal = Terminal::new(TestBackend::new(110, 32))?;

    terminal.draw(|frame| render(frame, &mut model))?;

    let rendered = terminal.backend().to_string();
    assert!(rendered.contains("1 behind main"), "{rendered}");
    Ok(())
}

#[test]
fn short_wide_report_places_selected_branch_inspector_beside_the_table(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    let mut terminal = Terminal::new(TestBackend::new(110, 32))?;

    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();
    let table_column = rendered
        .lines()
        .find_map(|line| line.find("BRANCH"))
        .ok_or("branch heading was not rendered")?;
    let inspector_column = rendered
        .lines()
        .find_map(|line| line.find("No branch selected"))
        .ok_or("selected branch inspector was not rendered")?;
    let summary_column = rendered
        .lines()
        .find_map(|line| line.find("In  but not"))
        .ok_or("report summary was not rendered")?;

    assert!(inspector_column > table_column + 40);
    assert!(summary_column > table_column + 40);
    assert!(rendered.contains("GRADUATE"));
    assert!(!rendered.contains("Promotion report"));
    assert!(!rendered.contains(theme::GRADUATE_ART[0]));
    Ok(())
}

#[test]
fn tall_report_stacks_details_above_the_full_table() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    let mut terminal = Terminal::new(TestBackend::new(110, 48))?;

    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();
    let lines = rendered.lines().collect::<Vec<_>>();
    let details_row = lines
        .iter()
        .position(|line| line.contains("No branch selected"))
        .ok_or("detail card title was not rendered")?;
    let table_row = lines
        .iter()
        .position(|line| line.contains("BRANCH") && line.contains("JIRA"))
        .ok_or("full table heading was not rendered")?;

    assert!(details_row < table_row, "rows: {lines:#?}");
    assert!(!rendered.contains(" SELECTED "));
    assert!(rendered.contains(theme::GRADUATE_ART[0]));
    Ok(())
}

#[test]
fn narrow_report_stacks_details_above_the_table() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = test_model()?;
    let mut terminal = Terminal::new(TestBackend::new(90, 48))?;

    terminal.draw(|frame| render(frame, &mut model))?;
    let rendered = terminal.backend().to_string();
    let lines: Vec<_> = rendered.lines().collect();
    let details_row = lines
        .iter()
        .position(|line| line.contains("No branch selected"))
        .ok_or("detail card title was not rendered")?;
    let table_row = lines
        .iter()
        .position(|line| line.contains("BRANCH"))
        .ok_or("table heading was not rendered")?;

    assert!(details_row < table_row, "rows: {lines:#?}");
    Ok(())
}
