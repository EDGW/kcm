//! Human-readable and JSON rendering for container metadata, lists, and link information.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use kako_craft_lib::container::{
    Container, ContainerEntryInfo, ContainerMetadata, LinkContainer, LinkInfo,
    LinkValidationReport, open_container,
};
use serde_json::json;

use crate::cli::{LinkInfoArgs, ListOptions};

mod validation;

use validation::{ensure_validation_success, print_validation_report, validation_status};
pub(crate) use validation::{validation_issue_kind, validation_json};

use super::interaction::{run_check_interaction, validations_need_fix, with_auto_fix};
use super::open_link;

/// Loads and prints common container metadata plus a link container's path preference.
///
/// # Arguments
///
/// * `path` - Container root whose authoritative metadata is loaded.
/// * `as_json` - When `true`, emit one pretty JSON object; when `false`, emit labeled text lines.
///
/// # Returns
///
/// `Ok(())` after metadata and any link-specific `prefer_relative` value are printed.
///
/// # Errors
///
/// Returns an error when metadata cannot be loaded, a declared link container cannot be opened or
/// locked, its outgoing metadata cannot be read, or JSON serialization fails.
pub(crate) fn print_info(path: &Path, as_json: bool) -> Result<()> {
    let metadata = ContainerMetadata::load(path)?;
    let prefer_relative = if metadata.kind == "link" {
        Some(LinkContainer::from_metadata(path, metadata.clone())?.prefer_relative()?)
    } else {
        None
    };
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "path": path,
                "version": metadata.version,
                "uid": metadata.uid,
                "logical_name": metadata.logical_name,
                "kind": metadata.kind,
                "prefer_relative": prefer_relative,
            }))?
        );
    } else {
        println!("path: {}", path.display());
        println!("version: {}", metadata.version);
        println!("uid: {}", metadata.uid);
        println!("name: {}", metadata.logical_name);
        println!("kind: {}", metadata.kind);
        if let Some(prefer_relative) = prefer_relative {
            println!("prefer_relative: {prefer_relative}");
        }
    }
    Ok(())
}

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

/// Lists only outgoing-link entries from a link container.
///
/// # Arguments
///
/// * `path` - Root required to declare concrete kind `link`.
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
    print_list(path, options, || open_link(path)?.link_list_info())
}

/// Lists only ordinary local entries from a link container.
///
/// # Arguments
///
/// * `path` - Root required to declare concrete kind `link`.
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
    print_list(path, options, || open_link(path)?.local_list_info())
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
/// * `validate_with` - Explicit corresponding container roots. When empty, only outgoing entries
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
    validate_with: &[PathBuf],
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

/// Opens all explicitly supplied corresponding containers in argument order.
///
/// # Arguments
///
/// * `paths` - Container roots to open as validation peers; duplicates remain available for the
///   library's UID/path validation rules.
///
/// # Returns
///
/// One boxed concrete container per input path, preserving order.
///
/// # Errors
///
/// Returns an error identifying the first path whose metadata or concrete implementation cannot be
/// opened.
pub(crate) fn open_corresponding(paths: &[PathBuf]) -> Result<Vec<Box<dyn Container>>> {
    paths
        .iter()
        .map(|path| {
            open_container(path).with_context(|| {
                format!("failed to open corresponding container {}", path.display())
            })
        })
        .collect()
}

/// Maps link metadata to the stable entry-type label used by list output.
///
/// # Arguments
///
/// * `link` - Entry link classification returned by the library.
///
/// # Returns
///
/// `local`, `link-to`, or `local-linked-from` according to the metadata direction.
fn entry_kind(link: &LinkInfo) -> &'static str {
    match link {
        LinkInfo::None => "local",
        LinkInfo::LinkTo { .. } => "link-to",
        LinkInfo::LinkFrom { .. } => "local-linked-from",
    }
}

/// Formats one detailed list entry as a tab-separated human-readable row.
///
/// # Arguments
///
/// * `entry` - Entry key, filesystem data, and link metadata to render.
/// * `options` - Verbosity and link-info controls determining included fields.
/// * `validation` - Optional report used only when link-info output is enabled.
///
/// # Returns
///
/// An owned row containing the key and type, optionally link identity and validation, size at
/// verbosity one, and paths plus full source or target UIDs at verbosity two or above.
fn entry_text(
    entry: &ContainerEntryInfo,
    options: &ListOptions,
    validation: Option<&LinkValidationReport>,
) -> String {
    let detail = options.verbose;
    let summary = match (&entry.link, options.link_info) {
        (LinkInfo::None, true) => format!("{}\ttype=local\tlink=none", entry.key),
        (LinkInfo::None, false) => format!("{}\ttype=local", entry.key),
        (
            LinkInfo::LinkTo {
                target_key,
                container_path,
                ..
            },
            true,
        ) => format!(
            "{}\ttype=link-to\ttarget={}:{}",
            entry.key,
            container_path.display(),
            target_key
        ),
        (LinkInfo::LinkTo { .. }, false) => format!("{}\ttype=link-to", entry.key),
        (LinkInfo::LinkFrom { linkers }, true) => format!(
            "{}\ttype=local-linked-from\tsources={}",
            entry.key,
            linkers.len()
        ),
        (LinkInfo::LinkFrom { .. }, false) => {
            format!("{}\ttype=local-linked-from", entry.key)
        }
    };
    let summary = if options.link_info {
        format!(
            "{summary}\tvalidation={}",
            validation.map_or("unverified", validation_status)
        )
    } else {
        summary
    };
    if detail == 0 {
        return summary;
    }

    let size = entry
        .size
        .map_or_else(|| "unknown".to_owned(), |size| size.to_string());
    let mut full = format!("{summary}\tsize={size}");
    if detail < 2 {
        return full;
    }
    full.push_str(&format!("\tfilepath={}", entry.filepath.display()));
    match (&entry.link, options.link_info) {
        (LinkInfo::LinkTo { container_uid, .. }, true) => {
            full.push_str(&format!("\ttarget_uid={container_uid}"));
        }
        (LinkInfo::LinkFrom { linkers }, true) => {
            for (index, linker) in linkers.iter().enumerate() {
                full.push_str(&format!(
                    "\tsource[{index}]={}:{}",
                    linker.linker_uid, linker.linker_key
                ));
            }
        }
        _ => {}
    }
    full
}

/// Serializes one list entry according to verbosity and link-info controls.
///
/// # Arguments
///
/// * `entry` - Entry key, filesystem data, and link metadata to encode.
/// * `options` - Verbosity and link-info controls determining emitted object fields.
/// * `validation` - Optional report encoded when link-info output is enabled; absence becomes an
///   explicit `unverified` validation object.
///
/// # Returns
///
/// A JSON object with stable key and type fields, optional target/source metadata, optional size and
/// filepath detail, and optional validation.
pub(crate) fn entry_json(
    entry: &ContainerEntryInfo,
    options: &ListOptions,
    validation: Option<&LinkValidationReport>,
) -> serde_json::Value {
    let detail = options.verbose;
    let mut value = json!({
        "key": entry.key,
        "type": entry_kind(&entry.link),
    });
    let object = value.as_object_mut().expect("entry JSON is an object");
    match (&entry.link, options.link_info) {
        (
            LinkInfo::LinkTo {
                target_key,
                container_path,
                container_uid,
            },
            true,
        ) => {
            object.insert("target_key".into(), json!(target_key));
            object.insert("container_path".into(), json!(container_path));
            if detail >= 2 {
                object.insert("container_uid".into(), json!(container_uid));
            }
        }
        (LinkInfo::LinkFrom { linkers }, true) => {
            if detail >= 2 {
                object.insert(
                    "sources".into(),
                    json!(
                        linkers
                            .iter()
                            .map(|linker| json!({
                                "container_uid": linker.linker_uid,
                                "linker_key": linker.linker_key,
                            }))
                            .collect::<Vec<_>>()
                    ),
                );
            } else {
                object.insert("source_count".into(), json!(linkers.len()));
            }
        }
        _ => {}
    }
    if detail >= 1 {
        object.insert("size".into(), json!(entry.size));
    }
    if detail >= 2 {
        object.insert("filepath".into(), json!(entry.filepath));
    }
    if options.link_info {
        object.insert(
            "validation".into(),
            validation.map_or_else(|| json!({ "status": "unverified" }), validation_json),
        );
    }
    value
}

/// Prints one entry's incoming or outgoing link metadata and optional reciprocal validation.
///
/// # Arguments
///
/// * `path` - Root of the current container containing the selected entry.
/// * `args` - Entry key, corresponding peer paths, output format, and auto-fix policy.
///
/// # Returns
///
/// `Ok(())` after JSON or human output is emitted and any requested validation is successful.
///
/// # Errors
///
/// Returns an error for container access, link-info or validation failure, interactive repair or
/// reload failure, JSON serialization, or a broken or unavailable final validation result.
pub(crate) fn link_info(path: &Path, args: LinkInfoArgs) -> Result<()> {
    let mut result = collect_link_info(path, &args)?;
    if args.auto_fix
        && result
            .1
            .as_ref()
            .is_some_and(|validation| !validation.is_valid())
    {
        run_check_interaction(path, &args.validate_with)?;
        result = collect_link_info(path, &args)
            .context("failed to reload link info after interactive link check")?;
    }
    let (info, validation) = result;

    if args.json {
        let value = json!({
            "link_info": link_info_json(&info),
            "validation": validation.as_ref().map(validation_json),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        match &info {
            LinkInfo::None => println!("not linked"),
            LinkInfo::LinkTo {
                target_key,
                container_uid,
                container_path,
            } => {
                println!("direction: to");
                println!("container_uid: {container_uid}");
                println!("container_path: {}", container_path.display());
                println!("target_key: {target_key}");
            }
            LinkInfo::LinkFrom { linkers } => {
                println!("direction: from");
                for (index, linker) in linkers.iter().enumerate() {
                    println!("source[{index}].container_uid: {}", linker.linker_uid);
                    println!("source[{index}].linker_key: {}", linker.linker_key);
                }
            }
        }
        if let Some(report) = &validation {
            print_validation_report(report);
        } else {
            println!("validation: unverified");
        }
    }
    ensure_validation_success(validation.iter())
}

/// Collects link metadata and, when peers are supplied, validates it under one current writer.
///
/// # Arguments
///
/// * `path` - Current container root to open and exclusively lock.
/// * `args` - Entry key and corresponding peer roots; rendering and auto-fix flags are ignored here.
///
/// # Returns
///
/// The selected entry's link classification and `Some(report)` when `validate_with` is nonempty, or
/// `None` when validation was not requested.
///
/// # Errors
///
/// Returns an error when the current container or peers cannot be opened, the current writer cannot
/// be acquired, link metadata cannot be read, or validation fails.
fn collect_link_info(
    path: &Path,
    args: &LinkInfoArgs,
) -> Result<(LinkInfo, Option<LinkValidationReport>)> {
    let container = open_container(path)?;
    let guard = container.writer()?;
    let info = guard.link_info(&args.entry)?;
    let validation = if args.validate_with.is_empty() {
        None
    } else {
        let corresponding = open_corresponding(&args.validate_with)?;
        let refs = corresponding
            .iter()
            .map(|container| container.as_ref())
            .collect::<Vec<_>>();
        Some(guard.validate_links(&args.entry, &refs)?)
    };
    Ok((info, validation))
}

/// Serializes an entry's directional link metadata independently of validation.
///
/// # Arguments
///
/// * `info` - None, outgoing target identity, or all incoming source identities.
///
/// # Returns
///
/// A JSON object tagged with `kind`: `none`, `to` with target fields, or `from` with every linker
/// record in library-provided order.
fn link_info_json(info: &LinkInfo) -> serde_json::Value {
    match info {
        LinkInfo::None => json!({ "kind": "none" }),
        LinkInfo::LinkTo {
            target_key,
            container_uid,
            container_path,
        } => json!({
            "kind": "to",
            "target_key": target_key,
            "container_uid": container_uid,
            "container_path": container_path,
        }),
        LinkInfo::LinkFrom { linkers } => json!({
            "kind": "from",
            "linkers": linkers.iter().map(|linker| json!({
                "linker_key": linker.linker_key,
                "container_uid": linker.linker_uid,
            })).collect::<Vec<_>>(),
        }),
    }
}
