//! Container initialization, concrete link-capable opening, and warning interaction.

use std::io;
use std::path::Path;

use anyhow::{Result, bail};
use kako_craft_lib::container::{
    ConfigurableContainer, ContainerMetadata, EntryKey, LinkContainer, StorageClass,
};
use kako_craft_lib::locator::{ContainerLocator, initialize_container};

use crate::cli::{ContainerKind, InitArgs};

use super::render::print_info;

/// Initializes the selected concrete Container and prints its metadata.
///
/// # Arguments
///
/// * `locator` - Destination/FakeDestination Container locator.
/// * `pwd` - Working directory used by locator resolution.
/// * `args` - Kind, optional logical name, and absolute-path preference.
///
/// # Returns
///
/// `Ok(())` after initialization, path-policy persistence, and output.
///
/// # Errors
///
/// Returns an error for locator, initialization, writer, metadata, or rendering failures.
pub(super) fn initialize(locator: &ContainerLocator, pwd: &Path, args: InitArgs) -> Result<()> {
    let kind = match args.kind {
        ContainerKind::Local => "local",
        ContainerKind::Link => "link",
        ContainerKind::Configurable => "configurable",
    };
    let container = initialize_container(locator, pwd, kind, args.name.as_deref())?;
    match args.kind {
        ContainerKind::Local => {}
        ContainerKind::Link => {
            LinkContainer::from_metadata(container.root_path(), container.metadata())?
                .writer()?
                .set_prefer_relative(!args.absolute)?
        }
        ContainerKind::Configurable => {
            ConfigurableContainer::from_metadata(container.root_path(), container.metadata())?
                .writer()?
                .set_prefer_relative(!args.absolute)?
        }
    }
    print_info(&container.root_path(), false)
}

/// Opens a root only when common metadata declares kind `configurable`.
///
/// # Arguments
///
/// * `path` - Existing Container root.
///
/// # Returns
///
/// A configurable handle constructed from authoritative metadata.
///
/// # Errors
///
/// Returns an error for metadata access, kind mismatch, or invalid storage.
pub(super) fn open_configurable(path: &Path) -> Result<ConfigurableContainer> {
    let metadata = ContainerMetadata::load(path)?;
    if metadata.kind != "configurable" {
        bail!(
            "operation requires a configurable container, but {} is '{}'",
            path.display(),
            metadata.kind
        );
    }
    ConfigurableContainer::from_metadata(path, metadata)
}

/// Opens a root only when common metadata declares kind `link`.
///
/// # Arguments
///
/// * `path` - Existing Container root.
///
/// # Returns
///
/// A link handle constructed from authoritative metadata.
///
/// # Errors
///
/// Returns an error for metadata access, kind mismatch, or invalid storage.
pub(super) fn open_link(path: &Path) -> Result<LinkContainer> {
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

/// Runs one warning preflight for forced configurable storage.
///
/// # Arguments
///
/// * `path` - Current Container root.
/// * `key` - Key affected by a forced local/link command.
/// * `forced` - Explicit storage class.
/// * `yes` - Whether to accept without terminal input.
///
/// # Returns
///
/// `Ok(())` when no warning is required or it is accepted.
///
/// # Errors
///
/// Returns an error for rule loading, input failure, or user rejection.
pub(super) fn warn_forced(
    path: &Path,
    key: &EntryKey,
    forced: StorageClass,
    yes: bool,
) -> Result<()> {
    let metadata = ContainerMetadata::load(path)?;
    if metadata.kind != "configurable" {
        return Ok(());
    }
    let configurable = ConfigurableContainer::from_metadata(path, metadata)?;
    let Some(warning) = configurable.forced_storage_warning(key, forced)? else {
        return Ok(());
    };
    if yes {
        return Ok(());
    }
    eprintln!(
        "warning: '{}' is forced to {:?}; its next ordinary write/update will use {:?}. Continue? [y/N]",
        warning.key,
        warning.forced,
        warning.automatic.storage_class()
    );
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        bail!("operation cancelled")
    }
}
