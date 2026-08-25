//! Byte input/output and common Container CRUD wrappers.

use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use kako_craft_lib::container::{Container, EntryKey};

use crate::cli::{DataArgs, TwoKeys};

/// Loads complete entry bytes from a file, inline content, or stdin.
///
/// # Arguments
///
/// * `args` - Data arguments selecting exactly one input source.
///
/// # Returns
///
/// A newly allocated complete byte buffer.
///
/// # Errors
///
/// Returns an error when the selected file or stdin cannot be read.
pub(super) fn input_data(args: &DataArgs) -> Result<Vec<u8>> {
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

/// Adds or replaces one entry through a Container writer.
///
/// # Arguments
///
/// * `container` - Open Container whose library writer performs the mutation.
/// * `args` - Entry key and byte-input source.
/// * `add_only` - `true` to require absence; `false` to write/update.
///
/// # Returns
///
/// `Ok(())` after input and guarded mutation complete.
///
/// # Errors
///
/// Returns an error for input, contention, entry state, routing, or persistence failures.
pub(super) fn put(container: &dyn Container, args: DataArgs, add_only: bool) -> Result<()> {
    let data = input_data(&args)?;
    let mut writer = container.writer()?;
    if add_only {
        writer.add(&args.entry, data)?;
    } else {
        writer.write(&args.entry, data)?;
    }
    Ok(())
}

/// Writes complete bytes to a selected file or stdout.
///
/// # Arguments
///
/// * `data` - Owned bytes emitted without text conversion.
/// * `output` - Destination file, or `None` for stdout.
///
/// # Returns
///
/// `Ok(())` after all bytes are written.
///
/// # Errors
///
/// Returns an error when the file or stdout cannot be written.
pub(super) fn output_data(data: Vec<u8>, output: Option<PathBuf>) -> Result<()> {
    if let Some(path) = output {
        fs::write(&path, data).with_context(|| format!("failed to write {}", path.display()))?;
    } else {
        io::stdout().write_all(&data)?;
    }
    Ok(())
}

/// Removes a preflighted batch through one library writer.
///
/// # Arguments
///
/// * `container` - Open Container owning the entries.
/// * `entries` - Keys removed by the library batch operation.
///
/// # Returns
///
/// `Ok(())` after every entry is removed.
///
/// # Errors
///
/// Returns an error for contention, invalid/absent/protected keys, or persistence failures.
pub(super) fn remove(container: &dyn Container, entries: Vec<EntryKey>) -> Result<()> {
    container.writer()?.remove_many(&entries)?;
    Ok(())
}

/// Renames one entry through an exclusive library writer.
///
/// # Arguments
///
/// * `container` - Open Container owning the source.
/// * `args` - Source and unoccupied destination keys.
///
/// # Returns
///
/// `Ok(())` after rename.
///
/// # Errors
///
/// Returns an error for contention, state, routing, or persistence failures.
pub(super) fn rename(container: &dyn Container, args: TwoKeys) -> Result<()> {
    container.writer()?.rename(&args.from, &args.to)?;
    Ok(())
}

/// Copies one entry through an exclusive library writer.
///
/// # Arguments
///
/// * `container` - Open Container owning the source.
/// * `args` - Source and unoccupied destination keys.
///
/// # Returns
///
/// `Ok(())` after copy.
///
/// # Errors
///
/// Returns an error for contention, state, routing, or persistence failures.
pub(super) fn copy(container: &dyn Container, args: TwoKeys) -> Result<()> {
    container.writer()?.copy(&args.from, &args.to)?;
    Ok(())
}
