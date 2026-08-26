//! Terminal selection, review, and conflict handoff for interactive restacks.

use std::io::{self, Write};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use graduate::promotion::jira_key_from_branch;
use graduate::restack::{
    RestackInteraction, RestackInteractionAction, RestackInteractionEffect,
    RestackInteractionStage, RestackPlan, RestackSelection, SelectionError,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
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
    loop {
        terminal
            .terminal_mut()
            .draw(|frame| render(frame, interaction, None, rejection.as_deref()))?;
        let Some(action) = next_action(interaction.stage())? else {
            continue;
        };
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
    loop {
        terminal
            .terminal_mut()
            .draw(|frame| render(frame, interaction, Some(plan), None))?;
        let Some(action) = next_action(interaction.stage())? else {
            continue;
        };
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
        "Restacked {}/{}: {} -> {} (tree {}).",
        escape(&plan.snapshot.remote),
        escape(&plan.snapshot.environment),
        short_oid(&plan.snapshot.environment_tip),
        short_oid(&plan.preview_commit),
        short_oid(&plan.final_tree),
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
        "Restack of {} paused on {}.\nUnresolved paths:\n{}\nWork area: {}\nResume with: gd restack {} --resume {}",
        escape(handoff.environment),
        escape(handoff.branch),
        paths,
        escape(handoff.work_area),
        escape(handoff.environment),
        handoff.resume_token,
    )
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
        (RestackInteractionStage::Selection, KeyCode::Esc) => {
            Some(RestackInteractionAction::Cancel)
        }
        (RestackInteractionStage::Selection, KeyCode::Up | KeyCode::Char('k')) => {
            Some(RestackInteractionAction::MoveUp)
        }
        (RestackInteractionStage::Selection, KeyCode::Down | KeyCode::Char('j')) => {
            Some(RestackInteractionAction::MoveDown)
        }
        (RestackInteractionStage::Review, KeyCode::Up | KeyCode::Char('k')) => {
            Some(RestackInteractionAction::MoveUp)
        }
        (RestackInteractionStage::Review, KeyCode::Down | KeyCode::Char('j')) => {
            Some(RestackInteractionAction::MoveDown)
        }
        (RestackInteractionStage::Selection, KeyCode::Char(' ')) => {
            Some(RestackInteractionAction::Toggle)
        }
        (RestackInteractionStage::Selection | RestackInteractionStage::Review, KeyCode::Enter) => {
            Some(RestackInteractionAction::Continue)
        }
        (RestackInteractionStage::Review | RestackInteractionStage::Confirmation, KeyCode::Esc) => {
            Some(RestackInteractionAction::Back)
        }
        (RestackInteractionStage::Confirmation, KeyCode::Char('y' | 'Y')) => {
            Some(RestackInteractionAction::Confirm)
        }
        (RestackInteractionStage::Confirmation, KeyCode::Char('n' | 'N')) => {
            Some(RestackInteractionAction::Back)
        }
        _ => None,
    }
}

fn render(
    frame: &mut Frame<'_>,
    interaction: &RestackInteraction,
    plan: Option<&RestackPlan>,
    rejection: Option<&str>,
) {
    let area = constrain_content_width(frame.area());
    let rows = Layout::vertical([
        Constraint::Length(GRADUATE_ART_HEIGHT + 1),
        Constraint::Min(6),
        Constraint::Length(2),
    ])
    .split(area);
    render_brand_header(frame, rows[0]);
    match interaction.stage() {
        RestackInteractionStage::Selection => {
            render_selection(frame, rows[1], interaction, rejection);
        }
        RestackInteractionStage::Review => {
            render_review(frame, rows[1], plan, interaction.review_scroll());
        }
        RestackInteractionStage::Confirmation => render_confirmation(frame, rows[1], plan),
    }
    render_footer(frame, rows[2], interaction.stage());
}

fn render_selection(
    frame: &mut Frame<'_>,
    area: Rect,
    interaction: &RestackInteraction,
    rejection: Option<&str>,
) {
    let snapshot = interaction.snapshot();
    let items = snapshot
        .features
        .iter()
        .enumerate()
        .map(|(index, feature)| {
            let retained = if interaction.is_retained(index) {
                "[x]"
            } else {
                "[ ]"
            };
            let jira = jira_key_from_branch(&feature.name).unwrap_or_else(|| "—".to_owned());
            let rerere = if feature.historical_merges.is_empty() {
                "rerere unavailable"
            } else {
                "rerere history available"
            };
            ListItem::new(format!(
                "{retained} {:>2}.  {}  {}  {jira}  {rerere}",
                index + 1,
                escape(&feature.name),
                short_oid(&feature.tip),
            ))
        })
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(Some(interaction.cursor()));
    let title = format!(
        " SELECT FEATURES · {}/{} ",
        snapshot.remote, snapshot.environment
    );
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title(escape(&title)))
            .highlight_style(Palette::action_focus())
            .highlight_symbol("› "),
        area,
        &mut state,
    );
    if let Some(rejection) = rejection {
        let warning = Rect::new(
            area.x.saturating_add(2),
            area.bottom().saturating_sub(3),
            area.width.saturating_sub(4),
            2,
        );
        frame.render_widget(
            Paragraph::new(rejection.to_owned())
                .style(Palette::error())
                .wrap(Wrap { trim: true }),
            warning,
        );
    }
}

fn render_review(frame: &mut Frame<'_>, area: Rect, plan: Option<&RestackPlan>, scroll: usize) {
    let text = plan.map_or_else(
        || Text::from("The reviewed plan is unavailable."),
        review_text,
    );
    let scroll = scroll.min(usize::from(u16::MAX)) as u16;
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(" REVIEW "))
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn review_text(plan: &RestackPlan) -> Text<'static> {
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Base          ", Palette::muted()),
            Span::raw(format!(
                "{} @ {}",
                escape(&plan.snapshot.main_ref),
                escape(&plan.snapshot.main_tip)
            )),
        ]),
        Line::from(vec![
            Span::styled("Environment   ", Palette::muted()),
            Span::raw(format!(
                "{} @ {}",
                escape(&plan.snapshot.environment_ref),
                escape(&plan.snapshot.environment_tip)
            )),
        ]),
        Line::from(vec![
            Span::styled("Author        ", Palette::muted()),
            Span::raw(format!(
                "{} <{}>",
                escape(&plan.author.name),
                escape(&plan.author.email)
            )),
        ]),
        Line::from(vec![
            Span::styled("Fetch endpoint", Palette::muted()),
            Span::raw(format!(" sha256:{}", plan.remote_endpoints.fetch_sha256)),
        ]),
        Line::from(vec![
            Span::styled("Push endpoint ", Palette::muted()),
            Span::raw(format!(" sha256:{}", plan.remote_endpoints.push_sha256)),
        ]),
        Line::from(vec![
            Span::styled("Remote rewrite", Palette::muted()),
            Span::raw(format!(
                " refs/heads/{}  {} -> {}",
                escape(&plan.snapshot.environment),
                escape(&plan.snapshot.environment_tip),
                escape(&plan.preview_commit)
            )),
        ]),
        Line::from(vec![
            Span::styled("Final tree    ", Palette::muted()),
            Span::raw(escape(&plan.final_tree)),
        ]),
        Line::from(vec![
            Span::styled("Signing       ", Palette::muted()),
            Span::raw("unsigned canonical merge commits"),
        ]),
        Line::from(vec![
            Span::styled("Dropped markers", Palette::muted()),
            Span::raw(format!(
                " {} exact phase marker(s)",
                plan.snapshot.dropped_markers.len()
            )),
        ]),
        Line::from(""),
        Line::styled("Retained merge order", Palette::primary().bold()),
    ];
    if plan.selection.retained.is_empty() {
        lines.push(Line::raw("  none (environment becomes the captured base)"));
    } else {
        lines.extend(
            plan.selection
                .retained
                .iter()
                .enumerate()
                .map(|(index, branch)| {
                    let outcome = plan
                        .merges
                        .get(index)
                        .map_or("unavailable", |merge| match merge.resolution {
                            graduate::restack::MergeResolution::Clean => "clean",
                            graduate::restack::MergeResolution::Reused => "rerere reused",
                            graduate::restack::MergeResolution::Manual => "manual",
                        });
                    Line::raw(format!(
                        "  {:>2}. {} @ {} ({outcome})",
                        index + 1,
                        escape(&branch.name),
                        escape(&branch.tip)
                    ))
                }),
        );
    }
    lines.push(Line::from(""));
    lines.push(Line::styled(
        "Deliberate removals",
        Palette::warning().bold(),
    ));
    if plan.selection.removed.is_empty() {
        lines.push(Line::raw("  none"));
    } else {
        lines.extend(plan.selection.removed.iter().map(|branch| {
            Line::raw(format!(
                "  {} @ {}",
                escape(&branch.name),
                escape(&branch.tip)
            ))
        }));
    }
    Text::from(lines)
}

fn render_confirmation(frame: &mut Frame<'_>, area: Rect, plan: Option<&RestackPlan>) {
    let text = plan.map_or_else(
        || "The reviewed plan is unavailable.".to_owned(),
        |plan| {
            format!(
                "Replace {}/refs/heads/{} under an exact lease?\n\n{} -> {}\n\nThis is the only remote ref Graduate will change. The source checkout, local refs, hooks, and personal rerere cache stay untouched. Merge commits are unsigned.\n\nPress y to publish this reviewed in-memory plan.",
                escape(&plan.snapshot.remote),
                escape(&plan.snapshot.environment),
                escape(&plan.snapshot.environment_tip),
                escape(&plan.preview_commit),
            )
        },
    );
    frame.render_widget(
        Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Palette::warning())
                    .title(" CONFIRM REMOTE REWRITE "),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, stage: RestackInteractionStage) {
    let controls = match stage {
        RestackInteractionStage::Selection => {
            " ↑/↓ move   Space retain/remove   Enter review   Esc cancel "
        }
        RestackInteractionStage::Review => " ↑/↓ scroll   Enter confirm   Esc revise   q cancel ",
        RestackInteractionStage::Confirmation => " y publish   n/Esc back   q cancel ",
    };
    frame.render_widget(
        Paragraph::new(Line::styled(controls, Palette::muted())),
        area,
    );
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
        build_plan, AttributedCommit, BranchIdentity, ExplicitFeature, HistoricalMerge,
        MergeOutcome, MergeResolution, RemoteEndpointIdentity, RestackAuthor, RestackSnapshot,
    };
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::*;

    #[test]
    fn checklist_renders_order_identity_jira_key_and_rerere_availability(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let interaction = RestackInteraction::new(snapshot());
        let rendered = rendered(&interaction, None, None)?;

        assert!(rendered.contains("1.  feature/PROJ-12-one  aaaaaaa  PROJ-12"));
        assert!(rendered.contains("rerere history available"));
        assert!(rendered.contains("2.  feature/two  bbbbbbb  —  rerere unavailable"));
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
        Ok(())
    }

    #[test]
    fn review_and_confirmation_show_the_exact_rewrite_and_safety_effects(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let plan = plan()?;
        let mut interaction = RestackInteraction::new(snapshot());
        interaction.review_ready();
        let review = rendered(&interaction, Some(&plan), None)?;

        assert!(review.contains("refs/remotes/origin/main @ main-tip"));
        assert!(review.contains("refs/heads/qa"));
        assert!(review.contains("Pat <pat@example.com>"));
        assert!(review.contains("sha256:ffffffff"));
        assert!(review.contains("Retained merge order"));
        assert!(review.contains("(clean)"));
        assert!(review.contains("unsigned canonical merge commits"));
        assert!(review.contains("0 exact phase marker(s)"));

        let _ = interaction.update(RestackInteractionAction::Continue);
        let confirmation = rendered(&interaction, Some(&plan), None)?;
        assert!(confirmation.contains("exact lease"));
        assert!(confirmation.contains("environment-tip -> preview"));
        assert!(confirmation.contains("source checkout, local refs, hooks"));
        assert!(confirmation.contains("Press y to publish"));
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
            Some(RestackInteractionAction::Confirm)
        );
        assert_eq!(
            action_for_key(
                RestackInteractionStage::Confirmation,
                KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)
            ),
            Some(RestackInteractionAction::Cancel)
        );
    }

    #[test]
    fn ordinary_completion_and_conflict_handoff_are_redacted_and_actionable(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let plan = plan()?;
        let success = success_text(&plan);
        assert!(success.contains("Restacked origin/qa"));

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
        assert!(handoff.contains("gd restack qa --resume v1.safe.token"));
        Ok(())
    }

    fn rendered(
        interaction: &RestackInteraction,
        plan: Option<&RestackPlan>,
        rejection: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut terminal = Terminal::new(TestBackend::new(115, 38))?;
        terminal.draw(|frame| render(frame, interaction, plan, rejection))?;
        Ok(terminal.backend().to_string())
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
            vec![MergeOutcome {
                branch: "feature/PROJ-12-one".to_owned(),
                tip: "a".repeat(40),
                commit: "preview".to_owned(),
                tree: "tree-tip".to_owned(),
                resolution: MergeResolution::Clean,
            }],
            "tree-tip".to_owned(),
            "preview".to_owned(),
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
        }
    }
}
