//! Read-only Destination metadata and catalog browsing commands.

use anyhow::Result;
use kako_craft_lib::destination::{DestinationMember, DestinationMetadata, open_destination};
use kako_craft_lib::locator::resolve_subcontainer;
use serde_json::json;

use crate::cli::{DestinationCommand, DestinationListArgs, DestinationOperation};

/// Executes one parsed Destination operation.
///
/// # Arguments
///
/// * `command` - Destination root and read-only operation.
///
/// # Returns
///
/// `Ok(())` after metadata or member output is emitted.
///
/// # Errors
///
/// Returns an error when Destination metadata/provider opening, Subcontainer
/// resolution, member enumeration, or JSON serialization fails.
pub(crate) fn run(command: DestinationCommand) -> Result<()> {
    match command.operation {
        DestinationOperation::Info(format) => print_info(&command.path, format.json),
        DestinationOperation::List(args) => print_members(&command.path, args, MemberFilter::All),
        DestinationOperation::Containers(args) => {
            print_members(&command.path, args, MemberFilter::Containers)
        }
        DestinationOperation::Subcontainers(args) => {
            print_members(&command.path, args, MemberFilter::Subcontainers)
        }
    }
}

/// Member subset selected by one Destination list command.
#[derive(Debug, Clone, Copy)]
enum MemberFilter {
    /// Container and Subcontainer descriptors.
    All,
    /// Container descriptors only.
    Containers,
    /// Subcontainer descriptors only.
    Subcontainers,
}

/// Prints common Destination metadata.
///
/// # Arguments
///
/// * `path` - Destination filesystem root.
/// * `as_json` - Whether to emit one JSON object instead of text fields.
///
/// # Returns
///
/// `Ok(())` after output.
///
/// # Errors
///
/// Returns an error when metadata/provider opening or JSON serialization fails.
fn print_info(path: &std::path::Path, as_json: bool) -> Result<()> {
    let metadata = DestinationMetadata::load(path)?;
    let destination = open_destination(path.to_owned())?;
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "path": destination.root_path(),
                "version": metadata.version,
                "kind": destination.kind(),
            }))?
        );
    } else {
        println!("path: {}", destination.root_path().display());
        println!("version: {}", metadata.version);
        println!("kind: {}", destination.kind());
    }
    Ok(())
}

/// Resolves one catalog node and renders the requested direct members.
///
/// # Arguments
///
/// * `path` - Destination filesystem root.
/// * `args` - Optional Subcontainer path and detail/output controls.
/// * `filter` - Combined, Container-only, or Subcontainer-only selection.
///
/// # Returns
///
/// `Ok(())` after stable member output.
///
/// # Errors
///
/// Returns an error for Destination/Subcontainer opening, enumeration, or JSON serialization.
fn print_members(
    path: &std::path::Path,
    args: DestinationListArgs,
    filter: MemberFilter,
) -> Result<()> {
    let destination = open_destination(path.to_owned())?;
    let node = resolve_subcontainer(destination.as_ref(), args.subcontainer.as_ref())?;
    let mut members = Vec::new();
    if matches!(filter, MemberFilter::All | MemberFilter::Containers) {
        members.extend(
            node.containers()?
                .into_iter()
                .map(DestinationMember::Container),
        );
    }
    if matches!(filter, MemberFilter::All | MemberFilter::Subcontainers) {
        members.extend(
            node.subcontainers()?
                .into_iter()
                .map(DestinationMember::Subcontainer),
        );
    }
    members.sort_by(|left, right| member_path(left).cmp(member_path(right)));
    if args.json {
        println!("{}", serde_json::to_string_pretty(&members)?);
    } else {
        for member in members {
            match member {
                DestinationMember::Container(container) if args.verbose >= 2 => println!(
                    "{}\ttype=container\tkind={}\tinitialized={}\tpath={}",
                    container.logical_path,
                    container.kind,
                    container.initialized,
                    container.filesystem_path.display()
                ),
                DestinationMember::Container(container) if args.verbose >= 1 => println!(
                    "{}\ttype=container\tkind={}",
                    container.logical_path, container.kind
                ),
                DestinationMember::Container(container) => {
                    println!("{}\ttype=container", container.logical_path)
                }
                DestinationMember::Subcontainer(subcontainer) => {
                    println!("{}/\ttype=subcontainer", subcontainer.logical_path)
                }
            }
        }
    }
    Ok(())
}

/// Returns a tagged member's logical sort key.
///
/// # Arguments
///
/// * `member` - Container or Subcontainer descriptor.
///
/// # Returns
///
/// Its borrowed logical path.
fn member_path(member: &DestinationMember) -> &str {
    match member {
        DestinationMember::Container(member) => &member.logical_path,
        DestinationMember::Subcontainer(member) => &member.logical_path,
    }
}
