//! Directional link metadata rendering and optional reciprocal validation.

use std::path::Path;

use anyhow::{Context, Result};
use kako_craft_lib::container::{LinkInfo, LinkValidationReport, open_container};
use serde_json::json;

use crate::cli::LinkInfoArgs;

use super::peers::open_corresponding;
use super::validation::{ensure_validation_success, print_validation_report, validation_json};
use crate::commands::container::interaction::run_check_interaction;

/// Prints one entry's incoming or outgoing link metadata and optional reciprocal validation.
///
/// # Arguments
///
/// * `path` - Root of the current container containing the selected entry.
/// * `args` - Entry key, corresponding peer locators, output format, and auto-fix policy.
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
/// * `args` - Entry key and corresponding peer locators; rendering and auto-fix flags are ignored.
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
