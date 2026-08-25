//! Container fixture-generation command dispatch and result rendering.

use anyhow::Result;
use kako_craft_lib::container::testing::{
    BrokenContainersFixture, BrokenFixtureErrorKind, create_broken_containers,
};

use crate::cli::{ContainerTestsCommand, ContainerTestsOperation};

use super::render::validation_issue_kind;

/// Executes one nested container fixture-generation operation.
///
/// # Arguments
///
/// * `command` - Parsed fixture operation and its destination arguments.
///
/// # Returns
///
/// `Ok(())` after fixtures are generated, verified through the library, and summarized on stdout.
///
/// # Errors
///
/// Returns an error when the library refuses the destination, fixture creation or corruption fails,
/// or the generated state does not reproduce its expected validator output.
pub(crate) fn run_container_tests(command: ContainerTestsCommand) -> Result<()> {
    match command.operation {
        ContainerTestsOperation::NewBroken(args) => {
            let fixture = create_broken_containers(args.path)?;
            print_broken_fixture(&fixture);
            Ok(())
        }
    }
}

/// Prints created paths, per-case actual errors, aggregate coverage, and unsupported categories.
///
/// # Arguments
///
/// * `fixture` - Verified library fixture result to render without reopening or reclassifying it.
///
/// # Returns
///
/// Returns after emitting a deterministic human-readable summary to stdout.
fn print_broken_fixture(fixture: &BrokenContainersFixture) {
    println!("created: {}", fixture.root.display());
    println!("containers: {}", fixture.containers.len());
    println!("cases: {}", fixture.cases.len());
    for case in &fixture.cases {
        println!("case: {}", case.name);
        println!("  current: {}", case.current_container.display());
        println!("  key: {}", case.key);
        for corresponding in &case.corresponding_containers {
            println!("  corresponding: {}", corresponding.display());
        }
        for error in &case.errors {
            println!("  error: {}", broken_error_kind(*error));
        }
    }

    let mut covered = Vec::new();
    for case in &fixture.cases {
        for error in &case.errors {
            let name = broken_error_kind(*error);
            if !covered.contains(&name) {
                covered.push(name);
            }
        }
    }
    println!("covered error types: {}", covered.len());
    for error in covered {
        println!("  {error}");
    }
    println!(
        "uncovered validation types: {}",
        fixture.uncovered_validation_kinds.len()
    );
    for kind in &fixture.uncovered_validation_kinds {
        println!("  validation.{}", validation_issue_kind(*kind));
    }
}

/// Maps one verified fixture error to its stable dotted CLI name.
///
/// # Arguments
///
/// * `kind` - Validation, filesystem-check, or unexpected availability category to name.
///
/// # Returns
///
/// An owned dotted identifier prefixed with `validation.`, `check.`, or `availability.`.
fn broken_error_kind(kind: BrokenFixtureErrorKind) -> String {
    match kind {
        BrokenFixtureErrorKind::Validation(kind) => {
            format!("validation.{}", validation_issue_kind(kind))
        }
        BrokenFixtureErrorKind::MissingSymlink => "check.missing_symlink".to_owned(),
        BrokenFixtureErrorKind::IncorrectSymlink => "check.incorrect_symlink".to_owned(),
        BrokenFixtureErrorKind::UnrecordedSymlink => "check.unrecorded_symlink".to_owned(),
        BrokenFixtureErrorKind::Unavailable => "availability.unavailable".to_owned(),
    }
}
