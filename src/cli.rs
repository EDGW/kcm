use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use kako_craft_lib::container::EntryKey;

const CONTAINER_HELP_TEMPLATE: &str = "\
{about-with-newline}
{usage-heading} {usage}

General container commands:
  info          Show container metadata
  list          List all entries (ordinary entries and outgoing links)
  init          Initialize a container
  read          Read an entry, following an outgoing link when present
  path          Resolve an entry's filesystem path
  linkinfo      Show the link metadata recorded for an entry (alias: link-info)
  check         Check link metadata and symlinks, then interactively repair issues

Local container commands:
  add           Add a new entry
  update        Replace or create an entry
  remove        Remove one or more entries
  rename        Rename an entry
  copy          Copy an entry

Link container commands:
  local-list    List only ordinary local entries
  local-add     Add a new ordinary local entry
  local-update  Replace or create an ordinary local entry
  local-read    Read an ordinary entry without following outgoing links
  local-path    Resolve an ordinary entry's filesystem path
  local-remove  Remove one or more ordinary local entries
  local-rename  Rename an ordinary local entry
  local-copy    Copy an ordinary local entry
  link          Create an outgoing link to another container
  link-list     List outgoing links
  link-copy     Copy an outgoing-link definition
  link-rename   Rename an outgoing link
  link-remove   Remove an outgoing link (alias: unlink)

Arguments:
{positionals}

Options:
{options}

Run `kcm container help <COMMAND>` for details about a command.
";

#[derive(Debug, Parser)]
#[command(
    name = "kcm",
    version,
    about = "Command-line wrapper for kako-craft-lib"
)]
pub(crate) struct Cli {
    /// Enable library logs at this level. Logging is disabled when omitted.
    #[arg(long, value_enum, global = true, value_name = "LEVEL")]
    pub(crate) log_level: Option<LogLevel>,

    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl From<LogLevel> for tracing::Level {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Trace => Self::TRACE,
            LogLevel::Debug => Self::DEBUG,
            LogLevel::Info => Self::INFO,
            LogLevel::Warn => Self::WARN,
            LogLevel::Error => Self::ERROR,
        }
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Inspect and operate on a kako-craft container.
    Container(ContainerCommand),
}

#[derive(Debug, Args)]
#[command(help_template = CONTAINER_HELP_TEMPLATE)]
pub(crate) struct ContainerCommand {
    /// Container directory. Defaults to the current directory.
    #[arg(value_name = "PATH", default_value = ".")]
    pub(crate) path: PathBuf,

    #[command(subcommand)]
    pub(crate) operation: ContainerOperation,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ContainerOperation {
    /// Show container metadata.
    Info(JsonOutput),
    /// List all entries (ordinary entries and outgoing links).
    List(ListOptions),
    /// Initialize a container.
    Init(InitArgs),

    /// Add a new entry to a local container.
    Add(DataArgs),
    /// Replace or create an entry in a local container.
    Update(DataArgs),
    /// Read an entry, following an outgoing link when present.
    #[command(alias = "get")]
    Read(ReadArgs),
    /// Resolve an entry's filesystem path.
    Path(KeyArg),
    /// Check link metadata and symlinks, then interactively repair issues.
    Check(CheckArgs),
    /// Remove one or more entries from a local container.
    #[command(alias = "delete", alias = "rm")]
    Remove(RemoveArgs),
    /// Rename an entry in a local container.
    #[command(alias = "move", alias = "mv")]
    Rename(TwoKeys),
    /// Copy an entry in a local container.
    #[command(alias = "cp")]
    Copy(TwoKeys),

    /// List only the ordinary local entries of a link container.
    LocalList(ListOptions),
    /// Add a new ordinary entry to a link container.
    LocalAdd(DataArgs),
    /// Replace or create an ordinary entry in a link container.
    LocalUpdate(DataArgs),
    /// Read an ordinary entry without following outgoing links.
    LocalRead(ReadArgs),
    /// Resolve an ordinary entry's filesystem path.
    LocalPath(KeyArg),
    /// Remove one or more ordinary entries from a link container.
    LocalRemove(RemoveArgs),
    /// Rename an ordinary entry in a link container.
    LocalRename(TwoKeys),
    /// Copy an ordinary entry in a link container.
    LocalCopy(TwoKeys),

    /// Link an entry in a link container to an entry in another container.
    Link(LinkArgs),
    /// List outgoing links only.
    LinkList(ListOptions),
    /// Copy an outgoing-link definition to another key.
    LinkCopy(LinkKeysArgs),
    /// Rename an outgoing link while preserving its target.
    LinkRename(LinkKeysArgs),
    /// Remove an outgoing link and its reciprocal incoming-link record.
    #[command(name = "link-remove", visible_alias = "unlink")]
    LinkRemove(UnlinkArgs),
    /// Show the link metadata recorded for an entry.
    #[command(name = "linkinfo", visible_alias = "link-info")]
    LinkInfo(LinkInfoArgs),
}

#[derive(Debug, Args)]
pub(crate) struct JsonOutput {
    /// Emit machine-readable JSON.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ListOptions {
    /// Emit machine-readable JSON.
    #[arg(long)]
    pub(crate) json: bool,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    pub(crate) auto_fix: bool,

    /// Increase detail; use -vv for paths and full container identifiers.
    #[arg(short, long, action = ArgAction::Count)]
    pub(crate) verbose: u8,

    /// Include outgoing targets and all incoming-link sources.
    #[arg(long)]
    pub(crate) link_info: bool,

    /// Validate incoming-link sources against one or more containers.
    #[arg(
        long,
        value_name = "PATH",
        num_args = 1..,
        action = ArgAction::Append,
        requires = "link_info"
    )]
    pub(crate) validate_with: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum ContainerKind {
    Local,
    Link,
}

#[derive(Debug, Args)]
pub(crate) struct InitArgs {
    /// Container implementation to create.
    #[arg(value_enum, default_value = "local")]
    pub(crate) kind: ContainerKind,

    /// Logical name stored in container metadata.
    #[arg(long)]
    pub(crate) name: Option<String>,

    /// Use absolute target-container paths by default for link containers.
    #[arg(long)]
    pub(crate) absolute: bool,
}

#[derive(Debug, Args)]
pub(crate) struct DataArgs {
    /// Entry key inside the container.
    pub(crate) entry: EntryKey,

    /// Read entry data from this file instead of stdin.
    #[arg(short, long, value_name = "PATH", conflicts_with = "content")]
    pub(crate) file: Option<PathBuf>,

    /// Use this UTF-8 string as the entry data instead of stdin.
    #[arg(short, long, conflicts_with = "file")]
    pub(crate) content: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ReadArgs {
    /// Entry key inside the container.
    pub(crate) entry: EntryKey,

    /// Write data to a file instead of stdout.
    #[arg(short, long, value_name = "PATH")]
    pub(crate) output: Option<PathBuf>,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    pub(crate) auto_fix: bool,
}

#[derive(Debug, Args)]
pub(crate) struct KeyArg {
    pub(crate) entry: EntryKey,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    pub(crate) auto_fix: bool,
}

#[derive(Debug, Args)]
pub(crate) struct RemoveArgs {
    #[arg(required = true)]
    pub(crate) entries: Vec<EntryKey>,
}

#[derive(Debug, Args)]
pub(crate) struct TwoKeys {
    pub(crate) from: EntryKey,
    pub(crate) to: EntryKey,
}

#[derive(Debug, Args)]
pub(crate) struct LinkArgs {
    /// Entry key to create in this link container.
    pub(crate) linker_key: EntryKey,

    /// Path to the target container.
    pub(crate) target_container: PathBuf,

    /// Entry key in the target container.
    pub(crate) target_key: EntryKey,

    /// Override the container default and use an absolute container path.
    #[arg(long, conflicts_with = "relative")]
    pub(crate) absolute: bool,

    /// Override the container default and prefer a relative container path.
    #[arg(long, conflicts_with = "absolute")]
    pub(crate) relative: bool,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    pub(crate) auto_fix: bool,
}

#[derive(Debug, Args)]
pub(crate) struct UnlinkArgs {
    /// Outgoing-link entry key in this link container.
    pub(crate) linker_key: EntryKey,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    pub(crate) auto_fix: bool,
}

#[derive(Debug, Args)]
pub(crate) struct LinkKeysArgs {
    /// Existing outgoing-link key.
    pub(crate) from: EntryKey,

    /// Destination outgoing-link key.
    pub(crate) to: EntryKey,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    pub(crate) auto_fix: bool,
}

#[derive(Debug, Args)]
pub(crate) struct LinkInfoArgs {
    pub(crate) entry: EntryKey,

    /// Validate reciprocal metadata against one or more containers.
    #[arg(long, value_name = "PATH", num_args = 1.., action = ArgAction::Append)]
    pub(crate) validate_with: Vec<PathBuf>,

    /// Emit machine-readable JSON.
    #[arg(long)]
    pub(crate) json: bool,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    pub(crate) auto_fix: bool,
}

#[derive(Debug, Args)]
pub(crate) struct CheckArgs {
    /// Check reciprocal metadata against one or more containers.
    #[arg(long, value_name = "PATH", num_args = 1.., action = ArgAction::Append)]
    pub(crate) validate_with: Vec<PathBuf>,
}

impl LinkArgs {
    pub(crate) fn prefer_relative(&self) -> Option<bool> {
        if self.relative {
            Some(true)
        } else if self.absolute {
            Some(false)
        } else {
            None
        }
    }
}
