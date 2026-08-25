//! Container metadata rendering.

use std::path::Path;

use anyhow::Result;
use kako_craft_lib::container::{ConfigurableContainer, ContainerMetadata, LinkContainer};
use serde_json::json;

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
    let prefer_relative = match metadata.kind.as_str() {
        "link" => Some(LinkContainer::from_metadata(path, metadata.clone())?.prefer_relative()?),
        "configurable" => {
            Some(ConfigurableContainer::from_metadata(path, metadata.clone())?.prefer_relative()?)
        }
        _ => None,
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
