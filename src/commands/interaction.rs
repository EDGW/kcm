use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use kako_craft_lib::container::{
    BrokenLinkError, CheckRepairAction, Container, ContainerWriteGuard, LinkAccessError,
    LinkCheckIssue, LinkCheckKind, LinkToError, LinkUnavailableError, LinkValidationReport,
    UnlinkToError, WriterError, open_container,
};

use crate::cli::CheckArgs;

use super::render::{open_corresponding, validation_issue_kind};

pub(crate) fn check(path: &Path, args: CheckArgs) -> Result<()> {
    run_check_interaction(path, &args.validate_with).map(|_| ())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CheckInteractionOutcome {
    skipped_issues: usize,
}

pub(crate) fn run_check_interaction(
    path: &Path,
    validate_with: &[PathBuf],
) -> Result<CheckInteractionOutcome> {
    let container = open_container(path)?;
    let corresponding = open_corresponding(validate_with)?;
    let refs = corresponding
        .iter()
        .map(|container| container.as_ref())
        .collect::<Vec<_>>();
    let mut writer = container.writer()?;
    interact_check(writer.as_mut(), &refs)
}

pub(crate) fn validations_need_fix(validations: &[Option<LinkValidationReport>]) -> bool {
    validations
        .iter()
        .flatten()
        .any(|report| !report.is_valid())
}

fn interact_check(
    writer: &mut dyn ContainerWriteGuard,
    corresponding: &[&dyn Container],
) -> Result<CheckInteractionOutcome> {
    let mut skipped = BTreeSet::new();
    let mut announced = false;
    loop {
        let issues = writer.check(corresponding)?;
        let remaining = issues
            .into_iter()
            .filter(|issue| !skipped.contains(&issue.id))
            .collect::<Vec<_>>();
        if remaining.is_empty() {
            if announced {
                eprintln!("link consistency interaction complete");
            } else {
                eprintln!("no link consistency issues found");
            }
            return Ok(CheckInteractionOutcome {
                skipped_issues: skipped.len(),
            });
        }
        if !announced {
            eprintln!("found {} link consistency issue(s)", remaining.len());
            announced = true;
        }
        let issue = &remaining[0];
        eprintln!("\n{}: {}", issue.key, describe_issue(issue));
        if let Some(expected) = &issue.expected {
            eprintln!("  expected: {expected}");
        }
        if let Some(actual) = &issue.actual {
            eprintln!("  actual:   {actual}");
        }
        for (index, action) in issue.actions.iter().enumerate() {
            eprintln!("  {}. {}", index + 1, check_action(issue, *action));
        }
        loop {
            eprint!("  select action [1-{}]: ", issue.actions.len());
            io::stderr().flush()?;
            let mut answer = String::new();
            if io::stdin().read_line(&mut answer)? == 0 {
                bail!("standard input closed during link-check interaction");
            }
            let Ok(selection) = answer.trim().parse::<usize>() else {
                eprintln!("  please enter an action number");
                continue;
            };
            let Some(action) = selection
                .checked_sub(1)
                .and_then(|index| issue.actions.get(index))
                .copied()
            else {
                eprintln!("  please enter an action number from the list");
                continue;
            };
            if action == CheckRepairAction::RemoveLocalOutgoingOnly {
                eprint!("  type 'remove-local-only' to confirm this one-sided cleanup: ");
                io::stderr().flush()?;
                let mut confirmation = String::new();
                if io::stdin().read_line(&mut confirmation)? == 0 {
                    bail!("standard input closed during link-check confirmation");
                }
                if confirmation.trim() != "remove-local-only" {
                    eprintln!("  confirmation did not match; no data changed");
                    continue;
                }
            }
            match writer.apply_check_action(issue, action, corresponding) {
                Ok(result) => {
                    eprintln!("  completed: {}", result.description);
                    if action == CheckRepairAction::Skip {
                        skipped.insert(issue.id.clone());
                    }
                    break;
                }
                Err(error) => {
                    eprintln!("  action failed: {error:#}");
                    eprintln!("  choose an action again");
                }
            }
        }
    }
}

pub(crate) fn with_auto_fix<T>(
    path: &Path,
    auto_fix: bool,
    mut operation: impl FnMut() -> Result<T>,
) -> Result<T> {
    match operation() {
        Ok(value) => Ok(value),
        Err(error) if auto_fix && is_repairable_link_error(&error) => {
            eprintln!("operation found a broken link: {error:#}");
            let container = open_container(path)?;
            let outcome = {
                let mut writer = container.writer()?;
                interact_check(writer.as_mut(), &[])?
            };
            if outcome.skipped_issues != 0 {
                bail!(
                    "link consistency still has {} skipped issue(s); operation was not retried",
                    outcome.skipped_issues
                );
            }
            operation().context("operation still failed after interactive link check")
        }
        Err(error) => Err(error),
    }
}

fn is_repairable_link_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<BrokenLinkError>().is_some()
            || cause.downcast_ref::<LinkUnavailableError>().is_some()
    }) || matches!(
        error.downcast_ref::<UnlinkToError>(),
        Some(UnlinkToError::Access(
            LinkAccessError::Broken(_) | LinkAccessError::Unavailable(_)
        ))
    ) || matches!(
        error.downcast_ref::<LinkToError>(),
        Some(LinkToError::Access(
            LinkAccessError::Broken(_) | LinkAccessError::Unavailable(_)
        ))
    )
}

pub(crate) fn is_unavailable_link_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<LinkUnavailableError>().is_some()
            || matches!(
                cause.downcast_ref::<WriterError>(),
                Some(WriterError::ContainerLocked)
            )
    }) || matches!(
        error.downcast_ref::<UnlinkToError>(),
        Some(UnlinkToError::Access(LinkAccessError::Unavailable(_)))
    ) || matches!(
        error.downcast_ref::<LinkToError>(),
        Some(LinkToError::Access(LinkAccessError::Unavailable(_)))
    )
}

fn describe_issue(issue: &LinkCheckIssue) -> String {
    match issue.kind {
        LinkCheckKind::MissingSymlink => "metadata entry is missing its symlink",
        LinkCheckKind::IncorrectSymlink => "metadata entry has an incorrect symlink",
        LinkCheckKind::UnrecordedSymlink => "filesystem symlink is not recorded in metadata",
        LinkCheckKind::Validation(kind) => validation_issue_kind(kind),
        LinkCheckKind::Unavailable => "corresponding container is temporarily unavailable",
    }
    .to_owned()
}

pub(crate) fn check_action(issue: &LinkCheckIssue, action: CheckRepairAction) -> String {
    match action {
        CheckRepairAction::CreateMissingSymlink => format!(
            "create missing symlink '{}' -> '{}'",
            issue.key,
            issue.expected.as_deref().unwrap_or("recorded target")
        ),
        CheckRepairAction::ReplaceIncorrectSymlink => format!(
            "replace symlink '{}' with target '{}'",
            issue.key,
            issue.expected.as_deref().unwrap_or("recorded target")
        ),
        CheckRepairAction::DeleteUnrecordedSymlink => {
            format!("delete unrecorded symlink '{}'", issue.key)
        }
        CheckRepairAction::AddMissingIncomingRecord => format!(
            "add missing reciprocal incoming record for linker '{}:{}'",
            issue
                .corresponding_container_uid
                .as_deref()
                .unwrap_or("current"),
            issue.linker_key.as_deref().unwrap_or(&issue.key)
        ),
        CheckRepairAction::RemoveStaleIncomingRecord => format!(
            "remove stale incoming record '{}:{}'",
            issue
                .corresponding_container_uid
                .as_deref()
                .unwrap_or("unknown"),
            issue.linker_key.as_deref().unwrap_or(&issue.key)
        ),
        CheckRepairAction::AddMissingOutgoingRecord => {
            format!("add missing outgoing record for '{}'", issue.key)
        }
        CheckRepairAction::RemoveStaleOutgoingRecord => {
            format!("remove stale outgoing record for '{}'", issue.key)
        }
        CheckRepairAction::RemoveLocalOutgoingOnly => format!(
            "DANGEROUS: remove only local outgoing record and symlink '{}'",
            issue.key
        ),
        CheckRepairAction::RetryUnavailable => format!(
            "retry unavailable container '{}'",
            issue
                .corresponding_container_path
                .as_deref()
                .map_or_else(|| "unknown".into(), |path| path.display().to_string())
        ),
        CheckRepairAction::Skip => "skip this issue without changing data".into(),
    }
}
