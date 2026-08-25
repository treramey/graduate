//! Release-gated machine workflow for isolated clean restack previews.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use graduate::restack::{
    build_plan, canonical_merge_message, select_features, InventoryError, MergeOutcome, PlanError,
    RestackAuthor, RestackPlan, SelectionError, RESTACK_SCHEMA_VERSION,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cli::RestackArgs;
use crate::environment_git::{
    inspect_environment, restack_snapshot, validate_ref_component, RestackInspectionError,
};
use crate::error::{CliError, MachineError};
use crate::git_process;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreviewParams {
    remove_branches: Vec<String>,
}

pub(crate) fn run(args: RestackArgs) -> Result<(), CliError> {
    validate_inputs(&args)?;
    let params = parse_params(args.params.as_deref())?;
    let source = std::env::current_dir().map_err(|_| {
        machine_failure(
            "repository_unavailable",
            "could not access the source repository",
            json!({"stage": "currentDirectory"}),
        )
    })?;

    git_process::fetch_restack_remote(&args.remote, &source).map_err(|_| {
        machine_failure(
            "fetch_failed",
            "could not fetch the selected remote",
            json!({"remote": args.remote}),
        )
    })?;

    let repository = gix::discover(&source).map_err(|_| {
        machine_failure(
            "repository_not_found",
            "the current directory is not inside a Git repository",
            json!({}),
        )
    })?;
    let inspection = inspect_environment(
        &repository,
        &args.remote,
        &args.environment,
        args.main.as_deref(),
    )
    .map_err(|_| {
        machine_failure(
            "inspection_failed",
            "could not inspect the fetched environment refs",
            json!({"stage": "refs"}),
        )
    })?;
    let snapshot = restack_snapshot(&repository, &inspection).map_err(inspection_error)?;
    let selection = select_features(&snapshot, &params.remove_branches).map_err(selection_error)?;
    let author = configured_author(&source)?;
    let source_objects = source_object_directory(&source)?;
    let isolated = IsolatedRepository::create(&source_objects)?;
    let reconstruction = isolated.reconstruct(
        &snapshot.main_tip,
        &snapshot.environment,
        &selection.retained,
        &author,
    )?;
    let plan = build_plan(
        snapshot,
        author,
        selection,
        reconstruction.merges,
        reconstruction.final_tree,
        reconstruction.preview_commit,
    )
    .map_err(plan_error)?;
    write_plan(&plan)
}

fn validate_inputs(args: &RestackArgs) -> Result<(), CliError> {
    for (label, value) in [
        ("environment", args.environment.as_str()),
        ("remote", args.remote.as_str()),
    ] {
        validate_ref_component(label, value).map_err(|_| {
            machine_usage(
                "invalid_ref",
                "a restack ref name is not valid",
                json!({"field": label}),
            )
        })?;
    }
    if let Some(main) = &args.main {
        validate_ref_component("main", main).map_err(|_| {
            machine_usage(
                "invalid_ref",
                "a restack ref name is not valid",
                json!({"field": "main"}),
            )
        })?;
    }
    Ok(())
}

fn parse_params(params: Option<&str>) -> Result<PreviewParams, CliError> {
    let Some(params) = params else {
        return Err(machine_usage(
            "params_required",
            "the release-gated restack preview requires --params",
            json!({"expected": {"removeBranches": []}}),
        ));
    };
    let parsed: PreviewParams = serde_json::from_str(params).map_err(|_| {
        machine_usage(
            "invalid_params",
            "--params must match the schema-v1 restack preview parameters",
            json!({"expected": {"removeBranches": ["feature/BRANCH"]}}),
        )
    })?;
    for (index, branch) in parsed.remove_branches.iter().enumerate() {
        validate_ref_component("removeBranches entry", branch).map_err(|_| {
            machine_usage(
                "invalid_params",
                "removeBranches contains an invalid Git branch name",
                json!({"index": index}),
            )
        })?;
    }
    Ok(parsed)
}

fn configured_author(source: &Path) -> Result<RestackAuthor, CliError> {
    let name = source_config(source, "user.name")?;
    let email = source_config(source, "user.email")?;
    if !valid_identity_value(&name) || !valid_identity_value(&email) {
        return Err(machine_failure(
            "missing_identity",
            "Git user.name and user.email must be configured",
            json!({}),
        ));
    }
    Ok(RestackAuthor { name, email })
}

fn source_config(source: &Path, key: &str) -> Result<String, CliError> {
    let output = source_git(source)
        .args(["config", "--get", key])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| {
            machine_failure(
                "git_unavailable",
                "could not run Git for restack preflight",
                json!({"stage": "identity"}),
            )
        })?;
    if !output.status.success() {
        return Err(machine_failure(
            "missing_identity",
            "Git user.name and user.email must be configured",
            json!({}),
        ));
    }
    let value = String::from_utf8(output.stdout).map_err(|_| {
        machine_failure(
            "invalid_identity",
            "the configured Git identity is not valid UTF-8",
            json!({}),
        )
    })?;
    Ok(value.trim_end_matches(['\r', '\n']).to_owned())
}

fn valid_identity_value(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}

fn source_object_directory(source: &Path) -> Result<Vec<u8>, CliError> {
    let output = source_git(source)
        .args([
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "objects",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| {
            machine_failure(
                "git_unavailable",
                "could not run Git for restack preflight",
                json!({"stage": "objectStore"}),
            )
        })?;
    if !output.status.success() {
        return Err(machine_failure(
            "object_store_unavailable",
            "could not locate the source repository object store",
            json!({}),
        ));
    }
    let mut path = output.stdout;
    while matches!(path.last(), Some(b'\r' | b'\n')) {
        path.pop();
    }
    if path.is_empty() || path.contains(&b'\n') || path.contains(&b'\r') {
        return Err(machine_failure(
            "object_store_unavailable",
            "could not locate the source repository object store",
            json!({}),
        ));
    }
    Ok(path)
}

fn source_git(source: &Path) -> Command {
    let mut command = Command::new("git");
    clear_repository_location_environment(&mut command);
    command.current_dir(source);
    command
}

struct Reconstruction {
    merges: Vec<MergeOutcome>,
    final_tree: String,
    preview_commit: String,
}

struct IsolatedRepository {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    hooks: PathBuf,
    global_config: PathBuf,
}

impl IsolatedRepository {
    fn create(source_objects: &[u8]) -> Result<Self, CliError> {
        let temporary = tempfile::tempdir().map_err(|_| isolated_setup_error())?;
        let root = temporary.path().join("repository");
        let hooks = temporary.path().join("hooks");
        let global_config = temporary.path().join("global.gitconfig");
        fs::create_dir(&root).map_err(|_| isolated_setup_error())?;
        fs::create_dir(&hooks).map_err(|_| isolated_setup_error())?;
        fs::write(&global_config, []).map_err(|_| isolated_setup_error())?;

        let isolated = Self {
            _temporary: temporary,
            root,
            hooks,
            global_config,
        };
        isolated.run_success(["init", "--quiet"], "initialize")?;
        let alternates = isolated.root.join(".git/objects/info/alternates");
        let mut contents = source_objects.to_vec();
        contents.push(b'\n');
        fs::write(alternates, contents).map_err(|_| isolated_setup_error())?;
        Ok(isolated)
    }

    fn reconstruct(
        &self,
        main_tip: &str,
        environment: &str,
        retained: &[graduate::restack::BranchIdentity],
        author: &RestackAuthor,
    ) -> Result<Reconstruction, CliError> {
        self.run_success(
            ["checkout", "--detach", "--quiet", main_tip, "--"],
            "checkoutBase",
        )?;
        let mut merges = Vec::with_capacity(retained.len());
        let mut previous = main_tip.to_owned();
        for feature in retained {
            let mut merge_command = self.command();
            merge_command
                .args([
                    "merge",
                    "--no-ff",
                    "--no-commit",
                    "--no-edit",
                    "--no-gpg-sign",
                    feature.tip.as_str(),
                ])
                .env("GIT_AUTHOR_NAME", &author.name)
                .env("GIT_AUTHOR_EMAIL", &author.email)
                .env("GIT_COMMITTER_NAME", &author.name)
                .env("GIT_COMMITTER_EMAIL", &author.email)
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let merge = merge_command
                .output()
                .map_err(|_| reconstruction_error("merge"))?;
            if !merge.status.success() {
                let unresolved = self.unresolved_paths()?;
                if !unresolved.is_empty() {
                    return Err(machine_failure(
                        "reconstruction_conflict",
                        "the clean restack preview has unresolved conflicts",
                        json!({"branch": feature.name, "unresolvedPaths": unresolved}),
                    ));
                }
                return Err(reconstruction_error("merge"));
            }
            self.validate_index()?;
            let tree = self.read_text(["write-tree"], "writeTree")?;
            let message = canonical_merge_message(&feature.name, environment);
            let commit = self.commit_tree(&tree, &previous, &feature.tip, &message, author)?;
            self.run_success(["reset", "--hard", "--quiet", &commit], "resetResult")?;
            self.validate_commit(&commit, &previous, &feature.tip, &message, author)?;
            self.validate_clean_state(&previous, &commit)?;
            previous.clone_from(&commit);
            merges.push(MergeOutcome {
                branch: feature.name.clone(),
                tip: feature.tip.clone(),
                commit,
                tree,
            });
        }
        self.validate_clean_state(main_tip, &previous)?;
        let final_tree = self.read_text(["rev-parse", "HEAD^{tree}"], "finalTree")?;
        if let Some(last) = merges.last() {
            if last.tree != final_tree {
                return Err(validation_error("finalTree"));
            }
        } else {
            let base_tree =
                self.read_text(["rev-parse", &format!("{main_tip}^{{tree}}")], "baseTree")?;
            if base_tree != final_tree || previous != main_tip {
                return Err(validation_error("finalTree"));
            }
        }
        Ok(Reconstruction {
            merges,
            final_tree,
            preview_commit: previous,
        })
    }

    fn commit_tree(
        &self,
        tree: &str,
        first_parent: &str,
        feature_parent: &str,
        message: &str,
        author: &RestackAuthor,
    ) -> Result<String, CliError> {
        let mut command = self.command();
        command
            .args([
                "commit-tree",
                tree,
                "-p",
                first_parent,
                "-p",
                feature_parent,
                "-m",
                message,
            ])
            .env("GIT_AUTHOR_NAME", &author.name)
            .env("GIT_AUTHOR_EMAIL", &author.email)
            .env("GIT_COMMITTER_NAME", &author.name)
            .env("GIT_COMMITTER_EMAIL", &author.email)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        read_success_text(command.output(), "commitTree")
    }

    fn validate_index(&self) -> Result<(), CliError> {
        let unmerged = self.read_bytes(["ls-files", "-u", "-z"], "unmergedIndex")?;
        if !unmerged.is_empty() {
            return Err(validation_error("unmergedIndex"));
        }
        self.run_success(["diff", "--cached", "--check"], "stagedDiffCheck")?;
        self.run_success(["diff", "--check"], "worktreeDiffCheck")
    }

    fn validate_commit(
        &self,
        commit: &str,
        first_parent: &str,
        feature_parent: &str,
        message: &str,
        author: &RestackAuthor,
    ) -> Result<(), CliError> {
        let parents = self.read_text(["rev-list", "--parents", "-n", "1", commit], "parents")?;
        let expected = format!("{commit} {first_parent} {feature_parent}");
        if parents != expected {
            return Err(validation_error("parents"));
        }
        let actual_message = self.read_text(["show", "-s", "--format=%B", commit], "message")?;
        if actual_message != message {
            return Err(validation_error("message"));
        }
        let identity = self.read_bytes(
            ["show", "-s", "--format=%an%x00%ae%x00%cn%x00%ce", commit],
            "identity",
        )?;
        let expected_identity = format!(
            "{}\0{}\0{}\0{}\n",
            author.name, author.email, author.name, author.email
        );
        if identity != expected_identity.as_bytes() {
            return Err(validation_error("identity"));
        }
        let raw = self.read_bytes(["cat-file", "commit", commit], "signature")?;
        let headers = raw
            .split(|byte| *byte == b'\n')
            .take_while(|line| !line.is_empty());
        if headers.into_iter().any(|line| line.starts_with(b"gpgsig ")) {
            return Err(validation_error("signature"));
        }
        Ok(())
    }

    fn validate_clean_state(&self, previous: &str, commit: &str) -> Result<(), CliError> {
        let status = self.read_bytes(["status", "--porcelain=v1", "-z"], "status")?;
        if !status.is_empty() {
            return Err(validation_error("indexState"));
        }
        self.run_success(
            ["diff-tree", "--check", previous, commit],
            "resultDiffCheck",
        )
    }

    fn unresolved_paths(&self) -> Result<Vec<String>, CliError> {
        let bytes = self.read_bytes(
            ["diff", "--name-only", "--diff-filter=U", "-z"],
            "conflictPaths",
        )?;
        bytes
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| {
                String::from_utf8(path.to_vec()).map_err(|_| {
                    machine_failure(
                        "invalid_path_encoding",
                        "an unresolved path is not valid UTF-8",
                        json!({}),
                    )
                })
            })
            .collect()
    }

    fn read_text<const N: usize>(
        &self,
        arguments: [&str; N],
        stage: &'static str,
    ) -> Result<String, CliError> {
        let bytes = self.read_bytes(arguments, stage)?;
        let text = String::from_utf8(bytes).map_err(|_| validation_error(stage))?;
        Ok(text.trim_end_matches(['\r', '\n']).to_owned())
    }

    fn read_bytes<const N: usize>(
        &self,
        arguments: [&str; N],
        stage: &'static str,
    ) -> Result<Vec<u8>, CliError> {
        let mut command = self.command();
        command
            .args(arguments)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let output = command.output().map_err(|_| reconstruction_error(stage))?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(validation_error(stage))
        }
    }

    fn run_success<const N: usize>(
        &self,
        arguments: [&str; N],
        stage: &'static str,
    ) -> Result<(), CliError> {
        let output = self
            .command()
            .args(arguments)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|_| reconstruction_error(stage))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(reconstruction_error(stage))
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new("git");
        clear_isolated_environment(&mut command);
        command
            .current_dir(&self.root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &self.global_config)
            .env("GIT_CONFIG_COUNT", "7")
            .env("GIT_CONFIG_KEY_0", "core.hooksPath")
            .env("GIT_CONFIG_VALUE_0", &self.hooks)
            .env("GIT_CONFIG_KEY_1", "core.fsmonitor")
            .env("GIT_CONFIG_VALUE_1", "false")
            .env("GIT_CONFIG_KEY_2", "commit.gpgSign")
            .env("GIT_CONFIG_VALUE_2", "false")
            .env("GIT_CONFIG_KEY_3", "tag.gpgSign")
            .env("GIT_CONFIG_VALUE_3", "false")
            .env("GIT_CONFIG_KEY_4", "rerere.enabled")
            .env("GIT_CONFIG_VALUE_4", "false")
            .env("GIT_CONFIG_KEY_5", "rerere.autoupdate")
            .env("GIT_CONFIG_VALUE_5", "false")
            .env("GIT_CONFIG_KEY_6", "core.autocrlf")
            .env("GIT_CONFIG_VALUE_6", "false")
            .env("GIT_MERGE_AUTOEDIT", "no")
            .env("GIT_TERMINAL_PROMPT", "0");
        command
    }
}

fn clear_repository_location_environment(command: &mut Command) {
    for variable in [
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_DIR",
        "GIT_GRAFT_FILE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_PREFIX",
        "GIT_QUARANTINE_PATH",
        "GIT_SHALLOW_FILE",
        "GIT_WORK_TREE",
    ] {
        command.env_remove(variable);
    }
}

fn clear_isolated_environment(command: &mut Command) {
    clear_repository_location_environment(command);
    for variable in [
        "GIT_AUTHOR_DATE",
        "GIT_AUTHOR_EMAIL",
        "GIT_AUTHOR_NAME",
        "GIT_COMMITTER_DATE",
        "GIT_COMMITTER_EMAIL",
        "GIT_COMMITTER_NAME",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_CONFIG_SYSTEM",
        "GIT_EXEC_PATH",
    ] {
        command.env_remove(variable);
    }
}

fn read_success_text(
    result: std::io::Result<Output>,
    stage: &'static str,
) -> Result<String, CliError> {
    let output = result.map_err(|_| reconstruction_error(stage))?;
    if !output.status.success() {
        return Err(reconstruction_error(stage));
    }
    let text = String::from_utf8(output.stdout).map_err(|_| validation_error(stage))?;
    Ok(text.trim_end_matches(['\r', '\n']).to_owned())
}

fn inspection_error(error: RestackInspectionError) -> CliError {
    match error {
        RestackInspectionError::Git(_) => machine_failure(
            "inspection_failed",
            "could not inspect the fetched environment history",
            json!({"stage": "history"}),
        ),
        RestackInspectionError::Unsupported(error) => inventory_error(error),
    }
}

fn inventory_error(error: InventoryError) -> CliError {
    let details = match error {
        InventoryError::MissingCommit { commit } => {
            json!({"kind": "missingCommit", "commit": commit})
        }
        InventoryError::DirectCommit { commit } => {
            json!({"kind": "directCommit", "commit": commit})
        }
        InventoryError::FastForwardHistory { commit, branches } => {
            json!({"kind": "fastForwardHistory", "commit": commit, "branches": branches})
        }
        InventoryError::OctopusMerge {
            merge_commit,
            parents,
        } => {
            json!({"kind": "octopusMerge", "mergeCommit": merge_commit, "parents": parents})
        }
        InventoryError::DeletedFeatureRef {
            merge_commit,
            feature_parent,
        } => {
            json!({"kind": "deletedFeatureRef", "mergeCommit": merge_commit, "featureParent": feature_parent})
        }
        InventoryError::AmbiguousFeatureRefs {
            merge_commit,
            feature_parent,
            branches,
        } => {
            json!({"kind": "ambiguousFeatureRefs", "mergeCommit": merge_commit, "featureParent": feature_parent, "branches": branches})
        }
    };
    machine_failure(
        "unsupported_history",
        "the environment history cannot be reconstructed without guessing",
        details,
    )
}

fn selection_error(error: SelectionError) -> CliError {
    let (kind, branch, dependents) = match error {
        SelectionError::Duplicate { branch } => ("duplicate", branch, Vec::new()),
        SelectionError::Graduated { branch } => ("graduated", branch, Vec::new()),
        SelectionError::IndirectOnly { branch } => ("indirectOnly", branch, Vec::new()),
        SelectionError::Unknown { branch } => ("unknown", branch, Vec::new()),
        SelectionError::RetainedDependency { branch, dependents } => {
            ("retainedDependency", branch, dependents)
        }
    };
    machine_usage(
        "invalid_removal",
        "removeBranches contains a feature that cannot be removed",
        json!({"kind": kind, "branch": branch, "dependents": dependents}),
    )
}

fn plan_error(error: PlanError) -> CliError {
    let details = match error {
        PlanError::MergeCount { expected, actual } => {
            json!({"stage": "mergeCount", "expected": expected, "actual": actual})
        }
        PlanError::MergeIdentity { index, expected } => {
            json!({"stage": "mergeIdentity", "index": index, "expected": expected})
        }
    };
    machine_failure(
        "validation_failed",
        "isolated reconstruction did not match the selected plan",
        details,
    )
}

fn write_plan(plan: &RestackPlan) -> Result<(), CliError> {
    let value = plan_json(plan);
    let output = serde_json::to_string(&value).map_err(|_| {
        machine_failure(
            "serialization_failed",
            "could not serialize the restack plan",
            json!({}),
        )
    })?;
    writeln!(std::io::stdout().lock(), "{output}").map_err(|_| {
        machine_failure(
            "output_failed",
            "could not write the restack plan to stdout",
            json!({}),
        )
    })
}

fn plan_json(plan: &RestackPlan) -> Value {
    let branches = |branches: &[graduate::restack::BranchIdentity]| {
        branches
            .iter()
            .map(|branch| json!({"name": branch.name, "tip": branch.tip}))
            .collect::<Vec<_>>()
    };
    let mut first_parent = plan.snapshot.main_tip.as_str();
    let merges = plan
        .merges
        .iter()
        .map(|merge| {
            let value = json!({
                "branch": merge.branch,
                "tip": merge.tip,
                "outcome": "clean",
                "commit": merge.commit,
                "tree": merge.tree,
                "firstParent": first_parent,
                "featureParent": merge.tip,
                "message": canonical_merge_message(&merge.branch, &plan.snapshot.environment),
            });
            first_parent = &merge.commit;
            value
        })
        .collect::<Vec<_>>();
    json!({
        "kind": "restackPlan",
        "schemaVersion": RESTACK_SCHEMA_VERSION,
        "remote": plan.snapshot.remote,
        "environment": {
            "name": plan.snapshot.environment,
            "ref": plan.snapshot.environment_ref,
            "oid": plan.snapshot.environment_tip,
        },
        "base": {
            "name": plan.snapshot.main,
            "ref": plan.snapshot.main_ref,
            "oid": plan.snapshot.main_tip,
        },
        "author": {"name": plan.author.name, "email": plan.author.email},
        "retainedBranches": branches(&plan.selection.retained),
        "removedBranches": branches(&plan.selection.removed),
        "droppedMarkers": plan.snapshot.dropped_markers.iter().map(|marker| json!({
            "commit": marker.commit,
            "parent": marker.parent,
            "tree": marker.tree,
        })).collect::<Vec<_>>(),
        "merges": merges,
        "finalTree": plan.final_tree,
        "previewCommit": plan.preview_commit,
        "planDigest": plan.digest,
        "effects": {
            "fetchedRemoteTrackingRefs": true,
            "pushed": false,
            "sourceCheckoutChanged": false,
            "localRefsChanged": false,
            "personalRerereChanged": false,
            "commitSigning": "unsigned",
        },
    })
}

fn isolated_setup_error() -> CliError {
    machine_failure(
        "isolated_setup_failed",
        "could not create the isolated restack work area",
        json!({}),
    )
}

fn reconstruction_error(stage: &'static str) -> CliError {
    machine_failure(
        "reconstruction_failed",
        "Git could not complete isolated reconstruction",
        json!({"stage": stage}),
    )
}

fn validation_error(stage: &'static str) -> CliError {
    machine_failure(
        "validation_failed",
        "isolated reconstruction failed validation",
        json!({"stage": stage}),
    )
}

fn machine_usage(code: &'static str, message: &'static str, details: Value) -> CliError {
    MachineError::usage(code, message, details).into()
}

fn machine_failure(code: &'static str, message: &'static str, details: Value) -> CliError {
    MachineError::failure(code, message, details).into()
}
