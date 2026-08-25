//! Command-line interface for inspecting and mutating `kako-craft-lib` containers.
//!
//! `kcm` parses structured container commands, delegates storage and consistency
//! operations to the library, renders human or JSON output, and maps validation
//! outcomes to stable process exit codes.

#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![cfg_attr(not(test), deny(clippy::missing_docs_in_private_items))]

use std::fmt;
use std::io;

use anyhow::Result;
use clap::Parser;
use kako_craft_lib::container::LinkValidationRunError;

mod cli;
mod commands;

use cli::*;
use commands::container::interaction::is_unavailable_link_error;
use commands::run;

/// Parses one process invocation, executes it, and enforces the CLI exit-code contract.
///
/// # Returns
///
/// Returns normally with implicit process status `0` after successful logging initialization and
/// command execution. On failure, prints the complete error chain to stderr and terminates the
/// process with status `1` for ordinary or broken-link failures, `2` for invalid validation input,
/// or `3` for temporary container unavailability.
fn main() {
    let cli = Cli::parse();
    if let Err(error) = initialize_logging(cli.log_level).and_then(|()| run(cli)) {
        eprintln!("error: {error:#}");
        let exit_code = error
            .downcast_ref::<ValidationCommandError>()
            .map(ValidationCommandError::exit_code)
            .unwrap_or_else(|| {
                if is_unavailable_link_error(&error) {
                    3
                } else if error.chain().any(|cause| {
                    matches!(
                        cause.downcast_ref::<LinkValidationRunError>(),
                        Some(
                            LinkValidationRunError::SelfValidation(_)
                                | LinkValidationRunError::DuplicateContainerUid { .. }
                        )
                    )
                }) {
                    2
                } else {
                    1
                }
            });
        std::process::exit(exit_code);
    }
}

/// Installs stderr tracing when the user explicitly selects a log level.
///
/// # Arguments
///
/// * `level` - Maximum enabled verbosity, or `None` to leave tracing uninitialized and emit no
///   library logs.
///
/// # Returns
///
/// `Ok(())` when logging is intentionally disabled or the global subscriber is installed.
///
/// # Errors
///
/// Returns an error when another global tracing subscriber has already been installed or subscriber
/// initialization otherwise fails.
fn initialize_logging(level: Option<LogLevel>) -> Result<()> {
    let Some(level) = level else {
        return Ok(());
    };
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::from(level))
        .with_writer(io::stderr)
        .try_init()
        .map_err(|error| anyhow::anyhow!("failed to initialize logging: {error}"))
}

#[derive(Debug)]
/// Validation outcome translated into the CLI's stable nonzero exit-code contract.
pub(crate) enum ValidationCommandError {
    /// One or more persistent reciprocal-link inconsistencies; the payload is the issue count.
    Broken(usize),
    /// One or more peer containers could not be validated; the payload is the container count.
    Unavailable(usize),
}

impl ValidationCommandError {
    /// Maps a validation outcome to its documented process status.
    ///
    /// # Returns
    ///
    /// Exit code `1` for persistent broken relationships or `3` for temporarily unavailable peers.
    fn exit_code(&self) -> i32 {
        match self {
            Self::Broken(_) => 1,
            Self::Unavailable(_) => 3,
        }
    }
}

impl fmt::Display for ValidationCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Broken(count) => {
                write!(
                    formatter,
                    "link validation found {count} broken relationship(s)"
                )
            }
            Self::Unavailable(count) => write!(
                formatter,
                "link validation could not access {count} container(s)"
            ),
        }
    }
}

impl std::error::Error for ValidationCommandError {}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// Extracts the parsed Container command or fails the test with its actual family.
    fn container(cli: Cli) -> ContainerCommand {
        let Command::Container(command) = cli.command else {
            panic!("expected Container command");
        };
        command
    }

    #[test]
    fn parses_locator_commands_and_rejects_legacy_paths() {
        let command = container(
            Cli::try_parse_from([
                "kcm",
                "-y",
                "container",
                ":./links",
                "local-add",
                "note.txt",
                "--content",
                "hello",
            ])
            .unwrap(),
        );
        assert_eq!(command.locator.to_string(), ":./links");
        assert!(matches!(command.operation, ContainerOperation::LocalAdd(_)));
        assert!(Cli::try_parse_from(["kcm", "container", "./legacy", "info"]).is_err());

        let command = container(
            Cli::try_parse_from([
                "kcm",
                "container",
                ":links",
                "link",
                "linked.txt",
                ":target",
                "source.txt",
                "--absolute",
            ])
            .unwrap(),
        );
        let ContainerOperation::Link(args) = command.operation else {
            panic!("expected link operation");
        };
        assert_eq!(args.target_container.to_string(), ":target");
        assert_eq!(args.prefer_relative(), Some(false));
    }

    #[test]
    fn parses_destination_and_multiple_validation_locators() {
        let cli = Cli::try_parse_from([
            "kcm",
            "destination",
            "/game",
            "list",
            "versions/",
            "-vv",
            "--json",
        ])
        .unwrap();
        let Command::Destination(command) = cli.command else {
            panic!("expected Destination command");
        };
        let DestinationOperation::List(args) = command.operation else {
            panic!("expected Destination list");
        };
        assert!(args.subcontainer.unwrap().requires_subcontainer());
        assert_eq!(args.verbose, 2);
        assert!(args.json);

        let command = container(
            Cli::try_parse_from([
                "kcm",
                "container",
                ":target",
                "link-info",
                "entry",
                "--validate-with",
                ":one",
                ":two",
            ])
            .unwrap(),
        );
        let ContainerOperation::LinkInfo(args) = command.operation else {
            panic!("expected link-info");
        };
        assert_eq!(
            args.validate_with
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec![":one", ":two"]
        );
    }

    #[test]
    fn container_help_groups_every_visible_operation() {
        let mut command = Cli::command();
        let container = command.find_subcommand_mut("container").unwrap();
        let help = container.render_long_help().to_string();
        for heading in [
            "General container commands:",
            "Local container commands:",
            "Link container commands:",
        ] {
            assert!(help.contains(heading));
        }
        for subcommand in container
            .get_subcommands()
            .filter(|command| command.get_name() != "help")
        {
            assert!(help.contains(&format!("  {}", subcommand.get_name())));
        }
    }

    #[test]
    fn parses_log_levels_and_configurable_init() {
        let cli = Cli::try_parse_from([
            "kcm",
            "--log-level",
            "info",
            "container",
            ":version",
            "init",
            "configurable",
        ])
        .unwrap();
        assert_eq!(cli.log_level, Some(LogLevel::Info));
        let command = container(cli);
        assert!(matches!(
            command.operation,
            ContainerOperation::Init(InitArgs {
                kind: ContainerKind::Configurable,
                ..
            })
        ));
    }
}
