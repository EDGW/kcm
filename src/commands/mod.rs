//! Command dispatch and library-call orchestration for the container command family.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use kako_craft_lib::container::{
    Container, ContainerMetadata, EntryKey, LinkContainer, LocalContainer, open_container,
};

use crate::cli::*;

pub(crate) mod interaction;
pub(crate) mod render;
pub(crate) mod tests;

use interaction::*;
use render::*;
use tests::run_container_tests;

/// Dispatches one parsed top-level invocation to its command-family handler.
///
/// # Arguments
///
/// * `cli` - Fully parsed global options and selected command family.
///
/// # Returns
///
/// `Ok(())` after the selected command completes and emits its output.
///
/// # Errors
///
/// Returns any container, validation, input/output, rendering, or interaction error produced by
/// the selected command.
pub(crate) fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Container(command) => run_container(command),
    }
}

/// Dispatches a parsed container operation while preserving its selected root path.
///
/// # Arguments
///
/// * `command` - Container root and exactly one general, local-only, or link operation.
///
/// # Returns
///
/// `Ok(())` after the operation and any requested output or interactive repair complete.
///
/// # Errors
///
/// Returns an error when opening the required container kind, executing the delegated library
/// operation, reading or writing command data, validating links, or rendering output fails.
fn run_container(command: ContainerCommand) -> Result<()> {
    let path = command.path;
    match command.operation {
        ContainerOperation::Info(format) => print_info(&path, format.json),
        ContainerOperation::List(options) => print_container_list(&path, options),
        ContainerOperation::Init(args) => initialize(&path, args),

        ContainerOperation::Add(args) => {
            let container = open_local(&path)?;
            put(&container, args, true)
        }
        ContainerOperation::Update(args) => {
            let container = open_local(&path)?;
            put(&container, args, false)
        }
        ContainerOperation::Read(args) => {
            let data = with_auto_fix(&path, args.auto_fix, || {
                open_container(&path)?.read(&args.entry)
            })?;
            output_data(data, args.output)
        }
        ContainerOperation::Path(args) => {
            let entry_path = with_auto_fix(&path, args.auto_fix, || {
                open_container(&path)?.filepath(&args.entry)
            })?;
            println!("{}", entry_path.display());
            Ok(())
        }
        ContainerOperation::Check(args) => check(&path, args),
        ContainerOperation::Remove(args) => {
            let container = open_local(&path)?;
            remove(&container, args.entries)
        }
        ContainerOperation::Rename(args) => {
            let container = open_local(&path)?;
            rename(&container, args)
        }
        ContainerOperation::Copy(args) => {
            let container = open_local(&path)?;
            copy(&container, args)
        }

        ContainerOperation::LocalList(options) => print_local_list(&path, options),
        ContainerOperation::LocalAdd(args) => {
            let container = open_link(&path)?;
            put(&container, args, true)
        }
        ContainerOperation::LocalUpdate(args) => {
            let container = open_link(&path)?;
            put(&container, args, false)
        }
        ContainerOperation::LocalRead(args) => {
            let container = open_link(&path)?;
            output_data(container.local_read(&args.entry)?, args.output)
        }
        ContainerOperation::LocalPath(args) => {
            let container = open_link(&path)?;
            println!("{}", container.local_filepath(&args.entry)?.display());
            Ok(())
        }
        ContainerOperation::LocalRemove(args) => {
            let container = open_link(&path)?;
            remove(&container, args.entries)
        }
        ContainerOperation::LocalRename(args) => {
            let container = open_link(&path)?;
            rename(&container, args)
        }
        ContainerOperation::LocalCopy(args) => {
            let container = open_link(&path)?;
            copy(&container, args)
        }

        ContainerOperation::Link(args) => link(&path, args),
        ContainerOperation::LinkList(options) => print_link_list(&path, options),
        ContainerOperation::LinkCopy(args) => link_copy(&path, args),
        ContainerOperation::LinkRename(args) => link_rename(&path, args),
        ContainerOperation::LinkRemove(args) => unlink(&path, args),
        ContainerOperation::LinkInfo(args) => link_info(&path, args),
        ContainerOperation::Tests(command) => run_container_tests(command),
    }
}

/// Initializes the selected concrete container and prints its resulting metadata.
///
/// # Arguments
///
/// * `path` - Filesystem root to create or open as a container.
/// * `args` - Requested kind, optional logical name, and link-container absolute-path default.
///
/// # Returns
///
/// `Ok(())` after initialization, link path-policy persistence when applicable, and human-readable
/// metadata output.
///
/// # Errors
///
/// Returns an error if container creation or opening, writer acquisition, preference persistence,
/// or metadata rendering fails.
fn initialize(path: &Path, args: InitArgs) -> Result<()> {
    let absolute = args.absolute;
    match (args.kind, args.name) {
        (ContainerKind::Local, Some(name)) => {
            LocalContainer::with_logical_name(path, name)?;
        }
        (ContainerKind::Local, None) => {
            LocalContainer::new(path)?;
        }
        (ContainerKind::Link, Some(name)) => {
            let container = LinkContainer::with_logical_name(path, name)?;
            container.writer()?.set_prefer_relative(!absolute)?;
        }
        (ContainerKind::Link, None) => {
            let container = LinkContainer::new(path)?;
            container.writer()?.set_prefer_relative(!absolute)?;
        }
    }
    print_info(path, false)
}

/// Opens a root only when its authoritative common metadata declares kind `local`.
///
/// # Arguments
///
/// * `path` - Existing container root whose `.kcl/container.json` is inspected.
///
/// # Returns
///
/// A local-container handle constructed from the already loaded metadata.
///
/// # Errors
///
/// Returns an error when metadata cannot be loaded, the declared kind is not `local`, or the local
/// implementation rejects the root or metadata.
fn open_local(path: &Path) -> Result<LocalContainer> {
    let metadata = ContainerMetadata::load(path)?;
    if metadata.kind != "local" {
        bail!(
            "operation requires a local container, but {} is '{}' (use a local-* operation for ordinary entries in a link container)",
            path.display(),
            metadata.kind
        );
    }
    LocalContainer::from_metadata(path, metadata)
}

/// Opens a root only when its authoritative common metadata declares kind `link`.
///
/// # Arguments
///
/// * `path` - Existing container root whose `.kcl/container.json` is inspected.
///
/// # Returns
///
/// A link-container handle constructed from the already loaded metadata.
///
/// # Errors
///
/// Returns an error when metadata cannot be loaded, the declared kind is not `link`, or the link
/// implementation rejects the root or metadata.
fn open_link(path: &Path) -> Result<LinkContainer> {
    let metadata = ContainerMetadata::load(path)?;
    if metadata.kind != "link" {
        bail!(
            "operation requires a link container, but {} is '{}'",
            path.display(),
            metadata.kind
        );
    }
    LinkContainer::from_metadata(path, metadata)
}

/// Loads complete entry bytes from the input source selected by add or update arguments.
///
/// # Arguments
///
/// * `args` - Data arguments selecting a file, inline UTF-8 content, or stdin when both options are
///   absent.
///
/// # Returns
///
/// A newly allocated byte vector containing the entire selected input.
///
/// # Errors
///
/// Returns an error when the selected file or stdin cannot be read.
fn input_data(args: &DataArgs) -> Result<Vec<u8>> {
    if let Some(path) = &args.file {
        return fs::read(path).with_context(|| format!("failed to read {}", path.display()));
    }
    if let Some(content) = &args.content {
        return Ok(content.as_bytes().to_vec());
    }
    let mut data = Vec::new();
    io::stdin()
        .read_to_end(&mut data)
        .context("failed to read entry data from stdin")?;
    Ok(data)
}

/// Adds or replaces one ordinary entry through a newly acquired container writer.
///
/// # Arguments
///
/// * `container` - Open container whose writer performs the mutation.
/// * `args` - Destination entry key and byte-input source.
/// * `add_only` - When `true`, require the key to be absent and call `add`; when `false`, call
///   `write` to create or replace it.
///
/// # Returns
///
/// `Ok(())` after all input bytes are loaded and the guarded mutation completes.
///
/// # Errors
///
/// Returns an error for input failure, writer contention, invalid or protected keys, add conflicts,
/// or entry persistence failure.
fn put(container: &dyn Container, args: DataArgs, add_only: bool) -> Result<()> {
    let data = input_data(&args)?;
    let mut writer = container.writer()?;
    if add_only {
        writer.add(&args.entry, data)?;
    } else {
        writer.write(&args.entry, data)?;
    }
    Ok(())
}

/// Writes complete entry bytes to a selected file or standard output.
///
/// # Arguments
///
/// * `data` - Owned entry bytes to emit without text conversion.
/// * `output` - Destination file path, or `None` to write raw bytes to stdout.
///
/// # Returns
///
/// `Ok(())` after the complete buffer is written.
///
/// # Errors
///
/// Returns an error when the destination file or stdout cannot be written.
fn output_data(data: Vec<u8>, output: Option<PathBuf>) -> Result<()> {
    if let Some(path) = output {
        fs::write(&path, data).with_context(|| format!("failed to write {}", path.display()))?;
    } else {
        io::stdout().write_all(&data)?;
    }
    Ok(())
}

/// Removes a batch of ordinary entries after the library's full preflight validation.
///
/// # Arguments
///
/// * `container` - Open container whose writer owns the entire batch operation.
/// * `entries` - Entry keys to preflight and remove as one guarded operation.
///
/// # Returns
///
/// `Ok(())` after every requested entry has been removed.
///
/// # Errors
///
/// Returns an error for writer contention, any invalid, absent, or link-protected key, or a
/// filesystem failure during removal.
fn remove(container: &dyn Container, entries: Vec<EntryKey>) -> Result<()> {
    let mut writer = container.writer()?;
    writer.remove_many(&entries)?;
    Ok(())
}

/// Renames one ordinary entry through an exclusive container writer.
///
/// # Arguments
///
/// * `container` - Open container owning the ordinary entry.
/// * `args` - Existing source key and unoccupied destination key.
///
/// # Returns
///
/// `Ok(())` after the guarded rename completes.
///
/// # Errors
///
/// Returns an error for writer contention, invalid, absent, occupied, or link-protected keys, or a
/// filesystem failure.
fn rename(container: &dyn Container, args: TwoKeys) -> Result<()> {
    container.writer()?.rename(&args.from, &args.to)?;
    Ok(())
}

/// Copies one ordinary entry through an exclusive container writer.
///
/// # Arguments
///
/// * `container` - Open container owning the ordinary source entry.
/// * `args` - Existing source key and unoccupied destination key.
///
/// # Returns
///
/// `Ok(())` after the destination copy is created.
///
/// # Errors
///
/// Returns an error for writer contention, invalid, absent, occupied, or outgoing-link keys, or a
/// filesystem failure.
fn copy(container: &dyn Container, args: TwoKeys) -> Result<()> {
    container.writer()?.copy(&args.from, &args.to)?;
    Ok(())
}

/// Creates an outgoing relationship and reciprocal incoming record with optional repair-and-retry.
///
/// # Arguments
///
/// * `path` - Root of the link container receiving the outgoing key.
/// * `args` - Linker key, target container and key, optional relative-path override, and auto-fix
///   policy.
///
/// # Returns
///
/// `Ok(())` after both sides of the relationship and its symlink are installed.
///
/// # Errors
///
/// Returns an error when either container cannot be opened or locked, validation or repair fails,
/// the target or linker key is invalid or conflicting, or the multi-container mutation fails.
fn link(path: &Path, args: LinkArgs) -> Result<()> {
    let preference = args.prefer_relative();
    with_auto_fix(path, args.auto_fix, || {
        let linker = open_link(path)?;
        let mut target = open_container(&args.target_container)?;
        linker.writer()?.link_to(
            &args.linker_key,
            target.as_mut(),
            &args.target_key,
            preference,
        )?;
        Ok(())
    })
}

/// Removes an outgoing relationship and reciprocal incoming record with optional repair-and-retry.
///
/// # Arguments
///
/// * `path` - Root of the link container owning the outgoing key.
/// * `args` - Outgoing linker key and auto-fix policy.
///
/// # Returns
///
/// `Ok(())` after reciprocal metadata, outgoing metadata, and the symlink are removed.
///
/// # Errors
///
/// Returns an error when the container cannot be opened or locked, validation or interactive repair
/// fails, the relationship is absent or mismatched, or removal cannot complete safely.
fn unlink(path: &Path, args: UnlinkArgs) -> Result<()> {
    with_auto_fix(path, args.auto_fix, || {
        open_link(path)?.writer()?.unlink_to(&args.linker_key)?;
        Ok(())
    })
}

/// Copies an outgoing-link definition to a new linker key with optional repair-and-retry.
///
/// # Arguments
///
/// * `path` - Root of the link container owning the source relationship.
/// * `args` - Source key, new destination key, and auto-fix policy.
///
/// # Returns
///
/// `Ok(())` after the new outgoing record, symlink, and additional reciprocal record exist.
///
/// # Errors
///
/// Returns an error for open or lock failure, invalid or occupied keys, broken source state,
/// interactive repair failure, or an incomplete multi-container mutation.
fn link_copy(path: &Path, args: LinkKeysArgs) -> Result<()> {
    with_auto_fix(path, args.auto_fix, || {
        open_link(path)?.writer()?.link_copy(&args.from, &args.to)?;
        Ok(())
    })
}

/// Renames an outgoing-link key and its reciprocal identity with optional repair-and-retry.
///
/// # Arguments
///
/// * `path` - Root of the link container owning the relationship.
/// * `args` - Existing linker key, new linker key, and auto-fix policy.
///
/// # Returns
///
/// `Ok(())` after outgoing metadata, symlink, and reciprocal incoming record use the destination
/// key.
///
/// # Errors
///
/// Returns an error for open or lock failure, invalid or occupied keys, broken source state,
/// interactive repair failure, or an incomplete multi-container mutation or rollback.
fn link_rename(path: &Path, args: LinkKeysArgs) -> Result<()> {
    with_auto_fix(path, args.auto_fix, || {
        open_link(path)?
            .writer()?
            .link_rename(&args.from, &args.to)?;
        Ok(())
    })
}
