//! Locator-based opening of peer Containers used by validation.

use anyhow::{Context, Result};
use kako_craft_lib::container::Container;
use kako_craft_lib::locator::{ContainerLocator, resolve_container};

/// Opens all explicitly supplied corresponding containers in argument order.
///
/// # Arguments
///
/// * `locators` - Container locators to open as validation peers; duplicates remain available for
///   the library's UID/path validation rules.
///
/// # Returns
///
/// One boxed concrete container per input locator, preserving order.
///
/// # Errors
///
/// Returns an error identifying the first locator whose Destination, metadata, or concrete
/// implementation cannot be opened.
pub(crate) fn open_corresponding(locators: &[ContainerLocator]) -> Result<Vec<Box<dyn Container>>> {
    let pwd = std::env::current_dir()?;
    locators
        .iter()
        .map(|locator| {
            resolve_container(locator, &pwd)
                .with_context(|| format!("failed to open corresponding container {locator}"))
        })
        .collect()
}
