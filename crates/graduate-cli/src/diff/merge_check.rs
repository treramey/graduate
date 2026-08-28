//! Read-only merge cleanliness check of a branch tip onto main.

use std::collections::HashSet;

use graduate::promotion::MergeOntoMain;

use crate::shared::environment_git::gitoxide_error;
use crate::shared::error::CliError;

/// Merge `tip` onto `main` in memory and report only whether it is clean and
/// how many paths conflict.
///
/// The merge runs on a repository handle with in-memory object storage, so
/// merged blobs and trees never reach the object database; the scanned
/// repository is left untouched. Conflict content and paths are never
/// surfaced, only the count.
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
    let repository = repository.clone().with_object_memory();
    let base = repository.merge_base(main, tip).map_err(gitoxide_error)?;
    if base.detach() == tip {
        return Ok(MergeOntoMain {
            clean: true,
            conflicting_paths: 0,
        });
    }
    let tree_of = |id: gix::ObjectId| -> Result<gix::ObjectId, CliError> {
        Ok(repository
            .find_commit(id)
            .map_err(gitoxide_error)?
            .tree_id()
            .map_err(gitoxide_error)?
            .detach())
    };
    let base_tree = tree_of(base.detach())?;
    let main_tree = tree_of(main)?;
    let tip_tree = tree_of(tip)?;
    let options = repository.tree_merge_options().map_err(gitoxide_error)?;
    let outcome = repository
        .merge_trees(
            base_tree,
            main_tree,
            tip_tree,
            gix::merge::blob::builtin_driver::text::Labels::default(),
            options,
        )
        .map_err(gitoxide_error)?;
    let how = gix::merge::tree::TreatAsUnresolved::git();
    let conflicting_paths = outcome
        .conflicts
        .iter()
        .filter(|conflict| conflict.is_unresolved(how))
        .count();
    Ok(MergeOntoMain {
        clean: conflicting_paths == 0,
        conflicting_paths,
    })
}
