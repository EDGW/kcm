//! Clap command tree, positional arguments, options, aliases, and value enums.

use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use kako_craft_lib::container::EntryKey;

/// Custom long-help layout grouping general, local, and link-container operations.
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
  tests         Generate container fixtures for diagnostic testing

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
/// Fully parsed top-level `kcm` invocation.
pub(crate) struct Cli {
    /// Enable library logs at this level. Logging is disabled when omitted.
    #[arg(long, value_enum, global = true, value_name = "LEVEL")]
    pub(crate) log_level: Option<LogLevel>,

    #[command(subcommand)]
    /// Selected top-level command family and its parsed arguments.
    pub(crate) command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
/// User-selectable maximum tracing verbosity.
pub(crate) enum LogLevel {
    /// Emit trace, debug, informational, warning, and error events.
    Trace,
    /// Emit debug, informational, warning, and error events.
    Debug,
    /// Emit informational, warning, and error events.
    Info,
    /// Emit warning and error events.
    Warn,
    /// Emit only error events.
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
/// Top-level command families supported by `kcm`.
pub(crate) enum Command {
    /// Inspect and operate on a kako-craft container.
    Container(ContainerCommand),
}

#[derive(Debug, Args)]
#[command(help_template = CONTAINER_HELP_TEMPLATE)]
/// Container root followed by one structured container operation.
pub(crate) struct ContainerCommand {
    /// Container directory. Defaults to the current directory.
    #[arg(value_name = "PATH", default_value = ".")]
    pub(crate) path: PathBuf,

    #[command(subcommand)]
    /// Operation to execute against `path`.
    pub(crate) operation: ContainerOperation,
}

#[derive(Debug, Subcommand)]
/// Complete general, local-only, and outgoing-link container command set.
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
    /// Generate container fixtures for testing diagnostics.
    Tests(ContainerTestsCommand),
}

/// Nested operations that deliberately create container test fixtures.
#[derive(Debug, Args)]
pub(crate) struct ContainerTestsCommand {
    /// Fixture-generation operation to execute.
    #[command(subcommand)]
    pub(crate) operation: ContainerTestsOperation,
}

/// Container fixture generators exposed by `kcm container tests`.
#[derive(Debug, Subcommand)]
pub(crate) enum ContainerTestsOperation {
    /// Generate linked containers covering reproducible broken-link error types.
    NewBroken(NewBrokenArgs),
}

/// Destination arguments for the broken-container fixture generator.
#[derive(Debug, Args)]
pub(crate) struct NewBrokenArgs {
    /// New root under which isolated broken-container scenarios are created.
    #[arg(value_name = "PATH", default_value = "broken-containers")]
    pub(crate) path: PathBuf,
}

#[derive(Debug, Args)]
/// Output format shared by commands that only toggle JSON rendering.
pub(crate) struct JsonOutput {
    /// Emit machine-readable JSON.
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Debug, Args)]
/// Detail, validation, repair, and output controls shared by list commands.
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
/// Concrete container implementation accepted by `container init`.
pub(crate) enum ContainerKind {
    /// Ordinary filesystem container supporting entries and incoming-link records.
    Local,
    /// Container supporting ordinary entries plus validated outgoing links.
    Link,
}

#[derive(Debug, Args)]
/// Initialization arguments for a new local or link container.
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
/// Entry key and one of the supported byte-input sources for add or update.
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
/// Entry read arguments, output destination, and optional interactive recovery.
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
/// Single entry-key arguments used by path resolution commands.
pub(crate) struct KeyArg {
    /// Entry key to resolve inside the selected container namespace.
    pub(crate) entry: EntryKey,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    pub(crate) auto_fix: bool,
}

#[derive(Debug, Args)]
/// One-or-more entry keys removed by a single preflighted batch operation.
pub(crate) struct RemoveArgs {
    /// Entry keys to remove; Clap enforces that at least one value is supplied.
    #[arg(required = true)]
    pub(crate) entries: Vec<EntryKey>,
}

#[derive(Debug, Args)]
/// Source and destination entry keys for ordinary copy and rename operations.
pub(crate) struct TwoKeys {
    /// Existing ordinary source entry key.
    pub(crate) from: EntryKey,
    /// New destination entry key, which must not already be occupied.
    pub(crate) to: EntryKey,
}

#[derive(Debug, Args)]
/// Arguments for creating an outgoing link to an ordinary target entry.
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
/// Arguments for removing one outgoing link and its reciprocal record.
pub(crate) struct UnlinkArgs {
    /// Outgoing-link entry key in this link container.
    pub(crate) linker_key: EntryKey,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    pub(crate) auto_fix: bool,
}

#[derive(Debug, Args)]
/// Source, destination, and recovery controls for outgoing-link copy or rename.
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
/// Entry selection, peer validation, recovery, and rendering options for link information.
pub(crate) struct LinkInfoArgs {
    /// Current-container entry key whose incoming or outgoing metadata is inspected.
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
/// Corresponding containers supplied to an interactive full consistency check.
pub(crate) struct CheckArgs {
    /// Check reciprocal metadata against one or more containers.
    #[arg(long, value_name = "PATH", num_args = 1.., action = ArgAction::Append)]
    pub(crate) validate_with: Vec<PathBuf>,
}

impl LinkArgs {
    /// Converts mutually exclusive path-policy flags into the library's three-state override.
    ///
    /// # Returns
    ///
    /// `Some(true)` for `--relative`, `Some(false)` for `--absolute`, and `None` when neither flag
    /// is present so the link container's persisted default is reused.
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
