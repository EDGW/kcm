use std::fmt;
use std::io;

use anyhow::Result;
use clap::Parser;
use kako_craft_lib::container::LinkValidationRunError;

mod cli;
mod commands;

use cli::*;
use commands::interaction::is_unavailable_link_error;
use commands::run;

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
pub(crate) enum ValidationCommandError {
    Broken(usize),
    Unavailable(usize),
}

impl ValidationCommandError {
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
    use std::path::PathBuf;

    use super::*;
    use crate::commands::interaction::check_action;
    use crate::commands::render::{entry_json, validation_json};
    use clap::CommandFactory;
    use kako_craft_lib::container::{
        CheckRepairAction, ContainerEntryInfo, LinkCheckIssue, LinkCheckKind, LinkInfo,
        LinkValidationReport,
    };
    use serde_json::json;

    #[test]
    fn container_path_defaults_to_current_directory() {
        let cli = Cli::try_parse_from(["kcm", "container", "info"]).unwrap();
        assert!(cli.log_level.is_none());
        let Command::Container(command) = cli.command;
        assert_eq!(command.path, PathBuf::from("."));
        assert!(matches!(command.operation, ContainerOperation::Info(_)));
    }

    #[test]
    fn parses_global_log_level() {
        for (name, expected) in [
            ("trace", LogLevel::Trace),
            ("debug", LogLevel::Debug),
            ("info", LogLevel::Info),
            ("warn", LogLevel::Warn),
            ("error", LogLevel::Error),
        ] {
            let cli =
                Cli::try_parse_from(["kcm", "--log-level", name, "container", "info"]).unwrap();
            assert_eq!(cli.log_level, Some(expected));
        }
        let cli =
            Cli::try_parse_from(["kcm", "container", "info", "--log-level", "trace"]).unwrap();
        assert!(matches!(cli.log_level, Some(LogLevel::Trace)));
        assert!(
            Cli::try_parse_from(["kcm", "--log-level", "silent", "container", "info"]).is_err()
        );
    }

    #[test]
    fn parses_explicit_path_and_local_operation() {
        let cli = Cli::try_parse_from([
            "kcm",
            "container",
            "./links",
            "local-add",
            "note.txt",
            "--content",
            "hello",
        ])
        .unwrap();
        let Command::Container(command) = cli.command;
        assert_eq!(command.path, PathBuf::from("./links"));
        let ContainerOperation::LocalAdd(args) = command.operation else {
            panic!("expected local-add operation");
        };
        assert_eq!(args.entry, "note.txt");
        assert_eq!(args.content.as_deref(), Some("hello"));
    }

    #[test]
    fn parses_link_options() {
        let cli = Cli::try_parse_from([
            "kcm",
            "container",
            "links",
            "link",
            "linked.txt",
            "target",
            "source.txt",
            "--absolute",
        ])
        .unwrap();
        let Command::Container(command) = cli.command;
        let ContainerOperation::Link(args) = command.operation else {
            panic!("expected link operation");
        };
        assert!(args.absolute);
        assert_eq!(args.linker_key, "linked.txt");
        assert_eq!(args.target_container, PathBuf::from("target"));
        assert_eq!(args.target_key, "source.txt");
    }

    #[test]
    fn parses_link_crud_commands() {
        assert!(Cli::try_parse_from(["kcm", "container", "links", "link-list"]).is_ok());
        assert!(
            Cli::try_parse_from(["kcm", "container", "links", "link-copy", "old", "new"]).is_ok()
        );
        assert!(
            Cli::try_parse_from(["kcm", "container", "links", "link-rename", "old", "new"]).is_ok()
        );
        assert!(Cli::try_parse_from(["kcm", "container", "links", "link-remove", "old"]).is_ok());
        assert!(Cli::try_parse_from(["kcm", "container", "links", "unlink", "old"]).is_ok());
    }

    #[test]
    fn parses_all_visible_and_compatibility_aliases() {
        for arguments in [
            vec!["kcm", "container", "get", "entry"],
            vec!["kcm", "container", "delete", "entry"],
            vec!["kcm", "container", "rm", "entry"],
            vec!["kcm", "container", "move", "old", "new"],
            vec!["kcm", "container", "mv", "old", "new"],
            vec!["kcm", "container", "cp", "old", "new"],
            vec!["kcm", "container", "unlink", "entry"],
            vec!["kcm", "container", "link-info", "entry"],
        ] {
            assert!(
                Cli::try_parse_from(arguments.clone()).is_ok(),
                "failed to parse alias invocation {arguments:?}"
            );
        }
    }

    #[test]
    fn parses_list_detail_levels() {
        let cli = Cli::try_parse_from([
            "kcm",
            "container",
            "links",
            "link-list",
            "-vv",
            "--json",
            "--auto-fix",
            "--link-info",
        ])
        .unwrap();
        let Command::Container(command) = cli.command;
        let ContainerOperation::LinkList(options) = command.operation else {
            panic!("expected link-list");
        };
        assert_eq!(options.verbose, 2);
        assert!(options.json);
        assert!(options.auto_fix);
        assert!(options.link_info);
        assert!(options.validate_with.is_empty());
    }

    #[test]
    fn parses_multiple_validation_containers_in_both_forms() {
        let cli = Cli::try_parse_from([
            "kcm",
            "container",
            "target",
            "link-info",
            "entry",
            "--validate-with",
            "one",
            "two",
            "--validate-with",
            "three",
            "--json",
        ])
        .unwrap();
        let Command::Container(command) = cli.command;
        let ContainerOperation::LinkInfo(args) = command.operation else {
            panic!("expected link-info");
        };
        assert_eq!(
            args.validate_with,
            ["one", "two", "three"].map(PathBuf::from)
        );

        let cli = Cli::try_parse_from([
            "kcm",
            "container",
            "target",
            "list",
            "--link-info",
            "--validate-with",
            "one",
            "--validate-with",
            "two",
        ])
        .unwrap();
        let Command::Container(command) = cli.command;
        let ContainerOperation::List(options) = command.operation else {
            panic!("expected list");
        };
        assert_eq!(options.validate_with, ["one", "two"].map(PathBuf::from));
        assert!(
            Cli::try_parse_from([
                "kcm",
                "container",
                "target",
                "list",
                "--validate-with",
                "one",
            ])
            .is_err()
        );

        let cli = Cli::try_parse_from([
            "kcm",
            "container",
            "target",
            "check",
            "--validate-with",
            "one",
            "two",
        ])
        .unwrap();
        let Command::Container(command) = cli.command;
        let ContainerOperation::Check(args) = command.operation else {
            panic!("expected check");
        };
        assert_eq!(args.validate_with, ["one", "two"].map(PathBuf::from));
        assert!(
            Cli::try_parse_from([
                "kcm",
                "container",
                "target",
                "link-info",
                "entry",
                "--validate-with",
                "--json",
            ])
            .is_err()
        );
    }

    #[test]
    fn list_link_info_requires_option() {
        let entry = ContainerEntryInfo {
            key: "link.txt".into(),
            filepath: PathBuf::from("link.txt"),
            size: Some(4),
            link: LinkInfo::LinkTo {
                target_key: "target.txt".into(),
                container_uid: "target-uid".into(),
                container_path: PathBuf::from("../target"),
            },
        };
        let without = ListOptions {
            json: true,
            auto_fix: false,
            verbose: 2,
            link_info: false,
            validate_with: Vec::new(),
        };
        let without_value = entry_json(&entry, &without, None);
        assert!(without_value.get("target_key").is_none());
        assert!(without_value.get("container_uid").is_none());
        let with = ListOptions {
            link_info: true,
            ..without
        };
        let with_value = entry_json(&entry, &with, None);
        assert_eq!(with_value["target_key"], "target.txt");
        assert_eq!(with_value["container_uid"], "target-uid");
    }

    #[test]
    fn check_actions_are_specific() {
        let issue = LinkCheckIssue {
            id: "symlink:missing:missing".into(),
            key: "missing".into(),
            kind: LinkCheckKind::MissingSymlink,
            corresponding_container_uid: None,
            corresponding_container_path: None,
            linker_key: Some("missing".into()),
            target_key: Some("target".into()),
            expected: Some("../target".into()),
            actual: None,
            actions: vec![CheckRepairAction::CreateMissingSymlink],
        };
        assert_eq!(
            check_action(&issue, CheckRepairAction::CreateMissingSymlink),
            "create missing symlink 'missing' -> '../target'"
        );
    }

    #[test]
    fn validation_json_has_stable_sections() {
        let report = LinkValidationReport {
            ignored_current_records: 1,
            ignored_corresponding_records: 2,
            ignored_containers: 3,
            ..LinkValidationReport::default()
        };
        let value = validation_json(&report);
        assert_eq!(value["status"], "ignored");
        assert_eq!(value["valid"], json!([]));
        assert_eq!(value["broken"], json!([]));
        assert_eq!(value["unavailable"], json!([]));
        assert_eq!(value["ignored"]["current_records"], 1);
        assert_eq!(value["ignored"]["corresponding_records"], 2);
        assert_eq!(value["ignored"]["containers"], 3);
    }

    #[test]
    fn container_help_groups_every_subcommand() {
        let mut command = Cli::command();
        let container = command.find_subcommand_mut("container").unwrap();
        let subcommands = container
            .get_subcommands()
            .filter(|command| command.get_name() != "help")
            .map(|command| command.get_name().to_owned())
            .collect::<Vec<_>>();
        let help = container.render_long_help().to_string();

        assert!(help.contains("General container commands:"));
        assert!(help.contains("Local container commands:"));
        assert!(help.contains("Link container commands:"));
        for subcommand in subcommands {
            assert!(
                help.contains(&format!("  {subcommand}")),
                "container help is missing the '{subcommand}' subcommand"
            );
        }
    }
}
