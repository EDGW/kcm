//! Explicit ordinary-entry operations for link-capable Containers.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use kako_craft_lib::container::{ContainerMetadata, ContainerWriteGuard, EntryKey};

use crate::cli::{DataArgs, TwoKeys};

use super::io::{copy, input_data, remove, rename};
use super::lifecycle::{open_configurable, open_link};

/// Adds or writes a forced local entry.
///
/// # Arguments
///
/// * `path` - Link/configurable Container root.
/// * `args` - Entry key and complete-data input selection.
/// * `add_only` - `true` to reject an existing key; `false` to replace local data.
///
/// # Returns
///
/// `Ok(())` after the library's forced-local mutation completes.
///
/// # Errors
///
/// Returns an error for input, kind, lock, entry-state, or persistence failures.
pub(super) fn local_put(path: &Path, args: DataArgs, add_only: bool) -> Result<()> {
    let data = input_data(&args)?;
    match ContainerMetadata::load(path)?.kind.as_str() {
        "link" => {
            let mut writer = open_link(path)?.writer()?;
            if add_only {
                writer.add(&args.entry, data)?;
            } else {
                writer.write(&args.entry, data)?;
            }
        }
        "configurable" => {
            let mut writer = open_configurable(path)?.writer()?;
            if add_only {
                writer.local_add(&args.entry, data)?;
            } else {
                writer.local_write(&args.entry, data)?;
            }
        }
        kind => bail!("local-* requires a link or configurable container, found '{kind}'"),
    }
    Ok(())
}

/// Reads one explicit local entry.
///
/// # Arguments
///
/// * `path` - Link/configurable Container root.
/// * `key` - Ordinary local entry key.
///
/// # Returns
///
/// A newly allocated complete byte buffer.
///
/// # Errors
///
/// Returns an error for kind mismatch, outgoing/invalid keys, locking, or reading failures.
pub(super) fn local_read(path: &Path, key: &EntryKey) -> Result<Vec<u8>> {
    match ContainerMetadata::load(path)?.kind.as_str() {
        "link" => open_link(path)?.local_read(key),
        "configurable" => open_configurable(path)?.local_read(key),
        kind => bail!("local-read requires a link or configurable container, found '{kind}'"),
    }
}

/// Resolves one explicit local entry path.
///
/// # Arguments
///
/// * `path` - Link/configurable Container root.
/// * `key` - Ordinary local entry key.
///
/// # Returns
///
/// The local filesystem path below the Container root.
///
/// # Errors
///
/// Returns an error for kind mismatch, invalid/outgoing keys, or metadata/lock failures.
pub(super) fn local_filepath(path: &Path, key: &EntryKey) -> Result<PathBuf> {
    match ContainerMetadata::load(path)?.kind.as_str() {
        "link" => open_link(path)?.local_filepath(key),
        "configurable" => open_configurable(path)?.local_filepath(key),
        kind => bail!("local-path requires a link or configurable container, found '{kind}'"),
    }
}

/// Removes explicit local entries under one source writer.
///
/// # Arguments
///
/// * `path` - Link/configurable Container root.
/// * `entries` - Ordinary local keys to remove.
///
/// # Returns
///
/// `Ok(())` after every selected local entry is removed.
///
/// # Errors
///
/// Returns an error for kind mismatch, lock failure, absent/protected keys, or persistence failure.
pub(super) fn local_remove(path: &Path, entries: Vec<EntryKey>) -> Result<()> {
    match ContainerMetadata::load(path)?.kind.as_str() {
        "link" => remove(&open_link(path)?, entries),
        "configurable" => {
            let mut writer = open_configurable(path)?.writer()?;
            for entry in entries {
                writer.local_remove(&entry)?;
            }
            Ok(())
        }
        kind => bail!("local-remove requires a link or configurable container, found '{kind}'"),
    }
}

/// Renames one explicit local entry.
///
/// # Arguments
///
/// * `path` - Link/configurable Container root.
/// * `args` - Existing source and unoccupied destination keys.
///
/// # Returns
///
/// `Ok(())` after forced-local rename.
///
/// # Errors
///
/// Returns an error for kind, lock, key-state, protection, or filesystem failures.
pub(super) fn local_rename(path: &Path, args: TwoKeys) -> Result<()> {
    match ContainerMetadata::load(path)?.kind.as_str() {
        "link" => rename(&open_link(path)?, args),
        "configurable" => {
            open_configurable(path)?
                .writer()?
                .local_rename(&args.from, &args.to)?;
            Ok(())
        }
        kind => bail!("local-rename requires a link or configurable container, found '{kind}'"),
    }
}

/// Copies one explicit local entry.
///
/// # Arguments
///
/// * `path` - Link/configurable Container root.
/// * `args` - Existing source and unoccupied destination keys.
///
/// # Returns
///
/// `Ok(())` after forced-local copy.
///
/// # Errors
///
/// Returns an error for kind, lock, key-state, protection, or filesystem failures.
pub(super) fn local_copy(path: &Path, args: TwoKeys) -> Result<()> {
    match ContainerMetadata::load(path)?.kind.as_str() {
        "link" => copy(&open_link(path)?, args),
        "configurable" => {
            open_configurable(path)?
                .writer()?
                .local_copy(&args.from, &args.to)?;
            Ok(())
        }
        kind => bail!("local-copy requires a link or configurable container, found '{kind}'"),
    }
}
