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

pub(crate) fn print_container_list(path: &Path, options: ListOptions) -> Result<()> {
    print_list(path, options, || open_container(path)?.list_info())
}

pub(crate) fn print_link_list(path: &Path, options: ListOptions) -> Result<()> {
    print_list(path, options, || open_link(path)?.link_list_info())
}

pub(crate) fn print_local_list(path: &Path, options: ListOptions) -> Result<()> {
    print_list(path, options, || open_link(path)?.local_list_info())
}

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

fn entry_kind(link: &LinkInfo) -> &'static str {
    match link {
        LinkInfo::None => "local",
        LinkInfo::LinkTo { .. } => "link-to",
        LinkInfo::LinkFrom { .. } => "local-linked-from",
    }
}

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
