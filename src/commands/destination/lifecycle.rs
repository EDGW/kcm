//! Destination initialization delegated to concrete library providers.

use std::path::Path;

use anyhow::Result;
use kako_craft_lib::destination::minecraft::McDestination;

use crate::cli::{DestinationInitArgs, DestinationKind};

/// Initializes a concrete Destination and prints its common metadata.
///
/// # Arguments
///
/// * `path` - Filesystem root to create or validate as a Destination.
/// * `args` - Concrete provider kind selected by the user.
///
/// # Returns
///
/// `Ok(())` after library initialization and metadata output complete.
///
/// # Errors
///
/// Returns an error from the selected library provider or common Destination
/// metadata rendering.
pub(super) fn initialize(path: &Path, args: DestinationInitArgs) -> Result<()> {
    match args.kind {
        DestinationKind::Minecraft => {
            McDestination::new(path)?;
        }
    }
    super::print_info(path, false)
}
