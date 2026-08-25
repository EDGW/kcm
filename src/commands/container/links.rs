//! Explicit outgoing-link operations for link-capable Containers.

use std::path::Path;

use anyhow::{Result, bail};
use kako_craft_lib::container::ContainerMetadata;
use kako_craft_lib::locator::resolve_container;

use crate::cli::{LinkArgs, LinkKeysArgs, UnlinkArgs};

use super::interaction::with_auto_fix;
use super::lifecycle::{open_configurable, open_link};

/// Creates one explicit outgoing relationship with optional repair-and-retry.
///
/// # Arguments
///
/// * `path` - Link/configurable source Container root.
/// * `pwd` - Working directory for the target locator.
/// * `args` - Linker/target keys, target locator, path override, and auto-fix flag.
///
/// # Returns
///
/// `Ok(())` after reciprocal metadata and symlink installation.
///
/// # Errors
///
/// Returns an error for locator, kind, locking, validation, repair, conflict, or persistence failure.
pub(super) fn link(path: &Path, pwd: &Path, args: LinkArgs) -> Result<()> {
    let preference = args.prefer_relative();
    with_auto_fix(path, args.auto_fix, || {
        let mut target = resolve_container(&args.target_container, pwd)?;
        match ContainerMetadata::load(path)?.kind.as_str() {
            "link" => open_link(path)?.writer()?.link_to(
                &args.linker_key,
                target.as_mut(),
                &args.target_key,
                preference,
            )?,
            "configurable" => open_configurable(path)?.writer()?.link_to(
                &args.linker_key,
                target.as_mut(),
                &args.target_key,
                preference,
            )?,
            kind => bail!("link requires a link or configurable container, found '{kind}'"),
        }
        Ok(())
    })
}

/// Removes one explicit outgoing relationship with optional repair-and-retry.
///
/// # Arguments
///
/// * `path` - Link/configurable source Container root.
/// * `args` - Outgoing key and auto-fix flag.
///
/// # Returns
///
/// `Ok(())` after reciprocal metadata and symlink removal.
///
/// # Errors
///
/// Returns an error for kind, lock, missing/broken relationship, repair, or persistence failure.
pub(super) fn unlink(path: &Path, args: UnlinkArgs) -> Result<()> {
    with_auto_fix(path, args.auto_fix, || {
        match ContainerMetadata::load(path)?.kind.as_str() {
            "link" => open_link(path)?.writer()?.unlink_to(&args.linker_key)?,
            "configurable" => open_configurable(path)?
                .writer()?
                .unlink_to(&args.linker_key)?,
            kind => bail!("link-remove requires a link or configurable container, found '{kind}'"),
        }
        Ok(())
    })
}

/// Copies one explicit outgoing relationship with optional repair-and-retry.
///
/// # Arguments
///
/// * `path` - Link/configurable source Container root.
/// * `args` - Existing/new outgoing keys and auto-fix flag.
///
/// # Returns
///
/// `Ok(())` after the additional reciprocal relationship exists.
///
/// # Errors
///
/// Returns an error for kind, lock, validation, repair, conflict, or persistence failure.
pub(super) fn link_copy(path: &Path, args: LinkKeysArgs) -> Result<()> {
    with_auto_fix(path, args.auto_fix, || {
        match ContainerMetadata::load(path)?.kind.as_str() {
            "link" => open_link(path)?.writer()?.link_copy(&args.from, &args.to)?,
            "configurable" => open_configurable(path)?
                .writer()?
                .link_copy(&args.from, &args.to)?,
            kind => bail!("link-copy requires a link or configurable container, found '{kind}'"),
        }
        Ok(())
    })
}

/// Renames one explicit outgoing relationship with optional repair-and-retry.
///
/// # Arguments
///
/// * `path` - Link/configurable source Container root.
/// * `args` - Existing/new outgoing keys and auto-fix flag.
///
/// # Returns
///
/// `Ok(())` after reciprocal and local outgoing identities use the new key.
///
/// # Errors
///
/// Returns an error for kind, lock, validation, repair, conflict, rollback, or persistence failure.
pub(super) fn link_rename(path: &Path, args: LinkKeysArgs) -> Result<()> {
    with_auto_fix(path, args.auto_fix, || {
        match ContainerMetadata::load(path)?.kind.as_str() {
            "link" => open_link(path)?
                .writer()?
                .link_rename(&args.from, &args.to)?,
            "configurable" => open_configurable(path)?
                .writer()?
                .link_rename(&args.from, &args.to)?,
            kind => bail!("link-rename requires a link or configurable container, found '{kind}'"),
        }
        Ok(())
    })
}
