//! List loading, optional validation, repair coordination, and output selection.

mod entry;

use std::path::Path;

use anyhow::{Context, Result};
use kako_craft_lib::container::{
    ContainerEntryInfo, ContainerMetadata, LinkInfo, LinkValidationReport, open_container,
};
use kako_craft_lib::locator::ContainerLocator;

use crate::cli::ListOptions;

use self::entry::{entry_json, entry_text};
use super::super::interaction::{run_check_interaction, validations_need_fix, with_auto_fix};
use super::super::{open_configurable, open_link};
use super::peers::open_corresponding;
use super::validation::ensure_validation_success;

/// Lists all ordinary and outgoing-link entries using the requested detail and validation options.
///
/// # Arguments
///
/// * `path` - Root of any supported container kind.
/// * `options` - JSON, verbosity, link-info, peer-validation, and auto-fix controls.
///
/// # Returns
///
/// `Ok(())` after entries and requested details are rendered and all validations are successful.
///
/// # Errors
///
/// Returns an error for container access, listing, validation, interaction, serialization, or a
/// broken or unavailable validation result.
pub(crate) fn print_container_list(path: &Path, options: ListOptions) -> Result<()> {
    print_list(path, options, || open_container(path)?.list_info())
}

/// Lists only outgoing-link entries from a link-capable container.
///
/// # Arguments
///
/// * `path` - Root required to declare concrete kind `link` or `configurable`.
/// * `options` - JSON, verbosity, link-info, peer-validation, and auto-fix controls.
///
/// # Returns
///
/// `Ok(())` after outgoing entries are rendered and all requested validations succeed.
///
/// # Errors
///
/// Returns an error for kind mismatch, container access, listing, validation, interaction,
/// serialization, or a broken or unavailable validation result.
pub(crate) fn print_link_list(path: &Path, options: ListOptions) -> Result<()> {
    print_list(path, options, || {
        match ContainerMetadata::load(path)?.kind.as_str() {
            "link" => open_link(path)?.link_list_info(),
            "configurable" => open_configurable(path)?.link_list_info(),
            kind => {
                anyhow::bail!("link-list requires a link or configurable container, found '{kind}'")
            }
        }
    })
}

/// Lists only ordinary local entries from a link-capable container.
///
/// # Arguments
///
/// * `path` - Root required to declare concrete kind `link` or `configurable`.
/// * `options` - JSON, verbosity, incoming-link info, peer-validation, and auto-fix controls.
///
/// # Returns
///
/// `Ok(())` after ordinary entries are rendered and all requested validations succeed.
///
/// # Errors
///
/// Returns an error for kind mismatch, container access, listing, validation, interaction,
/// serialization, or a broken or unavailable validation result.
pub(crate) fn print_local_list(path: &Path, options: ListOptions) -> Result<()> {
    print_list(path, options, || {
        match ContainerMetadata::load(path)?.kind.as_str() {
            "link" => open_link(path)?.local_list_info(),
            "configurable" => open_configurable(path)?.local_list_info(),
            kind => anyhow::bail!(
                "local-list requires a link or configurable container, found '{kind}'"
            ),
        }
    })
}

/// Coordinates list loading, optional repair, validation, reload, and final rendering.
///
/// # Arguments
///
/// * `path` - Current container root used for validation and interactive repair.
/// * `options` - Detail, output, link-info, corresponding-peer, and auto-fix policy.
/// * `load` - Repeatable operation that returns the exact entry subset for this list command.
///
/// # Returns
///
/// `Ok(())` after a simple key list or detailed entries are printed and validation status permits a
/// successful exit.
///
/// # Errors
///
/// Returns an error when loading, auto-fix, peer validation, post-repair reload, rendering, or
/// validation-success enforcement fails.
fn print_list(
    path: &Path,
    options: ListOptions,
    mut load: impl FnMut() -> Result<Vec<ContainerEntryInfo>>,
) -> Result<()> {
    let mut entries = with_auto_fix(path, options.auto_fix, &mut load)?;
    if options.verbose == 0 && !options.link_info {
        return render_entry_list(entries, &options, Vec::new());
    }
    let mut validations = validate_entries(path, &entries, &options.validate_with)?;
    if options.auto_fix && validations_need_fix(&validations) {
        run_check_interaction(path, &options.validate_with)?;
        entries = load().context("failed to reload entries after interactive link check")?;
        validations = validate_entries(path, &entries, &options.validate_with)?;
    }
    render_entry_list(entries, &options, validations)
}

/// Renders preloaded entries as plain keys, detailed text rows, or structured JSON.
///
/// # Arguments
///
/// * `entries` - Ordered entry records to render.
/// * `options` - Output format, verbosity, and link-info controls.
/// * `validations` - Per-entry reports aligned by index with `entries`; detailed callers supply one
///   `Some` or `None` slot per entry.
///
/// # Returns
///
/// `Ok(())` after output is emitted and all present reports have no broken or unavailable items.
///
/// # Errors
///
/// Returns an error when JSON serialization fails or aggregate validation requires a nonzero CLI
/// outcome.
fn render_entry_list(
    entries: Vec<ContainerEntryInfo>,
    options: &ListOptions,
    validations: Vec<Option<LinkValidationReport>>,
) -> Result<()> {
    if options.verbose == 0 && !options.link_info {
        let keys = entries
            .into_iter()
            .map(|entry| entry.key)
            .collect::<Vec<_>>();
        if options.json {
            println!("{}", serde_json::to_string_pretty(&keys)?);
        } else {
            for key in keys {
                println!("{key}");
            }
        }
        return Ok(());
    }

    if options.json {
        let values = entries
            .iter()
            .zip(&validations)
            .map(|(entry, validation)| entry_json(entry, options, validation.as_ref()))
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&values)?);
    } else {
        for (entry, validation) in entries.iter().zip(&validations) {
            println!("{}", entry_text(entry, options, validation.as_ref()));
        }
    }
    ensure_validation_success(validations.iter().flatten())
}

/// Produces one optional validation report for every listed entry.
///
/// # Arguments
///
/// * `path` - Current container root to open and lock while snapshots are validated.
/// * `entries` - Ordered entry records whose keys determine validation subjects.
/// * `validate_with` - Explicit corresponding container locators. When empty, only outgoing entries
///   are validated against their recorded targets; when nonempty, every entry is checked against
///   all supplied peers.
///
/// # Returns
///
/// A vector aligned with `entries`: `Some(report)` for each validated key and `None` for an ordinary
/// key skipped during recorded-target-only validation.
///
/// # Errors
///
/// Returns an error when current or corresponding containers cannot be opened or locked, snapshots
/// cannot be read, or the library rejects or fails the validation run.
fn validate_entries(
    path: &Path,
    entries: &[ContainerEntryInfo],
    validate_with: &[ContainerLocator],
) -> Result<Vec<Option<LinkValidationReport>>> {
    if validate_with.is_empty() {
        if !entries
            .iter()
            .any(|entry| matches!(entry.link, LinkInfo::LinkTo { .. }))
        {
            return Ok((0..entries.len()).map(|_| None).collect());
        }
        let container = open_container(path)?;
        let guard = container.writer()?;
        return entries
            .iter()
            .map(|entry| {
                if matches!(entry.link, LinkInfo::LinkTo { .. }) {
                    guard
                        .validate_recorded_links(&entry.key)
                        .map(Some)
                        .map_err(Into::into)
                } else {
                    Ok(None)
                }
            })
            .collect();
    }
    let container = open_container(path)?;
    let guard = container.writer()?;
    let corresponding = open_corresponding(validate_with)?;
    let refs = corresponding
        .iter()
        .map(|container| container.as_ref())
        .collect::<Vec<_>>();
    entries
        .iter()
        .map(|entry| {
            guard
                .validate_links(&entry.key, &refs)
                .map(Some)
                .map_err(Into::into)
        })
        .collect()
}
