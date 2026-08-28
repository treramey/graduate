//! Read-only merge cleanliness check of a branch tip onto main.

use std::collections::HashSet;

use graduate::promotion::MergeOntoMain;

use crate::shared::environment_git::gitoxide_error;
use crate::shared::error::CliError;

/// Merge `tip` onto `main` in memory and report only whether it is clean and
/// how many paths conflict.
///
/// The merge runs on a repository handle with in-memory object storage, so
/// merged blobs and trees never reach the object database. Configured
/// external merge drivers are dropped from that handle first, so the check
/// never runs a process. Criss-cross histories use the recursive virtual
/// merge base as Git does; unrelated histories merge against an empty tree
/// and report their conflicts instead of failing the report. Conflict content
/// and paths are never surfaced, only the count.
pub(super) fn merge_onto_main(
    repository: &gix::Repository,
    main_ancestors: &HashSet<gix::ObjectId>,
    main: gix::ObjectId,
    tip: gix::ObjectId,
) -> Result<MergeOntoMain, CliError> {
    if main_ancestors.contains(&tip) {
        return Ok(MergeOntoMain {
            clean: true,
            conflicting_paths: 0,
        });
    }
    let mut repository = repository.clone().with_object_memory();
    drop_external_merge_drivers(&mut repository)?;
    let options =
        gix::merge::commit::Options::from(repository.tree_merge_options().map_err(gitoxide_error)?)
            .with_allow_missing_merge_base(true);
    let outcome = repository
        .merge_commits(
            main,
            tip,
            gix::merge::blob::builtin_driver::text::Labels::default(),
            options,
        )
        .map_err(gitoxide_error)?;
    let how = gix::merge::tree::TreatAsUnresolved::git();
    let conflicting_paths = outcome
        .tree_merge
        .conflicts
        .iter()
        .filter(|conflict| conflict.is_unresolved(how))
        .count();
    Ok(MergeOntoMain {
        clean: conflicting_paths == 0,
        conflicting_paths,
    })
}

/// Remove every `merge.<driver>.*` section so gitoxide only uses its built-in
/// text and binary drivers.
fn drop_external_merge_drivers(repository: &mut gix::Repository) -> Result<(), CliError> {
    let mut config = repository.config_snapshot_mut();
    let driver_sections = config
        .sections_and_ids_by_name("merge")
        .into_iter()
        .flatten()
        .filter(|(section, _)| section.header().subsection_name().is_some())
        .map(|(_, id)| id)
        .collect::<Vec<_>>();
    for id in driver_sections {
        config.remove_section_by_id(id);
    }
    config.commit().map_err(gitoxide_error)?;
    Ok(())
}
