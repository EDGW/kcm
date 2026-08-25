//! Per-entry human and JSON list formatting.

use kako_craft_lib::container::{ContainerEntryInfo, LinkInfo, LinkValidationReport};
use serde_json::json;

use crate::cli::ListOptions;

use super::super::validation::{validation_json, validation_status};

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
pub(super) fn entry_text(
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
pub(super) fn entry_json(
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
