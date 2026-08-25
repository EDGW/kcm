//! Top-level command-family dispatch.

pub(crate) mod container;
mod destination;

use anyhow::Result;

use crate::cli::{Cli, Command};

/// Dispatches one parsed top-level invocation.
///
/// # Arguments
///
/// * `cli` - Fully parsed global options and selected command family.
///
/// # Returns
///
/// `Ok(())` after the selected command completes.
///
/// # Errors
///
/// Returns any library, input/output, interaction, or rendering error from the command family.
pub(crate) fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Container(command) => container::run(command, cli.yes),
        Command::Destination(command) => destination::run(command),
    }
}
