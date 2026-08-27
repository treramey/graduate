//! Terminal selection, review, and conflict handoff for interactive restacks.

use std::io::{self, Write};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use graduate::promotion::jira_key_from_branch;
use graduate::restack::{
    RestackInteraction, RestackInteractionAction, RestackInteractionEffect,
    RestackInteractionStage, RestackPlan, RestackSelection, SelectionError,
};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::error::CliError;
use crate::terminal::StderrTerminal;
use crate::terminal_text::escape;
use crate::theme::{constrain_content_width, render_brand_header, Palette, GRADUATE_ART_HEIGHT};

pub(crate) enum SelectionDecision {
    Preview(RestackSelection),
    Cancel,
}

pub(crate) enum ReviewDecision {
    Revise,
    Publish,
    Cancel,
}

pub(crate) struct ConflictHandoff<'a> {
    pub(crate) environment: &'a str,
    pub(crate) branch: &'a str,
    pub(crate) unresolved_paths: &'a [String],
    pub(crate) resume_token: &'a str,
    pub(crate) work_area: &'a str,
}

#[derive(Default)]
struct RestackViewState {
    feature_list: ListState,
    scrollable: bool,
    undersized: bool,
    filter: String,
    filtering: bool,
    show_shortcuts: bool,
}

pub(crate) fn draw_loading(terminal: &mut StderrTerminal, message: &str) -> Result<(), CliError> {
    terminal.terminal_mut().draw(|frame| {
        let area = constrain_content_width(frame.area());
        let rows = Layout::vertical([
            Constraint::Length(GRADUATE_ART_HEIGHT + 1),
            Constraint::Min(3),
        ])
        .split(area);
        render_brand_header(frame, rows[0]);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("● ", Palette::pending().add_modifier(Modifier::BOLD)),
                Span::raw(message.to_owned()),
            ]))
            .block(Block::default().borders(Borders::TOP).title(" RESTACK ")),
            rows[1],
        );
    })?;
    Ok(())
}

pub(crate) fn choose_features(
    terminal: &mut StderrTerminal,
    interaction: &mut RestackInteraction,
) -> Result<SelectionDecision, CliError> {
    let mut rejection = None;
    let mut view = RestackViewState {
        feature_list: ListState::default().with_selected(Some(interaction.cursor())),
        scrollable: false,
        undersized: false,
        filter: String::new(),
        filtering: false,
        show_shortcuts: false,
    };
    loop {
        terminal
            .terminal_mut()
            .draw(|frame| render(frame, interaction, None, rejection.as_deref(), &mut view))?;
        let Some(action) = next_selection_action(interaction, &mut view)? else {
            continue;
        };
        if view.undersized && !action_allowed_when_undersized(action) {
            continue;
        }
        match interaction.update(action) {
            RestackInteractionEffect::Preview(selection) => {
                return Ok(SelectionDecision::Preview(selection));
            }
            RestackInteractionEffect::Cancel => return Ok(SelectionDecision::Cancel),
            RestackInteractionEffect::Rejected(error) => {
                rejection = Some(selection_error_message(&error));
            }
            RestackInteractionEffect::None
            | RestackInteractionEffect::Revise
            | RestackInteractionEffect::Publish => rejection = None,
        }
    }
}

pub(crate) fn review_plan(
    terminal: &mut StderrTerminal,
    interaction: &mut RestackInteraction,
    plan: &RestackPlan,
) -> Result<ReviewDecision, CliError> {
    interaction.review_ready();
    let mut view = RestackViewState::default();
    loop {
        terminal
            .terminal_mut()
            .draw(|frame| render(frame, interaction, Some(plan), None, &mut view))?;
        let Some(action) = next_action(interaction.stage())? else {
            continue;
        };
        if view.undersized && !action_allowed_when_undersized(action) {
            continue;
        }
        if matches!(
            action,
            RestackInteractionAction::MoveUp
                | RestackInteractionAction::MoveDown
                | RestackInteractionAction::MovePageUp
                | RestackInteractionAction::MovePageDown
                | RestackInteractionAction::MoveFirst
                | RestackInteractionAction::MoveLast
        ) && !view.scrollable
        {
            continue;
        }
        match interaction.update(action) {
            RestackInteractionEffect::Revise => return Ok(ReviewDecision::Revise),
            RestackInteractionEffect::Publish => return Ok(ReviewDecision::Publish),
            RestackInteractionEffect::Cancel => return Ok(ReviewDecision::Cancel),
            RestackInteractionEffect::None
            | RestackInteractionEffect::Preview(_)
            | RestackInteractionEffect::Rejected(_) => {}
        }
    }
}

pub(crate) fn write_cancelled(environment: &str) -> Result<(), CliError> {
    write_human(cancelled_text(environment))
}

pub(crate) fn write_success(plan: &RestackPlan) -> Result<(), CliError> {
    write_human(success_text(plan))
}

pub(crate) fn write_conflict(handoff: &ConflictHandoff<'_>) -> Result<(), CliError> {
    write_human(conflict_text(handoff))
}

fn write_human(text: String) -> Result<(), CliError> {
    writeln!(io::stderr().lock(), "{text}").map_err(CliError::Io)
}

fn cancelled_text(environment: &str) -> String {
    format!(
        "Restack of {} cancelled; no remote refs changed.",
        escape(environment)
    )
}

fn success_text(plan: &RestackPlan) -> String {
    format!(
        "Restacked {}/{}: {} -> {} (tree {}); {} retained, {} omitted from the environment.",
        escape(&plan.snapshot.remote),
        escape(&plan.snapshot.environment),
        short_oid(&plan.snapshot.environment_tip),
        short_oid(&plan.preview_commit),
        short_oid(&plan.final_tree),
        plan.selection.retained.len(),
        plan.selection.removed.len(),
    )
}

fn conflict_text(handoff: &ConflictHandoff<'_>) -> String {
    let paths = handoff
        .unresolved_paths
        .iter()
        .map(|path| format!("  - {}", escape(path)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "Restack of {} paused on {}.\nUnresolved paths:\n{}\nWork area: {}\n\nResolve this preserved session:\n  1. Edit the unresolved files in the work area.\n  2. Stage every resolution there; leave no unstaged or untracked files.\n  3. Resume with: gd restack {} --resume {}\n\nDo not commit; Graduate creates the canonical merge commit.\nThis resumable session expires after 24 hours of inactivity.",
        escape(handoff.environment),
        escape(handoff.branch),
        paths,
        escape(handoff.work_area),
        escape(handoff.environment),
        handoff.resume_token,
    )
}

fn next_selection_action(
    interaction: &RestackInteraction,
    view: &mut RestackViewState,
) -> Result<Option<RestackInteractionAction>, CliError> {
    let event = event::read()?;
    let Event::Key(key) = event else {
        return Ok(None);
    };
    if key.kind != KeyEventKind::Press {
        return Ok(None);
    }
    Ok(selection_action_for_key(interaction, view, key))
}

fn selection_action_for_key(
    interaction: &RestackInteraction,
    view: &mut RestackViewState,
    key: KeyEvent,
) -> Option<RestackInteractionAction> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(RestackInteractionAction::Cancel);
    }
    if view.filtering {
        match key.code {
            KeyCode::Esc => {
                view.filtering = false;
                view.filter.clear();
            }
            KeyCode::Enter => view.filtering = false,
            KeyCode::Backspace => {
                let _ = view.filter.pop();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                view.filter.push(character);
            }
            _ => return None,
        }
        return first_filtered_index(interaction, &view.filter)
            .map(RestackInteractionAction::MoveTo);
    }
    if key.code == KeyCode::Char('/') {
        view.filtering = true;
        return None;
    }
    if key.code == KeyCode::Char('?') {
        view.show_shortcuts = !view.show_shortcuts;
        return None;
    }
    let visible = filtered_feature_indices(interaction, &view.filter);
    let position = visible
        .iter()
        .position(|index| *index == interaction.cursor())
        .unwrap_or(0);
    let target = match key.code {
        KeyCode::Up | KeyCode::Char('k') => position.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => position.saturating_add(1),
        KeyCode::PageUp => position.saturating_sub(10),
        KeyCode::PageDown => position.saturating_add(10),
        KeyCode::Home => 0,
        KeyCode::End => visible.len().saturating_sub(1),
        _ => return action_for_key(RestackInteractionStage::Selection, key),
    }
    .min(visible.len().saturating_sub(1));
    visible
        .get(target)
        .copied()
        .map(RestackInteractionAction::MoveTo)
}

fn first_filtered_index(interaction: &RestackInteraction, filter: &str) -> Option<usize> {
    filtered_feature_indices(interaction, filter)
        .first()
        .copied()
}

fn filtered_feature_indices(interaction: &RestackInteraction, filter: &str) -> Vec<usize> {
    let filter = filter.to_lowercase();
    interaction
        .snapshot()
        .features
        .iter()
        .enumerate()
        .filter(|(_, feature)| filter.is_empty() || feature.name.to_lowercase().contains(&filter))
        .map(|(index, _)| index)
        .collect()
}

fn next_action(
    stage: RestackInteractionStage,
) -> Result<Option<RestackInteractionAction>, CliError> {
    let event = event::read()?;
    let Event::Key(key) = event else {
        return Ok(None);
    };
    if key.kind != KeyEventKind::Press {
        return Ok(None);
    }
    Ok(action_for_key(stage, key))
}

fn action_for_key(
    stage: RestackInteractionStage,
    key: KeyEvent,
) -> Option<RestackInteractionAction> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(RestackInteractionAction::Cancel);
    }
    match (stage, key.code) {
        (_, KeyCode::Char('q')) => Some(RestackInteractionAction::Cancel),
        (RestackInteractionStage::UnsupportedHistory, KeyCode::Char('r')) => {
            Some(RestackInteractionAction::AcceptInventoryFallback)
        }
        (
            RestackInteractionStage::UnsupportedHistory | RestackInteractionStage::Selection,
            KeyCode::Esc,
        ) => Some(RestackInteractionAction::Cancel),
        (RestackInteractionStage::Selection, KeyCode::Up | KeyCode::Char('k')) => {
            Some(RestackInteractionAction::MoveUp)
        }
        (RestackInteractionStage::Selection, KeyCode::Down | KeyCode::Char('j')) => {
            Some(RestackInteractionAction::MoveDown)
        }
        (RestackInteractionStage::Selection | RestackInteractionStage::Review, KeyCode::PageUp) => {
            Some(RestackInteractionAction::MovePageUp)
        }
        (
            RestackInteractionStage::Selection | RestackInteractionStage::Review,
            KeyCode::PageDown,
        ) => Some(RestackInteractionAction::MovePageDown),
        (RestackInteractionStage::Selection, KeyCode::Home) => {
            Some(RestackInteractionAction::MoveFirst)
        }
        (RestackInteractionStage::Selection, KeyCode::End) => {
            Some(RestackInteractionAction::MoveLast)
        }
        (RestackInteractionStage::Review, KeyCode::Up | KeyCode::Char('k')) => {
            Some(RestackInteractionAction::MoveUp)
        }
        (RestackInteractionStage::Review, KeyCode::Down | KeyCode::Char('j')) => {
            Some(RestackInteractionAction::MoveDown)
        }
        (RestackInteractionStage::Review, KeyCode::Home) => {
            Some(RestackInteractionAction::MoveFirst)
        }
        (RestackInteractionStage::Review, KeyCode::End) => Some(RestackInteractionAction::MoveLast),
        (RestackInteractionStage::Review, KeyCode::Char('d')) => {
            Some(RestackInteractionAction::ToggleDetails)
        }
        (RestackInteractionStage::Selection, KeyCode::Char(' ')) => {
            Some(RestackInteractionAction::Toggle)
        }
        (RestackInteractionStage::Selection, KeyCode::Char('a')) => {
            Some(RestackInteractionAction::KeepAll)
        }
        (RestackInteractionStage::Selection, KeyCode::Char('x')) => {
            Some(RestackInteractionAction::RemoveAll)
        }
        (RestackInteractionStage::Selection | RestackInteractionStage::Review, KeyCode::Enter) => {
            Some(RestackInteractionAction::Continue)
        }
        (RestackInteractionStage::Review | RestackInteractionStage::Confirmation, KeyCode::Esc) => {
            Some(RestackInteractionAction::Back)
        }
        (RestackInteractionStage::Confirmation, KeyCode::Char('y' | 'Y'))
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            Some(RestackInteractionAction::Confirm)
        }
        _ => None,
    }
}

const fn action_allowed_when_undersized(action: RestackInteractionAction) -> bool {
    matches!(
        action,
        RestackInteractionAction::Back | RestackInteractionAction::Cancel
    )
}

fn render(
    frame: &mut Frame<'_>,
    interaction: &RestackInteraction,
    plan: Option<&RestackPlan>,
    rejection: Option<&str>,
    view: &mut RestackViewState,
) {
    let content_width = frame.area().width.min(115);
    let minimum_height = match interaction.stage() {
        RestackInteractionStage::Confirmation => confirmation_minimum_height(plan, content_width),
        RestackInteractionStage::UnsupportedHistory | RestackInteractionStage::Selection => 18,
        RestackInteractionStage::Review => 12,
    };
    if frame.area().width < 56 || frame.area().height < minimum_height {
        view.scrollable = false;
        view.undersized = true;
        render_too_small(frame, frame.area(), minimum_height);
        return;
    }
    view.undersized = false;
    let area = constrain_content_width(frame.area());
    let footer_height = footer_height(interaction.stage(), area.width);
    let available_height = area.height.saturating_sub(4 + footer_height);
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(available_height),
        Constraint::Length(footer_height),
        Constraint::Min(0),
    ])
    .split(area);
    render_workflow_header(frame, rows[0], interaction.stage());
    view.scrollable = match interaction.stage() {
        RestackInteractionStage::UnsupportedHistory => {
            render_unsupported_history(frame, rows[2], interaction)
        }
        RestackInteractionStage::Selection => {
            render_selection(frame, rows[2], interaction, rejection, view)
        }
        RestackInteractionStage::Review => render_review(
            frame,
            rows[2],
            plan,
            interaction.review_scroll(),
            interaction.review_details(),
        ),
        RestackInteractionStage::Confirmation => {
            render_confirmation(frame, rows[2], plan);
            false
        }
    };
    render_footer(frame, rows[3], interaction.stage(), view);
}

fn confirmation_minimum_height(plan: Option<&RestackPlan>, width: u16) -> u16 {
    let text_height = wrapped_text_height(&confirmation_text(plan), width);
    let text_height = text_height.min(usize::from(u16::MAX)) as u16;
    text_height.saturating_add(6).max(16)
}

fn render_too_small(frame: &mut Frame<'_>, area: Rect, minimum_height: u16) {
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                "Terminal too small for a safe restack review.",
                Palette::warning().bold(),
            ),
            Line::styled(
                format!("Resize to at least 56 columns × {minimum_height} rows."),
                Palette::muted(),
            ),
        ])
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_workflow_header(frame: &mut Frame<'_>, area: Rect, stage: RestackInteractionStage) {
    let current = match stage {
        RestackInteractionStage::UnsupportedHistory | RestackInteractionStage::Selection => 0,
        RestackInteractionStage::Review => 1,
        RestackInteractionStage::Confirmation => 2,
    };
    let brand = Line::from(vec![
        Span::styled("GRADUATE", Palette::primary().bold()),
        Span::styled(
            format!("  v{}", env!("CARGO_PKG_VERSION")),
            Palette::muted(),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(brand),
        Rect::new(area.x, area.y, area.width, 1),
    );
    let mut spans = Vec::new();
    for (index, label) in ["Select", "Review", "Publish"].into_iter().enumerate() {
        let style = if index == current {
            Palette::primary().bold()
        } else if index < current {
            Palette::success()
        } else {
            Palette::muted()
        };
        let marker = if index < current {
            "✓".to_owned()
        } else if index == current {
            format!("● {}", index + 1)
        } else {
            format!("○ {}", index + 1)
        };
        spans.push(Span::styled(format!("{marker} {label}"), style));
        if index < 2 {
            spans.push(Span::styled(" › ", Palette::muted()));
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(area.x, area.y.saturating_add(2), area.width, 1),
    );
}

fn render_unsupported_history(
    frame: &mut Frame<'_>,
    area: Rect,
    interaction: &RestackInteraction,
) -> bool {
    let text = unsupported_history_text(interaction);
    let height = wrapped_text_height(&text, area.width);
    frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), area);
    height > usize::from(area.height)
}

const EVIDENCE_LIMIT: usize = 3;

fn unsupported_history_text(interaction: &RestackInteraction) -> Text<'static> {
    let snapshot = interaction.snapshot();
    let environment = escape(&snapshot.environment);
    let main = escape(&snapshot.main);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("HISTORY CANNOT BE READ", Palette::warning().bold()),
            Span::styled(
                format!("  ·  {}/{environment}", escape(&snapshot.remote)),
                Palette::muted(),
            ),
        ]),
        Line::from(""),
    ];
    let Some(reason) = interaction.unsupported_history() else {
        lines.push(Line::raw(
            "The environment history could not be classified.",
        ));
        return Text::from(lines);
    };
    let commit = reason
        .commit
        .as_deref()
        .map_or_else(|| "?".to_owned(), short_oid);
    let feature_parent = reason
        .feature_parent
        .as_deref()
        .map_or_else(|| "?".to_owned(), short_oid);
    let (explanation, evidence_title): (Vec<String>, Option<&str>) = match reason.kind.as_str() {
        "ambiguousFeatureRefs" => (
            vec![
                format!(
                    "Merge {commit} on {environment}'s history brings in {feature_parent}, which {} branches contain.",
                    reason.branches.len()
                ),
                "Restack cannot tell which one it meant.".to_owned(),
            ],
            Some("Branches containing that commit"),
        ),
        "deletedFeatureRef" => (
            vec![
                format!(
                    "Merge {commit} on {environment}'s history brings in {feature_parent},"
                ),
                "but no remote branch contains it any more.".to_owned(),
            ],
            None,
        ),
        "directCommit" => (
            vec![
                format!("Commit {commit} was made directly on {environment}"),
                "instead of being merged from a feature branch.".to_owned(),
            ],
            None,
        ),
        "fastForwardHistory" => (
            vec![
                format!("{environment} was fast-forwarded through {commit};"),
                "there is no merge commit to attribute that work to.".to_owned(),
            ],
            Some("Branches containing that commit"),
        ),
        "octopusMerge" => (
            vec![
                format!(
                    "Merge {commit} has {} parents;",
                    reason.parents.unwrap_or_default()
                ),
                "restack only understands two-parent merges.".to_owned(),
            ],
            None,
        ),
        "missingCommit" => (
            vec![format!(
                "Commit {commit} is missing from the fetched history."
            )],
            None,
        ),
        other => (
            vec![format!("The history proof failed with {}.", escape(other))],
            None,
        ),
    };
    lines.extend(
        explanation
            .into_iter()
            .map(|sentence| Line::styled(sentence, Palette::text())),
    );
    if let Some(title) = evidence_title {
        if !reason.branches.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::styled(title, Palette::muted().bold()));
            lines.extend(
                reason
                    .branches
                    .iter()
                    .take(EVIDENCE_LIMIT)
                    .map(|branch| Line::raw(format!("  •  {}", escape(branch)))),
            );
            if reason.branches.len() > EVIDENCE_LIMIT {
                lines.push(Line::styled(
                    format!(
                        "  … and {} more",
                        reason.branches.len().saturating_sub(EVIDENCE_LIMIT)
                    ),
                    Palette::muted(),
                ));
            }
        }
    }
    let top_level = snapshot.features.len();
    let carried = interaction.carried_features().len();
    let dropped = interaction.orphaned_commit_count();
    lines.extend([
        Line::from(""),
        Line::styled(
            "Rebuilding from inventory instead",
            Palette::primary().bold(),
        ),
        Line::raw(format!(
            "  •  Membership: remote tips in {environment}, not in {main}. You pick."
        )),
        Line::raw("  •  Order: oldest branch tip first. No reused resolutions."),
        Line::raw("  •  Commits on no kept branch are dropped; listed first."),
        Line::from(""),
        Line::from(vec![
            Span::raw(format!(
                "{top_level} top-level {} · {carried} carried · ",
                if top_level == 1 { "branch" } else { "branches" }
            )),
            Span::styled(
                format!(
                    "{dropped} {} dropped",
                    if dropped == 1 { "commit" } else { "commits" }
                ),
                if dropped == 0 {
                    Palette::muted()
                } else {
                    Palette::warning()
                },
            ),
        ]),
    ]);
    Text::from(lines)
}

fn render_selection(
    frame: &mut Frame<'_>,
    area: Rect,
    interaction: &RestackInteraction,
    rejection: Option<&str>,
    view: &mut RestackViewState,
) -> bool {
    let snapshot = interaction.snapshot();
    let retained_count = snapshot
        .features
        .iter()
        .enumerate()
        .filter(|(index, _)| interaction.is_retained(*index))
        .count();
    let removed_count = snapshot.features.len().saturating_sub(retained_count);
    let visible = filtered_feature_indices(interaction, &view.filter);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("SELECT FEATURES", Palette::primary().bold()),
            Span::styled(
                format!(
                    "  {}/{}",
                    escape(&snapshot.remote),
                    escape(&snapshot.environment)
                ),
                Palette::muted(),
            ),
        ])),
        Rect::new(area.x, area.y, area.width, 1),
    );
    let summary_style = if removed_count == 0 {
        Palette::muted()
    } else {
        Palette::warning()
    };
    let filter_summary = if !view.filter.is_empty() && area.width >= 76 {
        format!(" · {}/{} shown", visible.len(), snapshot.features.len())
    } else {
        String::new()
    };
    frame.render_widget(
        Paragraph::new(Line::styled(
            format!("{retained_count} retained · {removed_count} removed{filter_summary}"),
            summary_style,
        ))
        .alignment(Alignment::Right),
        Rect::new(area.x, area.y, area.width, 1),
    );
    let row_width = usize::from(area.width.saturating_sub(4));
    let wide_rows = area.width >= 96;
    let branch_width = if wide_rows {
        row_width.saturating_sub(63).clamp(16, 38)
    } else {
        row_width.saturating_sub(17).clamp(16, 46)
    };
    let items = visible
        .iter()
        .filter_map(|index| {
            snapshot
                .features
                .get(*index)
                .map(|feature| (*index, feature))
        })
        .map(|(index, feature)| {
            let retained = interaction.is_retained(index);
            let locked = !interaction.retained_dependents(index).is_empty();
            let keep = if retained { "✓" } else { "–" };
            let lock = if locked { "◆" } else { " " };
            let keep_style = if retained {
                Palette::success()
            } else {
                Palette::warning()
            };
            let jira = jira_key_from_branch(&feature.name).unwrap_or_else(|| "—".to_owned());
            let history = if feature.historical_merges.is_empty() {
                "—"
            } else {
                "available"
            };
            let history_style = if feature.historical_merges.is_empty() {
                Palette::muted()
            } else {
                Palette::success()
            };
            let branch = pad_text(
                &truncate_text(&escape(&feature.name), branch_width),
                branch_width,
            );
            let branch_style = if retained {
                Palette::text().bold()
            } else {
                Palette::muted()
            };
            let left_meta = format!("    #{} · {}", index + 1, short_oid(&feature.tip));
            let required = if locked { "  ◆ required" } else { "" };
            let right_width =
                "history: ".chars().count() + history.chars().count() + required.chars().count();
            let gap = row_width
                .saturating_sub(left_meta.chars().count() + right_width)
                .max(1);
            let first_line = Line::from(vec![
                Span::styled(keep, keep_style.bold()),
                Span::styled(lock, Palette::warning()),
                Span::raw("  "),
                Span::styled(branch.clone(), branch_style),
                Span::styled(format!(" {jira:>12}"), Palette::muted()),
            ]);
            if wide_rows {
                ListItem::new(Line::from(vec![
                    Span::styled(keep, keep_style.bold()),
                    Span::styled(lock, Palette::warning()),
                    Span::raw(format!("  #{}  ", index + 1)),
                    Span::styled(branch, branch_style),
                    Span::styled(format!("  {}", short_oid(&feature.tip)), Palette::muted()),
                    Span::styled(format!("  {jira:>12}"), Palette::muted()),
                    Span::styled("  history: ", Palette::muted()),
                    Span::styled(history, history_style),
                    Span::styled(required, Palette::warning()),
                ]))
            } else {
                ListItem::new(Text::from(vec![
                    first_line,
                    Line::from(vec![
                        Span::styled(left_meta, Palette::muted()),
                        Span::raw(" ".repeat(gap)),
                        Span::styled("history: ", Palette::muted()),
                        Span::styled(history, history_style),
                        Span::styled(required, Palette::warning()),
                    ]),
                ]))
            }
        })
        .collect::<Vec<_>>();
    let selected = visible
        .iter()
        .position(|index| *index == interaction.cursor());
    view.feature_list.select(selected);
    let selected_context = selection_context(interaction, view.show_shortcuts);
    let context_height = rejection.map_or_else(
        || {
            u16::try_from(
                Line::raw(&selected_context)
                    .width()
                    .div_ceil(usize::from(area.width).max(1)),
            )
            .unwrap_or(2)
            .max(1)
        },
        |message| {
            u16::try_from(
                Line::raw(message)
                    .width()
                    .div_ceil(usize::from(area.width).max(1)),
            )
            .unwrap_or(2)
            .max(1)
        },
    );
    let list_height = area
        .height
        .saturating_sub(2_u16.saturating_add(context_height));
    let list_area = Rect::new(area.x, area.y.saturating_add(2), area.width, list_height);
    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled(
                format!("No branches match “{}”.", escape(&view.filter)),
                Palette::muted(),
            ))
            .alignment(Alignment::Center),
            list_area,
        );
    } else {
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("› ")
                .scroll_padding(1),
            list_area,
            &mut view.feature_list,
        );
    }
    let context_y = list_area.bottom();
    let (context, style) = rejection.map_or_else(
        || (selected_context, Palette::muted()),
        |message| (message.to_owned(), Palette::error()),
    );
    frame.render_widget(
        Paragraph::new(context)
            .style(style)
            .wrap(Wrap { trim: true }),
        Rect::new(
            area.x,
            context_y,
            area.width,
            context_height.min(area.bottom().saturating_sub(context_y)),
        ),
    );
    visible.len().saturating_mul(if wide_rows { 1 } else { 2 }) > usize::from(list_height)
}

fn selection_context(interaction: &RestackInteraction, show_shortcuts: bool) -> String {
    let dependents = interaction.retained_dependents(interaction.cursor());
    if !dependents.is_empty() {
        return format!(
            "◆ Required by {}. Remove those retained features first.",
            dependents
                .iter()
                .map(|dependent| escape(dependent))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if show_shortcuts {
        return "Shortcuts: a keep all · x remove all · Home/End first/last · PgUp/PgDn page · q/Ctrl-C cancel"
            .to_owned();
    }
    "✓ retained · – removed · ◆ retained dependency · history = reusable resolution".to_owned()
}

fn render_review(
    frame: &mut Frame<'_>,
    area: Rect,
    plan: Option<&RestackPlan>,
    scroll: usize,
    show_details: bool,
) -> bool {
    let text = plan.map_or_else(
        || Text::from("The reviewed plan is unavailable."),
        |plan| review_text(plan, show_details),
    );
    let line_count = wrapped_text_height(&text, area.width);
    let max_scroll = line_count.saturating_sub(usize::from(area.height));
    let scroll = if scroll > max_scroll {
        let distance_from_end = usize::MAX.saturating_sub(scroll);
        max_scroll.saturating_sub(distance_from_end)
    } else {
        scroll
    };
    let scroll = scroll.min(usize::from(u16::MAX)) as u16;
    frame.render_widget(
        Paragraph::new(text)
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
    max_scroll > 0
}

fn wrapped_text_height(text: &Text<'_>, width: u16) -> usize {
    Paragraph::new(text.clone())
        .wrap(Wrap { trim: false })
        .line_count(width.max(1))
}

fn review_text(plan: &RestackPlan, show_details: bool) -> Text<'static> {
    let retained = plan.selection.retained.len();
    let removed = plan.selection.removed.len();
    let outcomes = resolution_summary(plan);
    let mut lines = vec![
        Line::from(vec![
            Span::styled("RESTACK REVIEW", Palette::primary().bold()),
            Span::styled(
                format!(
                    "  ·  {}/{}",
                    escape(&plan.snapshot.remote),
                    escape(&plan.snapshot.environment)
                ),
                Palette::muted(),
            ),
        ]),
        Line::from(""),
        Line::styled("Remote rewrite", Palette::warning().bold()),
        Line::from(vec![
            Span::styled("Rewrite         ", Palette::muted()),
            Span::styled(
                short_oid(&plan.snapshot.environment_tip),
                Palette::warning(),
            ),
            Span::styled("  →  ", Palette::muted()),
            Span::styled(short_oid(&plan.preview_commit), Palette::primary().bold()),
        ]),
        Line::from(vec![
            Span::styled("Impact          ", Palette::muted()),
            Span::raw(format!(
                "{retained} retained · {removed} omitted from the rebuilt environment · {outcomes}"
            )),
        ]),
    ];
    if !plan.selection.removed.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            format!(
                "Omitted from {}/{}",
                escape(&plan.snapshot.remote),
                escape(&plan.snapshot.environment)
            ),
            Palette::warning().bold(),
        ));
        lines.push(Line::styled(
            "Their remote branches are not changed or deleted; press Esc to revise.",
            Palette::muted(),
        ));
        lines.extend(plan.selection.removed.iter().map(|branch| {
            Line::from(vec![
                Span::styled(
                    format!("  –  {}", escape(&branch.name)),
                    Palette::warning().bold(),
                ),
                Span::styled(
                    format!("  {}  omitted by your selection", short_oid(&branch.tip)),
                    Palette::warning(),
                ),
            ])
        }));
    }
    lines.extend([
        Line::from(""),
        Line::from(vec![
            Span::styled("Remote guard    ", Palette::muted()),
            Span::raw(format!(
                "publish stops if {}/{} changed since this review",
                escape(&plan.snapshot.remote),
                escape(&plan.snapshot.environment)
            )),
        ]),
    ]);
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Plan details", Palette::muted().bold()),
        Span::styled(
            if show_details {
                "  ·  d hide refs, identities, endpoints, and signing"
            } else {
                "  ·  d show refs, identities, endpoints, and signing"
            },
            Palette::muted(),
        ),
    ]));
    if show_details {
        lines.extend(technical_detail_lines(plan));
    }
    lines.extend([
        Line::from(""),
        Line::styled("Retained merge order", Palette::primary().bold()),
        Line::styled(
            "Selected feature tips are rebuilt in this order.",
            Palette::muted(),
        ),
        Line::styled(
            "   #  BRANCH                            COMMIT   OUTCOME",
            Palette::muted(),
        ),
    ]);
    if plan.selection.retained.is_empty() {
        lines.push(Line::raw(
            "  —  none; the environment becomes the captured base",
        ));
    } else {
        lines.extend(
            plan.selection
                .retained
                .iter()
                .enumerate()
                .map(|(index, branch)| {
                    let (outcome, style) = plan.merges.get(index).map_or(
                        ("unavailable", Palette::warning()),
                        |merge| match merge.resolution {
                            graduate::restack::MergeResolution::Clean => {
                                ("✓ clean", Palette::success())
                            }
                            graduate::restack::MergeResolution::Reused => {
                                ("✓ history reused", Palette::success())
                            }
                            graduate::restack::MergeResolution::Manual => {
                                ("◆ manual", Palette::warning())
                            }
                        },
                    );
                    Line::from(vec![
                        Span::raw(format!(
                            "  {:>2}  {}  {:<7}  ",
                            index + 1,
                            pad_text(&truncate_text(&escape(&branch.name), 32), 32),
                            short_oid(&branch.tip)
                        )),
                        Span::styled(outcome, style),
                    ])
                }),
        );
    }
    if show_details {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            "Exact feature identities",
            Palette::muted().bold(),
        ));
        lines.extend(plan.selection.retained.iter().map(|branch| {
            Line::raw(format!(
                "  retained  {} @ {}",
                escape(&branch.name),
                escape(&branch.tip)
            ))
        }));
        lines.extend(plan.selection.removed.iter().map(|branch| {
            Line::raw(format!(
                "  removed   {} @ {}",
                escape(&branch.name),
                escape(&branch.tip)
            ))
        }));
    }
    Text::from(lines)
}

fn technical_detail_lines(plan: &RestackPlan) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::styled("Base            ", Palette::muted()),
            Span::raw(format!(
                "{} @ {}",
                escape(&plan.snapshot.main_ref),
                escape(&plan.snapshot.main_tip)
            )),
        ]),
        Line::from(vec![
            Span::styled("Environment     ", Palette::muted()),
            Span::raw(format!(
                "{} @ {}",
                escape(&plan.snapshot.environment_ref),
                escape(&plan.snapshot.environment_tip)
            )),
        ]),
        Line::from(vec![
            Span::styled("Preview commit  ", Palette::muted()),
            Span::raw(escape(&plan.preview_commit)),
        ]),
        Line::from(vec![
            Span::styled("Final tree      ", Palette::muted()),
            Span::raw(escape(&plan.final_tree)),
        ]),
        Line::from(vec![
            Span::styled("Author          ", Palette::muted()),
            Span::raw(format!(
                "{} <{}>",
                escape(&plan.author.name),
                escape(&plan.author.email)
            )),
        ]),
        Line::from(vec![
            Span::styled("Fetch endpoint  ", Palette::muted()),
            Span::raw(format!("sha256:{}", plan.remote_endpoints.fetch_sha256)),
        ]),
        Line::from(vec![
            Span::styled("Push endpoint   ", Palette::muted()),
            Span::raw(format!("sha256:{}", plan.remote_endpoints.push_sha256)),
        ]),
        Line::from(vec![
            Span::styled("Publish guard   ", Palette::muted()),
            Span::raw(format!(
                "exact lease; {}/{} must still be at {}",
                escape(&plan.snapshot.remote),
                escape(&plan.snapshot.environment),
                escape(&plan.snapshot.environment_tip)
            )),
        ]),
        Line::from(vec![
            Span::styled("Signing         ", Palette::muted()),
            Span::raw("unsigned canonical merge commits"),
        ]),
        Line::from(vec![
            Span::styled("Dropped markers ", Palette::muted()),
            Span::raw(format!(
                "{} exact phase marker(s)",
                plan.snapshot.dropped_markers.len()
            )),
        ]),
    ]
}

fn resolution_summary(plan: &RestackPlan) -> String {
    let clean = plan
        .merges
        .iter()
        .filter(|merge| merge.resolution == graduate::restack::MergeResolution::Clean)
        .count();
    let reused = plan
        .merges
        .iter()
        .filter(|merge| merge.resolution == graduate::restack::MergeResolution::Reused)
        .count();
    let manual = plan
        .merges
        .iter()
        .filter(|merge| merge.resolution == graduate::restack::MergeResolution::Manual)
        .count();
    let mut parts = Vec::new();
    if clean > 0 {
        parts.push(format!("{clean} clean"));
    }
    if reused > 0 {
        parts.push(format!("{reused} history reused"));
    }
    if manual > 0 {
        parts.push(format!("{manual} manual"));
    }
    let total = clean.saturating_add(reused).saturating_add(manual);
    if parts.is_empty() {
        "0 merges".to_owned()
    } else if parts.len() == 1 {
        format!(
            "{} {}",
            parts.join(""),
            if total == 1 { "merge" } else { "merges" }
        )
    } else {
        format!("{total} merges: {}", parts.join(" · "))
    }
}

fn render_confirmation(frame: &mut Frame<'_>, area: Rect, plan: Option<&RestackPlan>) {
    let text = confirmation_text(plan);
    let text_height = wrapped_text_height(&text, area.width).min(usize::from(u16::MAX)) as u16;
    let panel_height = area.height.min(text_height.saturating_add(1));
    frame.render_widget(
        Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Palette::warning())
                    .title(" PUBLISH REMOTE REWRITE "),
            )
            .wrap(Wrap { trim: false })
            .alignment(Alignment::Left),
        Rect::new(area.x, area.y, area.width, panel_height),
    );
}

fn confirmation_text(plan: Option<&RestackPlan>) -> Text<'static> {
    plan.map_or_else(
        || Text::from("The reviewed plan is unavailable."),
        |plan| {
            const OMITTED_BRANCH_LIMIT: usize = 3;
            let retained = plan.selection.retained.len();
            let removed = plan.selection.removed.len();
            let retained_label = if retained == 1 { "feature" } else { "features" };
            let target = format!(
                "{}/{}",
                escape(&plan.snapshot.remote),
                escape(&plan.snapshot.environment)
            );
            let mut lines = vec![
                Line::from(vec![
                    Span::styled("Current tip     ", Palette::muted()),
                    Span::raw(format!("{target} @ ")),
                    Span::styled(
                        short_oid(&plan.snapshot.environment_tip),
                        Palette::warning(),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Reviewed tip    ", Palette::muted()),
                    Span::raw(format!("{target} @ ")),
                    Span::styled(short_oid(&plan.preview_commit), Palette::primary().bold()),
                ]),
                Line::from(vec![
                    Span::styled("Rewrite scope   ", Palette::muted()),
                    Span::raw(format!(
                        "rebuild {target} from {retained} retained {retained_label} · {removed} omitted · {}",
                        resolution_summary(plan),
                    )),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Remote guard    ", Palette::muted()),
                    Span::raw(format!(
                        "publish stops if {target} changed since review (exact lease)"
                    )),
                ]),
            ];
            if removed > 0 {
                lines.push(Line::from(""));
                lines.push(Line::styled(
                    "Omitted from the reviewed result",
                    Palette::warning().bold(),
                ));
                lines.extend(
                    plan.selection
                        .removed
                        .iter()
                        .take(OMITTED_BRANCH_LIMIT)
                        .map(|branch| {
                            Line::styled(
                                format!(
                                    "  –  {} @ {}",
                                    escape(&branch.name),
                                    short_oid(&branch.tip)
                                ),
                                Palette::warning(),
                            )
                        }),
                );
                if removed > OMITTED_BRANCH_LIMIT {
                    lines.push(Line::styled(
                        format!(
                            "  … and {} more; press Esc to review every omission",
                            removed.saturating_sub(OMITTED_BRANCH_LIMIT)
                        ),
                        Palette::muted(),
                    ));
                }
            }
            lines.extend([
                Line::from(""),
                Line::styled(
                    format!(
                        "This rewrites {target} history; collaborators tracking it must resync after publish."
                    ),
                    Palette::warning(),
                ),
                Line::styled(
                    "Feature branches and local work remain unchanged.",
                    Palette::muted(),
                ),
                Line::from(""),
                Line::styled(
                    format!("Press Ctrl+Y to replace {target} with this reviewed result."),
                    Palette::warning().bold(),
                ),
                Line::styled(
                    "Esc returns to Review; q abandons this plan without changing refs.",
                    Palette::muted(),
                ),
            ]);
            Text::from(lines)
        },
    )
}

fn render_footer(
    frame: &mut Frame<'_>,
    area: Rect,
    stage: RestackInteractionStage,
    view: &RestackViewState,
) {
    if stage == RestackInteractionStage::Selection && view.filtering {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Filter: ", Palette::primary().bold()),
                Span::styled(format!("{}▏", escape(&view.filter)), Palette::text()),
            ])),
            area,
        );
        return;
    }
    let mut controls = Vec::new();
    match stage {
        RestackInteractionStage::UnsupportedHistory => {
            controls.extend(control("r", "Rebuild from inventory", true));
            controls.extend(control("Esc", "Cancel", false));
        }
        RestackInteractionStage::Selection => {
            controls.extend(control("Enter", "Review", true));
            controls.extend(control("Space", "Toggle", false));
            controls.extend(control("/", "Filter", false));
            controls.extend(control(
                "?",
                if view.show_shortcuts {
                    "Hide shortcuts"
                } else {
                    "Shortcuts"
                },
                false,
            ));
            controls.extend(control("Esc", "Cancel", false));
        }
        RestackInteractionStage::Review => {
            controls.extend(control("Enter", "Confirm publish", true));
            if view.scrollable {
                controls.extend(control("PgUp/Dn Home/End", "Scroll", false));
            }
            controls.extend(control("d", "Details", false));
            controls.extend(control("Esc", "Revise", false));
            controls.extend(control("q", "Cancel", false));
        }
        RestackInteractionStage::Confirmation => {
            controls.extend(control("Ctrl+Y", "Publish", true));
            controls.extend(control("Esc", "Review details", false));
            controls.extend(control("q", "Abandon plan", false));
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(controls)).wrap(Wrap { trim: true }),
        area,
    );
}

const fn footer_height(stage: RestackInteractionStage, width: u16) -> u16 {
    match stage {
        RestackInteractionStage::Selection if width < 90 => 2,
        RestackInteractionStage::Selection => 1,
        RestackInteractionStage::Review if width < 72 => 3,
        RestackInteractionStage::Review if width < 96 => 2,
        RestackInteractionStage::UnsupportedHistory
        | RestackInteractionStage::Review
        | RestackInteractionStage::Confirmation => 1,
    }
}

fn control(key: &'static str, label: &'static str, primary: bool) -> Vec<Span<'static>> {
    let key_style = if primary {
        Palette::primary().bold()
    } else {
        Palette::text().bold()
    };
    vec![
        Span::styled(key, key_style),
        Span::styled(format!(" {label}  "), Palette::muted()),
    ]
}

fn truncate_text(value: &str, width: usize) -> String {
    if Line::raw(value).width() <= width {
        return value.to_owned();
    }
    let visible = width.saturating_sub(1);
    let mut truncated = String::new();
    for character in value.chars() {
        let mut candidate = truncated.clone();
        candidate.push(character);
        if Line::raw(&candidate).width() > visible {
            break;
        }
        truncated.push(character);
    }
    format!("{truncated}…")
}

fn pad_text(value: &str, width: usize) -> String {
    let padding = width.saturating_sub(Line::raw(value).width());
    format!("{value}{}", " ".repeat(padding))
}

fn selection_error_message(error: &SelectionError) -> String {
    match error {
        SelectionError::RetainedDependency { branch, dependents } => format!(
            "Cannot remove {} while retained by {}.",
            escape(branch),
            dependents
                .iter()
                .map(|dependent| escape(dependent))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        SelectionError::Duplicate { branch }
        | SelectionError::Graduated { branch }
        | SelectionError::IndirectOnly { branch }
        | SelectionError::Unknown { branch } => {
            format!("Cannot remove {} from this inventory.", escape(branch))
        }
    }
}

fn short_oid(oid: &str) -> String {
    escape(&oid.chars().take(7).collect::<String>())
}

#[cfg(test)]
mod tests {
    use graduate::restack::{
        build_inventory_snapshot, build_plan, AttributedCommit, BranchIdentity, ExplicitFeature,
        FeatureRef, GraphCommit, HistoricalMerge, InventoryError, InventoryMode, MergeOutcome,
        MergeResolution, Reconstruction, RemoteEndpointIdentity, RestackAuthor, RestackGraph,
        RestackSnapshot, UnsupportedHistory,
    };
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;

    #[test]
    fn checklist_renders_order_identity_jira_key_and_rerere_availability(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let interaction = RestackInteraction::new(snapshot());
        let rendered = rendered(&interaction, None, None)?;

        assert!(rendered.contains("SELECT FEATURES"));
        assert!(rendered.contains("● 1 Select › ○ 2 Review › ○ 3 Publish"));
        assert!(!rendered.contains("Filter:"));
        assert!(rendered.contains("feature/PROJ-12-one"));
        assert!(rendered.contains("aaaaaaa"));
        assert!(rendered.contains("PROJ-12"));
        assert!(rendered.contains("available"));
        assert!(rendered.contains("feature/two"));
        assert!(rendered.contains("2 retained · 0 removed"));
        assert!(rendered.contains("◆ Required by feature/two"));
        let footer_row = rendered
            .lines()
            .position(|line| line.contains("Enter Review"))
            .ok_or("selection footer was not rendered")?;
        assert!(footer_row >= 36);
        Ok(())
    }

    #[test]
    fn dependency_rejection_names_the_retained_dependent() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut interaction = RestackInteraction::new(snapshot());
        let rejection = match interaction.update(RestackInteractionAction::Toggle) {
            RestackInteractionEffect::Rejected(error) => selection_error_message(&error),
            _ => String::new(),
        };
        let rendered = rendered(&interaction, None, Some(&rejection))?;

        assert!(rendered.contains("Cannot remove feature/PROJ-12-one"));
        assert!(rendered.contains("feature/two"));
        assert!(interaction.is_retained(0));
        let compact = rendered_at(&interaction, None, Some(&rejection), 56, 24)?;
        assert!(compact.contains("feature/two"));
        Ok(())
    }

    #[test]
    fn checklist_updates_the_impact_summary_after_a_toggle(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut snapshot = snapshot();
        snapshot.attributed_commits.clear();
        let mut interaction = RestackInteraction::new(snapshot);
        let _ = interaction.update(RestackInteractionAction::Toggle);

        let rendered = rendered(&interaction, None, None)?;

        assert!(rendered.contains("1 retained · 1 removed"));
        assert!(rendered.contains("Space Toggle"));
        Ok(())
    }

    #[test]
    fn checklist_preserves_list_viewport_while_the_cursor_moves(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut snapshot = snapshot();
        snapshot.features = (1..=10)
            .map(|index| ExplicitFeature {
                name: format!("feature/{index}"),
                tip: format!("{index:040}"),
                historical_merges: Vec::new(),
            })
            .collect();
        snapshot.attributed_commits.clear();
        let mut interaction = RestackInteraction::new(snapshot);
        for _ in 0..9 {
            let _ = interaction.update(RestackInteractionAction::MoveDown);
        }
        let mut view = RestackViewState::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 18))?;

        terminal.draw(|frame| render(frame, &interaction, None, None, &mut view))?;
        let scrolled_offset = view.feature_list.offset();
        assert!(scrolled_offset > 0);
        let _ = interaction.update(RestackInteractionAction::MoveUp);
        terminal.draw(|frame| render(frame, &interaction, None, None, &mut view))?;

        assert_eq!(view.feature_list.selected(), Some(interaction.cursor()));
        assert_eq!(view.feature_list.offset(), scrolled_offset);
        Ok(())
    }

    #[test]
    fn compact_checklist_reflows_issue_and_history_instead_of_hiding_them(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let interaction = RestackInteraction::new(snapshot());
        let rendered = rendered_at(&interaction, None, None, 80, 24)?;

        assert!(!rendered.contains("Filter:"));
        assert!(rendered.contains("PROJ-12"));
        assert!(rendered.contains("available"));
        assert!(rendered.contains("/ Filter"));
        assert!(rendered.contains("? Shortcuts"));
        assert!(rendered.contains("◆ Required by feature/two"));
        assert!(!rendered.contains("a keep all"));
        Ok(())
    }

    #[test]
    fn checklist_reveals_secondary_shortcuts_on_request() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut snapshot = snapshot();
        snapshot.attributed_commits.clear();
        let mut interaction = RestackInteraction::new(snapshot);
        let _ = interaction.update(RestackInteractionAction::MoveDown);
        let mut view = RestackViewState::default();

        assert_eq!(
            selection_action_for_key(
                &interaction,
                &mut view,
                KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE),
            ),
            None
        );
        let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
        terminal.draw(|frame| render(frame, &interaction, None, None, &mut view))?;
        let rendered = terminal.backend().to_string();

        assert!(rendered.contains("a keep all · x remove all"));
        assert!(rendered.contains("Home/End first/last"));
        assert!(rendered.contains("? Hide shortcuts"));
        Ok(())
    }

    #[test]
    fn wide_checklist_collapses_each_feature_to_one_evidence_row(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let interaction = RestackInteraction::new(snapshot());
        let rendered = rendered_at(&interaction, None, None, 100, 24)?;
        let feature_line = rendered
            .lines()
            .find(|line| line.contains("feature/PROJ-12-one"))
            .ok_or("wide feature row was not rendered")?;

        assert!(feature_line.contains("aaaaaaa"));
        assert!(feature_line.contains("PROJ-12"));
        assert!(feature_line.contains("history: available"));
        Ok(())
    }

    #[test]
    fn checklist_filter_narrows_rows_and_keeps_selection_on_a_visible_branch(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut snapshot = snapshot();
        snapshot.attributed_commits.clear();
        let mut interaction = RestackInteraction::new(snapshot);
        let mut view = RestackViewState::default();
        assert_eq!(
            selection_action_for_key(
                &interaction,
                &mut view,
                KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
            ),
            None
        );
        for character in ['t', 'w', 'o'] {
            if let Some(action) = selection_action_for_key(
                &interaction,
                &mut view,
                KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE),
            ) {
                let _ = interaction.update(action);
            }
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
        terminal.draw(|frame| render(frame, &interaction, None, None, &mut view))?;
        let rendered = terminal.backend().to_string();

        assert_eq!(view.filter, "two");
        assert_eq!(interaction.cursor(), 1);
        assert!(rendered.contains("feature/two"));
        assert!(!rendered.contains("feature/PROJ-12-one"));
        assert!(rendered.contains("1/2"));
        assert!(rendered.contains("Filter: two▏"));
        assert!(!rendered.contains("Enter Review"));
        Ok(())
    }

    #[test]
    fn checklist_explains_an_empty_filter_result() -> Result<(), Box<dyn std::error::Error>> {
        let interaction = RestackInteraction::new(snapshot());
        let mut view = RestackViewState {
            filter: "missing".to_owned(),
            ..RestackViewState::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 24))?;

        terminal.draw(|frame| render(frame, &interaction, None, None, &mut view))?;
        let rendered = terminal.backend().to_string();

        assert!(rendered.contains("No branches match “missing”"));
        assert!(rendered.contains("0/2"));
        Ok(())
    }

    #[test]
    fn undersized_terminal_replaces_the_workflow_with_resize_guidance(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let interaction = RestackInteraction::new(snapshot());
        let rendered = rendered_at(&interaction, None, None, 55, 11)?;

        assert!(rendered.contains("Terminal too small for a safe restack review"));
        assert!(rendered.contains("56 columns × 18 rows"));
        assert!(!rendered.contains("SELECT FEATURES"));
        Ok(())
    }

    #[test]
    fn undersized_review_allows_escape_but_blocks_progression_and_publication(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let plan = plan()?;
        let mut interaction = RestackInteraction::new(snapshot());
        interaction.review_ready();
        let mut view = RestackViewState::default();
        let mut terminal = Terminal::new(TestBackend::new(55, 11))?;

        terminal.draw(|frame| render(frame, &interaction, Some(&plan), None, &mut view))?;

        assert!(view.undersized);
        assert!(!action_allowed_when_undersized(
            RestackInteractionAction::Continue
        ));
        assert!(!action_allowed_when_undersized(
            RestackInteractionAction::Confirm
        ));
        assert!(action_allowed_when_undersized(
            RestackInteractionAction::Back
        ));
        assert!(action_allowed_when_undersized(
            RestackInteractionAction::Cancel
        ));
        Ok(())
    }

    #[test]
    fn wrapped_height_uses_paragraph_word_boundaries() {
        let text = Text::from("aaaa aaaa aaaa");

        assert_eq!(wrapped_text_height(&text, 8), 3);
    }

    #[test]
    fn review_and_confirmation_show_the_exact_rewrite_and_safety_effects(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let plan = plan()?;
        let mut interaction = RestackInteraction::new(snapshot());
        interaction.review_ready();
        let review = rendered(&interaction, Some(&plan), None)?;

        assert!(review.contains("RESTACK REVIEW"));
        assert!(review.contains("Remote rewrite"));
        assert!(review.contains("environ"));
        assert!(review.contains("preview"));
        assert!(
            review.contains("1 retained · 1 omitted from the rebuilt environment · 1 clean merge")
        );
        assert!(review.contains("publish stops if origin/qa changed since this review"));
        assert!(!review.contains("Unchanged"));
        assert!(!review.contains("Target          origin/qa"));
        assert!(!review.contains("Commit signing"));
        assert!(!review.contains("Starts from"));
        assert!(!review.contains("Builds"));
        assert!(review.contains("Retained merge order"));
        assert!(review.contains("Selected feature tips are rebuilt in this order"));
        assert!(review.contains("✓ clean"));
        let retained_header = review
            .lines()
            .find(|line| line.contains("BRANCH") && line.contains("OUTCOME"))
            .ok_or("retained header was not rendered")?;
        let retained_row = review
            .lines()
            .find(|line| line.contains("feature/PROJ-12-one"))
            .ok_or("retained row was not rendered")?;
        assert_eq!(retained_header.find('#'), retained_row.find('1'));
        assert_eq!(
            retained_header.find("BRANCH"),
            retained_row.find("feature/PROJ-12-one")
        );
        assert!(review.contains("Omitted from origin/qa"));
        assert!(review.contains("remote branches are not changed or deleted; press Esc to revise"));
        assert!(review.contains("omitted by your selection"));
        assert!(review.contains("Plan details  ·  d show refs, identities, endpoints, and signing"));
        assert!(!review.contains("sha256:ffffffff"));
        assert!(review.contains("Enter Confirm publish"));
        assert!(!review.contains("↑/↓ Scroll"));
        let footer_row = review
            .lines()
            .position(|line| line.contains("Enter Confirm publish"))
            .ok_or("review footer was not rendered")?;
        assert!(footer_row >= 36);

        let _ = interaction.update(RestackInteractionAction::ToggleDetails);
        let details = rendered(&interaction, Some(&plan), None)?;
        assert!(details.contains("refs/remotes/origin/main @ main-tip"));
        assert!(details.contains("Pat <pat@example.com>"));
        assert!(details.contains("sha256:ffffffff"));
        assert!(details.contains("unsigned canonical merge commits"));
        assert!(details.contains("0 exact phase marker(s)"));
        for _ in 0..20 {
            let _ = interaction.update(RestackInteractionAction::MoveDown);
        }
        let identities = rendered(&interaction, Some(&plan), None)?;
        assert!(identities.contains("Exact feature identities"));
        assert!(identities.contains("retained  feature/PROJ-12-one @ aaaaaaaaaa"));
        assert!(identities.contains("removed   feature/two @ bbbbbbbbbb"));

        let _ = interaction.update(RestackInteractionAction::Continue);
        let confirmation = rendered(&interaction, Some(&plan), None)?;
        assert!(confirmation.contains("publish stops if origin/qa changed since review"));
        assert!(confirmation.contains("(exact lease)"));
        assert!(confirmation.contains("Current tip     origin/qa @ environ"));
        assert!(confirmation.contains("Reviewed tip    origin/qa @ preview"));
        assert!(confirmation.contains("rebuild origin/qa from 1 retained feature"));
        assert!(confirmation.contains("1 omitted · 1 clean merge"));
        assert!(confirmation.contains("Omitted from the reviewed result"));
        assert!(confirmation.contains("feature/two @ bbbbbbb"));
        assert!(confirmation.contains("collaborators tracking it must resync after publish"));
        assert!(confirmation.contains("Feature branches and local work remain unchanged"));
        assert!(confirmation.contains("Press Ctrl+Y to replace origin/qa"));
        assert!(confirmation.contains("q abandons this plan without changing refs"));
        assert!(confirmation.contains("Ctrl+Y Publish"));
        assert!(confirmation.contains("Esc Review details"));
        assert!(confirmation.contains("q Abandon plan"));
        assert!(!confirmation.contains("unsigned"));
        let compact_confirmation = rendered_at(&interaction, Some(&plan), None, 80, 24)?;
        assert!(compact_confirmation.contains("rebuild origin/qa from 1 retained feature"));
        assert!(compact_confirmation.contains("publish stops if origin/qa changed since review"));
        assert!(compact_confirmation.contains("collaborators tracking it must resync"));
        assert!(compact_confirmation.contains("Press Ctrl+Y to replace origin/qa"));
        assert!(compact_confirmation.contains("Ctrl+Y Publish"));
        Ok(())
    }

    #[test]
    fn short_confirmation_requires_enough_height_for_the_publish_warning(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let plan = plan()?;
        let mut interaction = RestackInteraction::new(snapshot());
        interaction.review_ready();
        let _ = interaction.update(RestackInteractionAction::Continue);
        let minimum_height = confirmation_minimum_height(Some(&plan), 56);

        let rendered = rendered_at(
            &interaction,
            Some(&plan),
            None,
            56,
            minimum_height.saturating_sub(1),
        )?;

        assert!(rendered.contains("Terminal too small for a safe restack review"));
        assert!(rendered.contains(&format!("56 columns × {minimum_height} rows")));
        assert!(!rendered.contains("PUBLISH REMOTE REWRITE"));

        let boundary = rendered_at(&interaction, Some(&plan), None, 56, minimum_height)?;
        assert!(boundary.contains("Feature branches and local work"));
        assert!(boundary.contains("Press Ctrl+Y to replace origin/qa"));
        Ok(())
    }

    #[test]
    fn confirmation_bounds_large_omission_lists_and_points_back_to_review(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut plan = plan()?;
        plan.selection.removed = (1..=5)
            .map(|index| BranchIdentity {
                name: format!("feature/removed-{index}"),
                tip: format!("{index:040}"),
            })
            .collect();
        let mut interaction = RestackInteraction::new(snapshot());
        interaction.review_ready();
        let _ = interaction.update(RestackInteractionAction::Continue);

        let confirmation = rendered_at(&interaction, Some(&plan), None, 115, 38)?;

        assert!(confirmation.contains("5 omitted"));
        assert!(confirmation.contains("feature/removed-1"));
        assert!(confirmation.contains("feature/removed-3"));
        assert!(!confirmation.contains("feature/removed-4"));
        assert!(confirmation.contains("and 2 more; press Esc to review every omission"));
        Ok(())
    }

    #[test]
    fn checklist_truncates_wide_unicode_by_terminal_columns() {
        let value = "feature/界界界界界界界界界界";
        let truncated = truncate_text(value, 16);
        let padded = pad_text(&truncated, 16);

        assert!(Line::raw(&truncated).width() <= 16);
        assert_eq!(Line::raw(&padded).width(), 16);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn compact_review_keeps_every_action_visible() -> Result<(), Box<dyn std::error::Error>> {
        let plan = plan()?;
        let mut interaction = RestackInteraction::new(snapshot());
        interaction.review_ready();
        let _ = interaction.update(RestackInteractionAction::ToggleDetails);
        let mut terminal = Terminal::new(TestBackend::new(60, 24))?;
        let mut view = RestackViewState::default();

        terminal.draw(|frame| render(frame, &interaction, Some(&plan), None, &mut view))?;
        let rendered = terminal.backend().to_string();

        assert!(view.scrollable);
        assert!(rendered.contains("Omitted from origin/qa"));
        assert!(rendered.contains("feature/two"));
        assert!(rendered.contains("to revise."));
        assert!(rendered.contains("PgUp/Dn Home/End Scroll"));
        assert!(rendered.contains("Enter Confirm publish"));
        assert!(rendered.contains("Esc Revise"));
        assert!(rendered.contains("d Details"));
        assert!(rendered.contains("q Cancel"));
        Ok(())
    }

    #[test]
    fn review_navigates_hundreds_of_retained_features_and_keeps_details_near_the_top(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut plan = plan()?;
        plan.selection.removed.clear();
        plan.selection.retained = (1..=250)
            .map(|index| BranchIdentity {
                name: format!("feature/{index:03}"),
                tip: format!("{index:040}"),
            })
            .collect();
        plan.merges = plan
            .selection
            .retained
            .iter()
            .map(|branch| MergeOutcome {
                branch: branch.name.clone(),
                tip: branch.tip.clone(),
                commit: format!("merge-{}", branch.name),
                tree: format!("tree-{}", branch.name),
                resolution: MergeResolution::Clean,
            })
            .collect();
        let mut interaction = RestackInteraction::new(snapshot());
        interaction.review_ready();
        let mut view = RestackViewState::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 24))?;

        terminal.draw(|frame| render(frame, &interaction, Some(&plan), None, &mut view))?;
        let first_page = terminal.backend().to_string();
        assert!(view.scrollable);
        assert!(first_page.contains("250 retained"));
        assert!(first_page.contains("Plan details"));
        assert!(first_page.contains("Home/End Scroll"));

        let _ = interaction.update(RestackInteractionAction::MoveLast);
        terminal.draw(|frame| render(frame, &interaction, Some(&plan), None, &mut view))?;
        let last_page = terminal.backend().to_string();
        assert!(last_page.contains("feature/250"));

        let _ = interaction.update(RestackInteractionAction::MoveUp);
        terminal.draw(|frame| render(frame, &interaction, Some(&plan), None, &mut view))?;
        let above_last_page = terminal.backend().to_string();
        assert!(!above_last_page.contains("feature/250"));

        let _ = interaction.update(RestackInteractionAction::MoveFirst);
        terminal.draw(|frame| render(frame, &interaction, Some(&plan), None, &mut view))?;
        let returned = terminal.backend().to_string();
        assert!(returned.contains("RESTACK REVIEW"));
        assert!(returned.contains("Plan details"));
        Ok(())
    }

    #[test]
    fn explicit_confirmation_and_cancel_keys_have_distinct_actions() {
        assert_eq!(
            action_for_key(
                RestackInteractionStage::Confirmation,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            ),
            None
        );
        assert_eq!(
            action_for_key(
                RestackInteractionStage::Confirmation,
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)
            ),
            None
        );
        assert_eq!(
            action_for_key(
                RestackInteractionStage::Confirmation,
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)
            ),
            Some(RestackInteractionAction::Confirm)
        );
        assert_eq!(
            action_for_key(
                RestackInteractionStage::Confirmation,
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)
            ),
            Some(RestackInteractionAction::Cancel)
        );
        assert_eq!(
            action_for_key(
                RestackInteractionStage::Confirmation,
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
            ),
            Some(RestackInteractionAction::Back)
        );
        assert_eq!(
            action_for_key(
                RestackInteractionStage::Confirmation,
                KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)
            ),
            None
        );
        assert_eq!(
            action_for_key(
                RestackInteractionStage::Review,
                KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)
            ),
            Some(RestackInteractionAction::ToggleDetails)
        );
        assert_eq!(
            action_for_key(
                RestackInteractionStage::Review,
                KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)
            ),
            Some(RestackInteractionAction::MoveFirst)
        );
        assert_eq!(
            action_for_key(
                RestackInteractionStage::Review,
                KeyEvent::new(KeyCode::End, KeyModifiers::NONE)
            ),
            Some(RestackInteractionAction::MoveLast)
        );
        assert_eq!(
            action_for_key(
                RestackInteractionStage::Selection,
                KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)
            ),
            Some(RestackInteractionAction::MovePageDown)
        );
        assert_eq!(
            action_for_key(
                RestackInteractionStage::Selection,
                KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)
            ),
            Some(RestackInteractionAction::KeepAll)
        );
    }

    #[test]
    fn ordinary_completion_and_conflict_handoff_are_redacted_and_actionable(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let plan = plan()?;
        let success = success_text(&plan);
        assert!(success.contains("Restacked origin/qa"));
        assert!(success.contains("1 retained, 1 omitted from the environment"));
        assert!(!success.contains("1 removed"));

        let paths = vec!["src/file\nname.rs".to_owned()];
        let handoff = conflict_text(&ConflictHandoff {
            environment: "qa",
            branch: "feature/PROJ-12-one",
            unresolved_paths: &paths,
            resume_token: "v1.safe.token",
            work_area: "/tmp/work\narea",
        });
        assert!(handoff.contains("src/file\\nname.rs"));
        assert!(handoff.contains("Work area: /tmp/work\\narea"));
        assert!(handoff.contains("1. Edit the unresolved files in the work area"));
        assert!(handoff.contains("2. Stage every resolution there"));
        assert!(handoff.contains("3. Resume with: gd restack qa --resume v1.safe.token"));
        assert!(handoff.contains("Do not commit; Graduate creates the canonical merge commit"));
        assert!(handoff.contains("expires after 24 hours of inactivity"));
        Ok(())
    }

    fn rendered(
        interaction: &RestackInteraction,
        plan: Option<&RestackPlan>,
        rejection: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        rendered_at(interaction, plan, rejection, 115, 38)
    }

    fn rendered_at(
        interaction: &RestackInteraction,
        plan: Option<&RestackPlan>,
        rejection: Option<&str>,
        width: u16,
        height: u16,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut terminal = Terminal::new(TestBackend::new(width, height))?;
        let mut view = RestackViewState::default();
        terminal.draw(|frame| render(frame, interaction, plan, rejection, &mut view))?;
        Ok(terminal.backend().to_string())
    }

    #[test]
    fn unsupported_history_screen_explains_every_reason_and_fits_the_minimum_size(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let reasons = [
            (
                InventoryError::AmbiguousFeatureRefs {
                    merge_commit: "886faef4b24230540b9a5d8ae057a233a7dd0126".to_owned(),
                    feature_parent: "0bbff862c139d704a3c6431d9fa7f16c55c1aa5a".to_owned(),
                    branches: (1..=17).map(|n| format!("HPM-{n}")).collect(),
                },
                vec![
                    "brings in 0bbff86, which 17",
                    "cannot tell which one it meant",
                    "HPM-3",
                    "and 14 more",
                ],
            ),
            (
                InventoryError::DeletedFeatureRef {
                    merge_commit: "a".repeat(40),
                    feature_parent: "b".repeat(40),
                },
                vec!["no remote branch contains it any more"],
            ),
            (
                InventoryError::DirectCommit {
                    commit: "c".repeat(40),
                },
                vec!["was made directly on qa"],
            ),
            (
                InventoryError::FastForwardHistory {
                    commit: "d".repeat(40),
                    branches: vec!["feature/ff".to_owned()],
                },
                vec!["fast-forwarded through ddddddd", "feature/ff"],
            ),
            (
                InventoryError::OctopusMerge {
                    merge_commit: "e".repeat(40),
                    parents: 3,
                },
                vec!["has 3 parents"],
            ),
            (
                InventoryError::MissingCommit {
                    commit: "f".repeat(40),
                },
                vec!["is missing from the fetched history"],
            ),
        ];
        for (error, expectations) in reasons {
            let interaction = RestackInteraction::from_inventory(inventory_snapshot(error.into()));
            for (width, height) in [(60, 24), (100, 30)] {
                let rendered = rendered_at(&interaction, None, None, width, height)?;
                assert!(
                    rendered.contains("HISTORY CANNOT BE READ"),
                    "{width}x{height}: {rendered}"
                );
                for expectation in &expectations {
                    assert!(
                        rendered.contains(expectation),
                        "{width}x{height} missing {expectation:?}:\n{rendered}"
                    );
                }
                assert!(rendered.contains("Rebuilding from inventory instead"));
                assert!(rendered.contains("Membership: remote tips in qa, not in main. You pick."));
                assert!(rendered.contains("No reused resolutions"));
                assert!(rendered.contains("dropped; listed first"));
                assert!(rendered.contains("1 top-level branch · 1 carried · 1 commit dropped"));
                assert!(rendered.contains("Rebuild from inventory"));
                assert!(rendered.contains("Cancel"));
                assert!(!rendered.contains("SELECT FEATURES"));
            }
        }
        Ok(())
    }

    #[test]
    fn unsupported_history_keys_accept_the_fallback_or_cancel() {
        let stage = RestackInteractionStage::UnsupportedHistory;
        assert_eq!(
            action_for_key(stage, KeyEvent::from(KeyCode::Char('r'))),
            Some(RestackInteractionAction::AcceptInventoryFallback)
        );
        assert_eq!(
            action_for_key(stage, KeyEvent::from(KeyCode::Esc)),
            Some(RestackInteractionAction::Cancel)
        );
        assert_eq!(
            action_for_key(stage, KeyEvent::from(KeyCode::Char('q'))),
            Some(RestackInteractionAction::Cancel)
        );
        assert_eq!(action_for_key(stage, KeyEvent::from(KeyCode::Enter)), None);
        assert_eq!(
            action_for_key(stage, KeyEvent::from(KeyCode::Char(' '))),
            None
        );
        assert_eq!(
            action_for_key(
                RestackInteractionStage::Selection,
                KeyEvent::from(KeyCode::Char('r'))
            ),
            None
        );
    }

    /// Reachability snapshot: feature/b carries feature/a; `stray` is dropped.
    fn inventory_snapshot(reason: UnsupportedHistory) -> RestackSnapshot {
        let ids = |values: &[&str]| -> std::collections::BTreeSet<String> {
            values.iter().map(ToString::to_string).collect()
        };
        let mut commits = std::collections::BTreeMap::new();
        for (id, parents) in [
            ("base", vec![]),
            ("a", vec!["base"]),
            ("b", vec!["a"]),
            ("stray", vec!["b"]),
        ] {
            commits.insert(
                id.to_owned(),
                GraphCommit {
                    id: id.to_owned(),
                    tree: format!("tree-{id}"),
                    parents: parents.into_iter().map(str::to_owned).collect(),
                    message: id.to_owned(),
                },
            );
        }
        let graph = RestackGraph {
            remote: "origin".to_owned(),
            environment: "qa".to_owned(),
            environment_ref: "refs/remotes/origin/qa".to_owned(),
            environment_tip: "stray".to_owned(),
            main: "main".to_owned(),
            main_ref: "refs/remotes/origin/main".to_owned(),
            main_tip: "base".to_owned(),
            environment_ancestors: ids(&["base", "a", "b", "stray"]),
            main_ancestors: ids(&["base"]),
            feature_refs: vec![
                FeatureRef {
                    name: "feature/PROJ-12-one".to_owned(),
                    tip: "a".to_owned(),
                    ancestors: ids(&["a"]),
                },
                FeatureRef {
                    name: "feature/two".to_owned(),
                    tip: "b".to_owned(),
                    ancestors: ids(&["a", "b"]),
                },
            ],
            commits,
        };
        build_inventory_snapshot(&graph, reason, &std::collections::BTreeMap::new())
    }

    fn plan() -> Result<RestackPlan, Box<dyn std::error::Error>> {
        Ok(build_plan(
            snapshot(),
            RemoteEndpointIdentity {
                fetch_sha256: "f".repeat(64),
                push_sha256: "p".repeat(64),
            },
            RestackAuthor {
                name: "Pat".to_owned(),
                email: "pat@example.com".to_owned(),
            },
            RestackSelection {
                retained: vec![BranchIdentity {
                    name: "feature/PROJ-12-one".to_owned(),
                    tip: "a".repeat(40),
                }],
                removed: vec![BranchIdentity {
                    name: "feature/two".to_owned(),
                    tip: "b".repeat(40),
                }],
            },
            Reconstruction {
                merges: vec![MergeOutcome {
                    branch: "feature/PROJ-12-one".to_owned(),
                    tip: "a".repeat(40),
                    commit: "preview".to_owned(),
                    tree: "tree-tip".to_owned(),
                    resolution: MergeResolution::Clean,
                }],
                final_tree: "tree-tip".to_owned(),
                preview_commit: "preview".to_owned(),
            },
            Vec::new(),
        )?)
    }

    fn snapshot() -> RestackSnapshot {
        RestackSnapshot {
            remote: "origin".to_owned(),
            environment: "qa".to_owned(),
            environment_ref: "refs/remotes/origin/qa".to_owned(),
            environment_tip: "environment-tip".to_owned(),
            main: "main".to_owned(),
            main_ref: "refs/remotes/origin/main".to_owned(),
            main_tip: "main-tip".to_owned(),
            features: vec![
                ExplicitFeature {
                    name: "feature/PROJ-12-one".to_owned(),
                    tip: "a".repeat(40),
                    historical_merges: vec![HistoricalMerge {
                        commit: "merge".to_owned(),
                        first_parent: "parent".to_owned(),
                        feature_parent: "feature".to_owned(),
                        tree: "tree".to_owned(),
                    }],
                },
                ExplicitFeature {
                    name: "feature/two".to_owned(),
                    tip: "b".repeat(40),
                    historical_merges: Vec::new(),
                },
            ],
            graduated_features: Vec::new(),
            indirect_features: Vec::new(),
            dropped_markers: Vec::new(),
            attributed_commits: vec![AttributedCommit {
                commit: "shared".to_owned(),
                branches: vec!["feature/PROJ-12-one".to_owned(), "feature/two".to_owned()],
            }],
            inventory_mode: InventoryMode::History,
            unsupported_history: None,
            carried_features: Vec::new(),
            unattributed_commits: Vec::new(),
        }
    }
}
