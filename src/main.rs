use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use kako_craft_lib::container::{
    BrokenLinkError, CheckRepairAction, Container, ContainerEntryInfo, ContainerMetadata,
    ContainerWriteGuard, EntryKey, LinkAccessError, LinkCheckIssue, LinkCheckKind, LinkContainer,
    LinkInfo, LinkToError, LinkUnavailableError, LinkValidationIssue, LinkValidationIssueKind,
    LinkValidationReport, LinkValidationRunError, LocalContainer, UnlinkToError, open_container,
};
use serde_json::json;

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
struct Cli {
    /// Enable library logs at this level. Logging is disabled when omitted.
    #[arg(long, value_enum, global = true, value_name = "LEVEL")]
    log_level: Option<LogLevel>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LogLevel {
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
enum Command {
    /// Inspect and operate on a kako-craft container.
    Container(ContainerCommand),
}

#[derive(Debug, Args)]
#[command(help_template = CONTAINER_HELP_TEMPLATE)]
struct ContainerCommand {
    /// Container directory. Defaults to the current directory.
    #[arg(value_name = "PATH", default_value = ".")]
    path: PathBuf,

    #[command(subcommand)]
    operation: ContainerOperation,
}

#[derive(Debug, Subcommand)]
enum ContainerOperation {
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
struct JsonOutput {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct ListOptions {
    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    auto_fix: bool,

    /// Increase detail; use -vv for paths and full container identifiers.
    #[arg(short, long, action = ArgAction::Count)]
    verbose: u8,

    /// Include outgoing targets and all incoming-link sources.
    #[arg(long)]
    link_info: bool,

    /// Validate incoming-link sources against one or more containers.
    #[arg(
        long,
        value_name = "PATH",
        num_args = 1..,
        action = ArgAction::Append,
        requires = "link_info"
    )]
    validate_with: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ContainerKind {
    Local,
    Link,
}

#[derive(Debug, Args)]
struct InitArgs {
    /// Container implementation to create.
    #[arg(value_enum, default_value = "local")]
    kind: ContainerKind,

    /// Logical name stored in container metadata.
    #[arg(long)]
    name: Option<String>,

    /// Use absolute target-container paths by default for link containers.
    #[arg(long)]
    absolute: bool,
}

#[derive(Debug, Args)]
struct DataArgs {
    /// Entry key inside the container.
    entry: EntryKey,

    /// Read entry data from this file instead of stdin.
    #[arg(short, long, value_name = "PATH", conflicts_with = "content")]
    file: Option<PathBuf>,

    /// Use this UTF-8 string as the entry data instead of stdin.
    #[arg(short, long, conflicts_with = "file")]
    content: Option<String>,
}

#[derive(Debug, Args)]
struct ReadArgs {
    /// Entry key inside the container.
    entry: EntryKey,

    /// Write data to a file instead of stdout.
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    auto_fix: bool,
}

#[derive(Debug, Args)]
struct KeyArg {
    entry: EntryKey,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    auto_fix: bool,
}

#[derive(Debug, Args)]
struct RemoveArgs {
    #[arg(required = true)]
    entries: Vec<EntryKey>,
}

#[derive(Debug, Args)]
struct TwoKeys {
    from: EntryKey,
    to: EntryKey,
}

#[derive(Debug, Args)]
struct LinkArgs {
    /// Entry key to create in this link container.
    linker_key: EntryKey,

    /// Path to the target container.
    target_container: PathBuf,

    /// Entry key in the target container.
    target_key: EntryKey,

    /// Override the container default and use an absolute container path.
    #[arg(long, conflicts_with = "relative")]
    absolute: bool,

    /// Override the container default and prefer a relative container path.
    #[arg(long, conflicts_with = "absolute")]
    relative: bool,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    auto_fix: bool,
}

#[derive(Debug, Args)]
struct UnlinkArgs {
    /// Outgoing-link entry key in this link container.
    linker_key: EntryKey,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    auto_fix: bool,
}

#[derive(Debug, Args)]
struct LinkKeysArgs {
    /// Existing outgoing-link key.
    from: EntryKey,

    /// Destination outgoing-link key.
    to: EntryKey,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    auto_fix: bool,
}

#[derive(Debug, Args)]
struct LinkInfoArgs {
    entry: EntryKey,

    /// Validate reciprocal metadata against one or more containers.
    #[arg(long, value_name = "PATH", num_args = 1.., action = ArgAction::Append)]
    validate_with: Vec<PathBuf>,

    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    auto_fix: bool,
}

#[derive(Debug, Args)]
struct CheckArgs {
    /// Check reciprocal metadata against one or more containers.
    #[arg(long, value_name = "PATH", num_args = 1.., action = ArgAction::Append)]
    validate_with: Vec<PathBuf>,
}

impl LinkArgs {
    fn prefer_relative(&self) -> Option<bool> {
        if self.relative {
            Some(true)
        } else if self.absolute {
            Some(false)
        } else {
            None
        }
    }
}

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
enum ValidationCommandError {
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

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Container(command) => run_container(command),
    }
}

fn run_container(command: ContainerCommand) -> Result<()> {
    let path = command.path;
    match command.operation {
        ContainerOperation::Info(format) => print_info(&path, format.json),
        ContainerOperation::List(options) => print_container_list(&path, options),
        ContainerOperation::Init(args) => initialize(&path, args),

        ContainerOperation::Add(args) => {
            let container = open_local(&path)?;
            put(&container, args, true)
        }
        ContainerOperation::Update(args) => {
            let container = open_local(&path)?;
            put(&container, args, false)
        }
        ContainerOperation::Read(args) => {
            let data = with_auto_fix(&path, args.auto_fix, || {
                open_container(&path)?.read(&args.entry)
            })?;
            output_data(data, args.output)
        }
        ContainerOperation::Path(args) => {
            let entry_path = with_auto_fix(&path, args.auto_fix, || {
                open_container(&path)?.filepath(&args.entry)
            })?;
            println!("{}", entry_path.display());
            Ok(())
        }
        ContainerOperation::Check(args) => check(&path, args),
        ContainerOperation::Remove(args) => {
            let container = open_local(&path)?;
            remove(&container, args.entries)
        }
        ContainerOperation::Rename(args) => {
            let container = open_local(&path)?;
            rename(&container, args)
        }
        ContainerOperation::Copy(args) => {
            let container = open_local(&path)?;
            copy(&container, args)
        }

        ContainerOperation::LocalList(options) => print_local_list(&path, options),
        ContainerOperation::LocalAdd(args) => {
            let container = open_link(&path)?;
            put(&container, args, true)
        }
        ContainerOperation::LocalUpdate(args) => {
            let container = open_link(&path)?;
            put(&container, args, false)
        }
        ContainerOperation::LocalRead(args) => {
            let container = open_link(&path)?;
            output_data(container.local_read(&args.entry)?, args.output)
        }
        ContainerOperation::LocalPath(args) => {
            let container = open_link(&path)?;
            println!("{}", container.local_filepath(&args.entry)?.display());
            Ok(())
        }
        ContainerOperation::LocalRemove(args) => {
            let container = open_link(&path)?;
            remove(&container, args.entries)
        }
        ContainerOperation::LocalRename(args) => {
            let container = open_link(&path)?;
            rename(&container, args)
        }
        ContainerOperation::LocalCopy(args) => {
            let container = open_link(&path)?;
            copy(&container, args)
        }

        ContainerOperation::Link(args) => link(&path, args),
        ContainerOperation::LinkList(options) => print_link_list(&path, options),
        ContainerOperation::LinkCopy(args) => link_copy(&path, args),
        ContainerOperation::LinkRename(args) => link_rename(&path, args),
        ContainerOperation::LinkRemove(args) => unlink(&path, args),
        ContainerOperation::LinkInfo(args) => link_info(&path, args),
    }
}

fn initialize(path: &Path, args: InitArgs) -> Result<()> {
    let absolute = args.absolute;
    match (args.kind, args.name) {
        (ContainerKind::Local, Some(name)) => {
            LocalContainer::with_logical_name(path, name)?;
        }
        (ContainerKind::Local, None) => {
            LocalContainer::new(path)?;
        }
        (ContainerKind::Link, Some(name)) => {
            let container = LinkContainer::with_logical_name(path, name)?;
            container.writer()?.set_prefer_relative(!absolute)?;
        }
        (ContainerKind::Link, None) => {
            let container = LinkContainer::new(path)?;
            container.writer()?.set_prefer_relative(!absolute)?;
        }
    }
    print_info(path, false)
}

fn open_local(path: &Path) -> Result<LocalContainer> {
    let metadata = ContainerMetadata::load(path)?;
    if metadata.kind != "local" {
        bail!(
            "operation requires a local container, but {} is '{}' (use a local-* operation for ordinary entries in a link container)",
            path.display(),
            metadata.kind
        );
    }
    LocalContainer::from_metadata(path, metadata)
}

fn open_link(path: &Path) -> Result<LinkContainer> {
    let metadata = ContainerMetadata::load(path)?;
    if metadata.kind != "link" {
        bail!(
            "operation requires a link container, but {} is '{}'",
            path.display(),
            metadata.kind
        );
    }
    LinkContainer::from_metadata(path, metadata)
}

fn print_info(path: &Path, as_json: bool) -> Result<()> {
    let metadata = ContainerMetadata::load(path)?;
    let prefer_relative = if metadata.kind == "link" {
        Some(LinkContainer::from_metadata(path, metadata.clone())?.prefer_relative()?)
    } else {
        None
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

fn print_container_list(path: &Path, options: ListOptions) -> Result<()> {
    print_list(path, options, || open_container(path)?.list_info())
}

fn print_link_list(path: &Path, options: ListOptions) -> Result<()> {
    print_list(path, options, || open_link(path)?.link_list_info())
}

fn print_local_list(path: &Path, options: ListOptions) -> Result<()> {
    print_list(path, options, || open_link(path)?.local_list_info())
}

fn print_list(
    path: &Path,
    options: ListOptions,
    mut load: impl FnMut() -> Result<Vec<ContainerEntryInfo>>,
) -> Result<()> {
    let mut entries = with_auto_fix(path, options.auto_fix, &mut load)?;
    if options.verbose == 0 && !options.link_info {
        return render_entry_list(entries, &options, Vec::new());
    }
    let mut validations = validate_entries(path, &entries, &options.validate_with)?;
    if options.auto_fix && validations_need_fix(&validations) {
        run_check_interaction(path, &options.validate_with)?;
        entries = load().context("failed to reload entries after interactive link check")?;
        validations = validate_entries(path, &entries, &options.validate_with)?;
    }
    render_entry_list(entries, &options, validations)
}

fn render_entry_list(
    entries: Vec<ContainerEntryInfo>,
    options: &ListOptions,
    validations: Vec<Option<LinkValidationReport>>,
) -> Result<()> {
    if options.verbose == 0 && !options.link_info {
        let keys = entries
            .into_iter()
            .map(|entry| entry.key)
            .collect::<Vec<_>>();
        if options.json {
            println!("{}", serde_json::to_string_pretty(&keys)?);
        } else {
            for key in keys {
                println!("{key}");
            }
        }
        return Ok(());
    }

    if options.json {
        let values = entries
            .iter()
            .zip(&validations)
            .map(|(entry, validation)| entry_json(entry, options, validation.as_ref()))
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&values)?);
    } else {
        for (entry, validation) in entries.iter().zip(&validations) {
            println!("{}", entry_text(entry, options, validation.as_ref()));
        }
    }
    ensure_validation_success(validations.iter().flatten())
}

fn validate_entries(
    path: &Path,
    entries: &[ContainerEntryInfo],
    validate_with: &[PathBuf],
) -> Result<Vec<Option<LinkValidationReport>>> {
    if validate_with.is_empty() {
        if !entries
            .iter()
            .any(|entry| matches!(entry.link, LinkInfo::LinkTo { .. }))
        {
            return Ok((0..entries.len()).map(|_| None).collect());
        }
        let container = open_container(path)?;
        let guard = container.writer()?;
        return entries
            .iter()
            .map(|entry| {
                if matches!(entry.link, LinkInfo::LinkTo { .. }) {
                    guard
                        .validate_recorded_links(&entry.key)
                        .map(Some)
                        .map_err(Into::into)
                } else {
                    Ok(None)
                }
            })
            .collect();
    }
    let container = open_container(path)?;
    let guard = container.writer()?;
    let corresponding = open_corresponding(validate_with)?;
    let refs = corresponding
        .iter()
        .map(|container| container.as_ref())
        .collect::<Vec<_>>();
    entries
        .iter()
        .map(|entry| {
            guard
                .validate_links(&entry.key, &refs)
                .map(Some)
                .map_err(Into::into)
        })
        .collect()
}

fn open_corresponding(paths: &[PathBuf]) -> Result<Vec<Box<dyn Container>>> {
    paths
        .iter()
        .map(|path| {
            open_container(path).with_context(|| {
                format!("failed to open corresponding container {}", path.display())
            })
        })
        .collect()
}

fn entry_kind(link: &LinkInfo) -> &'static str {
    match link {
        LinkInfo::None => "local",
        LinkInfo::LinkTo { .. } => "link-to",
        LinkInfo::LinkFrom { .. } => "local-linked-from",
    }
}

fn entry_text(
    entry: &ContainerEntryInfo,
    options: &ListOptions,
    validation: Option<&LinkValidationReport>,
) -> String {
    let detail = options.verbose;
    let summary = match (&entry.link, options.link_info) {
        (LinkInfo::None, true) => format!("{}\ttype=local\tlink=none", entry.key),
        (LinkInfo::None, false) => format!("{}\ttype=local", entry.key),
        (
            LinkInfo::LinkTo {
                target_key,
                container_path,
                ..
            },
            true,
        ) => format!(
            "{}\ttype=link-to\ttarget={}:{}",
            entry.key,
            container_path.display(),
            target_key
        ),
        (LinkInfo::LinkTo { .. }, false) => format!("{}\ttype=link-to", entry.key),
        (LinkInfo::LinkFrom { linkers }, true) => format!(
            "{}\ttype=local-linked-from\tsources={}",
            entry.key,
            linkers.len()
        ),
        (LinkInfo::LinkFrom { .. }, false) => {
            format!("{}\ttype=local-linked-from", entry.key)
        }
    };
    let summary = if options.link_info {
        format!(
            "{summary}\tvalidation={}",
            validation.map_or("unverified", validation_status)
        )
    } else {
        summary
    };
    if detail < 2 {
        return summary;
    }

    let size = entry
        .size
        .map_or_else(|| "unknown".to_owned(), |size| size.to_string());
    let mut full = format!(
        "{summary}\tfilepath={}\tsize={size}",
        entry.filepath.display()
    );
    match (&entry.link, options.link_info) {
        (LinkInfo::LinkTo { container_uid, .. }, true) => {
            full.push_str(&format!("\ttarget_uid={container_uid}"));
        }
        (LinkInfo::LinkFrom { linkers }, true) => {
            for (index, linker) in linkers.iter().enumerate() {
                full.push_str(&format!(
                    "\tsource[{index}]={}:{}",
                    linker.linker_uid, linker.linker_key
                ));
            }
        }
        _ => {}
    }
    full
}

fn entry_json(
    entry: &ContainerEntryInfo,
    options: &ListOptions,
    validation: Option<&LinkValidationReport>,
) -> serde_json::Value {
    let detail = options.verbose;
    let mut value = json!({
        "key": entry.key,
        "type": entry_kind(&entry.link),
    });
    let object = value.as_object_mut().expect("entry JSON is an object");
    match (&entry.link, options.link_info) {
        (
            LinkInfo::LinkTo {
                target_key,
                container_path,
                container_uid,
            },
            true,
        ) => {
            object.insert("target_key".into(), json!(target_key));
            object.insert("container_path".into(), json!(container_path));
            if detail >= 2 {
                object.insert("container_uid".into(), json!(container_uid));
            }
        }
        (LinkInfo::LinkFrom { linkers }, true) => {
            if detail >= 2 {
                object.insert(
                    "sources".into(),
                    json!(
                        linkers
                            .iter()
                            .map(|linker| json!({
                                "container_uid": linker.linker_uid,
                                "linker_key": linker.linker_key,
                            }))
                            .collect::<Vec<_>>()
                    ),
                );
            } else {
                object.insert("source_count".into(), json!(linkers.len()));
            }
        }
        _ => {}
    }
    if detail >= 2 {
        object.insert("filepath".into(), json!(entry.filepath));
        object.insert("size".into(), json!(entry.size));
    }
    if options.link_info {
        object.insert(
            "validation".into(),
            validation.map_or_else(|| json!({ "status": "unverified" }), validation_json),
        );
    }
    value
}

fn input_data(args: &DataArgs) -> Result<Vec<u8>> {
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

fn put(container: &dyn Container, args: DataArgs, add_only: bool) -> Result<()> {
    let data = input_data(&args)?;
    let mut writer = container.writer()?;
    if add_only {
        writer.add(&args.entry, data)?;
    } else {
        writer.write(&args.entry, data)?;
    }
    Ok(())
}

fn output_data(data: Vec<u8>, output: Option<PathBuf>) -> Result<()> {
    if let Some(path) = output {
        fs::write(&path, data).with_context(|| format!("failed to write {}", path.display()))?;
    } else {
        io::stdout().write_all(&data)?;
    }
    Ok(())
}

fn check(path: &Path, args: CheckArgs) -> Result<()> {
    run_check_interaction(path, &args.validate_with)
}

fn run_check_interaction(path: &Path, validate_with: &[PathBuf]) -> Result<()> {
    let container = open_container(path)?;
    let corresponding = open_corresponding(validate_with)?;
    let refs = corresponding
        .iter()
        .map(|container| container.as_ref())
        .collect::<Vec<_>>();
    let mut writer = container.writer()?;
    interact_check(writer.as_mut(), &refs)
}

fn validations_need_fix(validations: &[Option<LinkValidationReport>]) -> bool {
    validations
        .iter()
        .flatten()
        .any(|report| !report.is_valid())
}

fn interact_check(
    writer: &mut dyn ContainerWriteGuard,
    corresponding: &[&dyn Container],
) -> Result<()> {
    let mut skipped = BTreeSet::new();
    let mut announced = false;
    loop {
        let issues = writer.check(corresponding)?;
        let remaining = issues
            .into_iter()
            .filter(|issue| !skipped.contains(&issue.id))
            .collect::<Vec<_>>();
        if remaining.is_empty() {
            if announced {
                eprintln!("link consistency interaction complete");
            } else {
                eprintln!("no link consistency issues found");
            }
            return Ok(());
        }
        if !announced {
            eprintln!("found {} link consistency issue(s)", remaining.len());
            announced = true;
        }
        let issue = &remaining[0];
        eprintln!("\n{}: {}", issue.key, describe_issue(issue));
        if let Some(expected) = &issue.expected {
            eprintln!("  expected: {expected}");
        }
        if let Some(actual) = &issue.actual {
            eprintln!("  actual:   {actual}");
        }
        for (index, action) in issue.actions.iter().enumerate() {
            eprintln!("  {}. {}", index + 1, check_action(issue, *action));
        }
        loop {
            eprint!("  select action [1-{}]: ", issue.actions.len());
            io::stderr().flush()?;
            let mut answer = String::new();
            if io::stdin().read_line(&mut answer)? == 0 {
                bail!("standard input closed during link-check interaction");
            }
            let Ok(selection) = answer.trim().parse::<usize>() else {
                eprintln!("  please enter an action number");
                continue;
            };
            let Some(action) = selection
                .checked_sub(1)
                .and_then(|index| issue.actions.get(index))
                .copied()
            else {
                eprintln!("  please enter an action number from the list");
                continue;
            };
            if action == CheckRepairAction::RemoveLocalOutgoingOnly {
                eprint!("  type 'remove-local-only' to confirm this one-sided cleanup: ");
                io::stderr().flush()?;
                let mut confirmation = String::new();
                if io::stdin().read_line(&mut confirmation)? == 0 {
                    bail!("standard input closed during link-check confirmation");
                }
                if confirmation.trim() != "remove-local-only" {
                    eprintln!("  confirmation did not match; no data changed");
                    continue;
                }
            }
            match writer.apply_check_action(issue, action, corresponding) {
                Ok(result) => {
                    eprintln!("  completed: {}", result.description);
                    if action == CheckRepairAction::Skip {
                        skipped.insert(issue.id.clone());
                    }
                    break;
                }
                Err(error) => {
                    eprintln!("  action failed: {error:#}");
                    eprintln!("  choose an action again");
                }
            }
        }
    }
}

fn with_auto_fix<T>(
    path: &Path,
    auto_fix: bool,
    mut operation: impl FnMut() -> Result<T>,
) -> Result<T> {
    match operation() {
        Ok(value) => Ok(value),
        Err(error) if auto_fix && is_repairable_link_error(&error) => {
            eprintln!("operation found a broken link: {error:#}");
            let container = open_container(path)?;
            {
                let mut writer = container.writer()?;
                interact_check(writer.as_mut(), &[])?;
            }
            operation().context("operation still failed after interactive link check")
        }
        Err(error) => Err(error),
    }
}

fn is_repairable_link_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<BrokenLinkError>().is_some()
            || cause.downcast_ref::<LinkUnavailableError>().is_some()
    }) || matches!(
        error.downcast_ref::<UnlinkToError>(),
        Some(UnlinkToError::Access(
            LinkAccessError::Broken(_) | LinkAccessError::Unavailable(_)
        ))
    ) || matches!(
        error.downcast_ref::<LinkToError>(),
        Some(LinkToError::Access(
            LinkAccessError::Broken(_) | LinkAccessError::Unavailable(_)
        ))
    )
}

fn is_unavailable_link_error(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<LinkUnavailableError>().is_some())
        || matches!(
            error.downcast_ref::<UnlinkToError>(),
            Some(UnlinkToError::Access(LinkAccessError::Unavailable(_)))
        )
        || matches!(
            error.downcast_ref::<LinkToError>(),
            Some(LinkToError::Access(LinkAccessError::Unavailable(_)))
        )
}

fn describe_issue(issue: &LinkCheckIssue) -> String {
    match issue.kind {
        LinkCheckKind::MissingSymlink => "metadata entry is missing its symlink",
        LinkCheckKind::IncorrectSymlink => "metadata entry has an incorrect symlink",
        LinkCheckKind::UnrecordedSymlink => "filesystem symlink is not recorded in metadata",
        LinkCheckKind::Validation(kind) => validation_issue_kind(kind),
        LinkCheckKind::Unavailable => "corresponding container is temporarily unavailable",
    }
    .to_owned()
}

fn check_action(issue: &LinkCheckIssue, action: CheckRepairAction) -> String {
    match action {
        CheckRepairAction::CreateMissingSymlink => format!(
            "create missing symlink '{}' -> '{}'",
            issue.key,
            issue.expected.as_deref().unwrap_or("recorded target")
        ),
        CheckRepairAction::ReplaceIncorrectSymlink => format!(
            "replace symlink '{}' with target '{}'",
            issue.key,
            issue.expected.as_deref().unwrap_or("recorded target")
        ),
        CheckRepairAction::DeleteUnrecordedSymlink => {
            format!("delete unrecorded symlink '{}'", issue.key)
        }
        CheckRepairAction::AddMissingIncomingRecord => format!(
            "add missing reciprocal incoming record for linker '{}:{}'",
            issue
                .corresponding_container_uid
                .as_deref()
                .unwrap_or("current"),
            issue.linker_key.as_deref().unwrap_or(&issue.key)
        ),
        CheckRepairAction::RemoveStaleIncomingRecord => format!(
            "remove stale incoming record '{}:{}'",
            issue
                .corresponding_container_uid
                .as_deref()
                .unwrap_or("unknown"),
            issue.linker_key.as_deref().unwrap_or(&issue.key)
        ),
        CheckRepairAction::AddMissingOutgoingRecord => {
            format!("add missing outgoing record for '{}'", issue.key)
        }
        CheckRepairAction::RemoveStaleOutgoingRecord => {
            format!("remove stale outgoing record for '{}'", issue.key)
        }
        CheckRepairAction::RemoveLocalOutgoingOnly => format!(
            "DANGEROUS: remove only local outgoing record and symlink '{}'",
            issue.key
        ),
        CheckRepairAction::RetryUnavailable => format!(
            "retry unavailable container '{}'",
            issue
                .corresponding_container_path
                .as_deref()
                .map_or_else(|| "unknown".into(), |path| path.display().to_string())
        ),
        CheckRepairAction::Skip => "skip this issue without changing data".into(),
    }
}

fn remove(container: &dyn Container, entries: Vec<EntryKey>) -> Result<()> {
    let mut writer = container.writer()?;
    writer.remove_many(&entries)?;
    Ok(())
}

fn rename(container: &dyn Container, args: TwoKeys) -> Result<()> {
    container.writer()?.rename(&args.from, &args.to)?;
    Ok(())
}

fn copy(container: &dyn Container, args: TwoKeys) -> Result<()> {
    container.writer()?.copy(&args.from, &args.to)?;
    Ok(())
}

fn link(path: &Path, args: LinkArgs) -> Result<()> {
    let preference = args.prefer_relative();
    with_auto_fix(path, args.auto_fix, || {
        let linker = open_link(path)?;
        let mut target = open_container(&args.target_container)?;
        linker.writer()?.link_to(
            &args.linker_key,
            target.as_mut(),
            &args.target_key,
            preference,
        )?;
        Ok(())
    })
}

fn unlink(path: &Path, args: UnlinkArgs) -> Result<()> {
    with_auto_fix(path, args.auto_fix, || {
        open_link(path)?.writer()?.unlink_to(&args.linker_key)?;
        Ok(())
    })
}

fn link_copy(path: &Path, args: LinkKeysArgs) -> Result<()> {
    with_auto_fix(path, args.auto_fix, || {
        open_link(path)?.writer()?.link_copy(&args.from, &args.to)?;
        Ok(())
    })
}

fn link_rename(path: &Path, args: LinkKeysArgs) -> Result<()> {
    with_auto_fix(path, args.auto_fix, || {
        open_link(path)?
            .writer()?
            .link_rename(&args.from, &args.to)?;
        Ok(())
    })
}

fn link_info(path: &Path, args: LinkInfoArgs) -> Result<()> {
    let mut result = collect_link_info(path, &args)?;
    if args.auto_fix
        && result
            .1
            .as_ref()
            .is_some_and(|validation| !validation.is_valid())
    {
        run_check_interaction(path, &args.validate_with)?;
        result = collect_link_info(path, &args)
            .context("failed to reload link info after interactive link check")?;
    }
    let (info, validation) = result;

    if args.json {
        let value = json!({
            "link_info": link_info_json(&info),
            "validation": validation.as_ref().map(validation_json),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        match &info {
            LinkInfo::None => println!("not linked"),
            LinkInfo::LinkTo {
                target_key,
                container_uid,
                container_path,
            } => {
                println!("direction: to");
                println!("container_uid: {container_uid}");
                println!("container_path: {}", container_path.display());
                println!("target_key: {target_key}");
            }
            LinkInfo::LinkFrom { linkers } => {
                println!("direction: from");
                for (index, linker) in linkers.iter().enumerate() {
                    println!("source[{index}].container_uid: {}", linker.linker_uid);
                    println!("source[{index}].linker_key: {}", linker.linker_key);
                }
            }
        }
        if let Some(report) = &validation {
            print_validation_report(report);
        } else {
            println!("validation: unverified");
        }
    }
    ensure_validation_success(validation.iter())
}

fn collect_link_info(
    path: &Path,
    args: &LinkInfoArgs,
) -> Result<(LinkInfo, Option<LinkValidationReport>)> {
    let container = open_container(path)?;
    let guard = container.writer()?;
    let info = guard.link_info(&args.entry)?;
    let validation = if args.validate_with.is_empty() {
        None
    } else {
        let corresponding = open_corresponding(&args.validate_with)?;
        let refs = corresponding
            .iter()
            .map(|container| container.as_ref())
            .collect::<Vec<_>>();
        Some(guard.validate_links(&args.entry, &refs)?)
    };
    Ok((info, validation))
}

fn link_info_json(info: &LinkInfo) -> serde_json::Value {
    match info {
        LinkInfo::None => json!({ "kind": "none" }),
        LinkInfo::LinkTo {
            target_key,
            container_uid,
            container_path,
        } => json!({
            "kind": "to",
            "target_key": target_key,
            "container_uid": container_uid,
            "container_path": container_path,
        }),
        LinkInfo::LinkFrom { linkers } => json!({
            "kind": "from",
            "linkers": linkers.iter().map(|linker| json!({
                "linker_key": linker.linker_key,
                "container_uid": linker.linker_uid,
            })).collect::<Vec<_>>(),
        }),
    }
}

fn validation_status(report: &LinkValidationReport) -> &'static str {
    if !report.broken.is_empty() {
        "broken"
    } else if !report.unavailable.is_empty() {
        "unavailable"
    } else if !report.valid.is_empty() {
        "verified"
    } else {
        "ignored"
    }
}

fn validation_json(report: &LinkValidationReport) -> serde_json::Value {
    json!({
        "status": validation_status(report),
        "valid": report.valid.iter().map(|matched| json!({
            "linker_container_uid": matched.linker_container_uid,
            "linker_key": matched.linker_key,
            "target_container_uid": matched.target_container_uid,
            "target_key": matched.target_key,
        })).collect::<Vec<_>>(),
        "broken": report.broken.iter().map(validation_issue_json).collect::<Vec<_>>(),
        "unavailable": report.unavailable.iter().map(|unavailable| json!({
            "container_uid": unavailable.container_uid,
            "container_path": unavailable.container_path,
            "reason": unavailable.reason,
        })).collect::<Vec<_>>(),
        "ignored": {
            "current_records": report.ignored_current_records,
            "corresponding_records": report.ignored_corresponding_records,
            "containers": report.ignored_containers,
        },
    })
}

fn validation_issue_json(issue: &LinkValidationIssue) -> serde_json::Value {
    json!({
        "kind": validation_issue_kind(issue.kind),
        "current_container_uid": issue.current_container_uid,
        "current_key": issue.current_key,
        "corresponding_container_uid": issue.corresponding_container_uid,
        "corresponding_container_path": issue.corresponding_container_path,
        "linker_key": issue.linker_key,
        "target_key": issue.target_key,
        "expected": issue.expected,
        "actual": issue.actual,
    })
}

fn validation_issue_kind(kind: LinkValidationIssueKind) -> &'static str {
    match kind {
        LinkValidationIssueKind::MissingOutgoingRecord => "missing_outgoing_record",
        LinkValidationIssueKind::UnexpectedOutgoingRecord => "unexpected_outgoing_record",
        LinkValidationIssueKind::MissingIncomingRecord => "missing_incoming_record",
        LinkValidationIssueKind::UnexpectedIncomingRecord => "unexpected_incoming_record",
        LinkValidationIssueKind::LinkerUidMismatch => "linker_uid_mismatch",
        LinkValidationIssueKind::TargetUidMismatch => "target_uid_mismatch",
        LinkValidationIssueKind::LinkerKeyMismatch => "linker_key_mismatch",
        LinkValidationIssueKind::TargetKeyMismatch => "target_key_mismatch",
        LinkValidationIssueKind::TargetEntryMissing => "target_entry_missing",
        LinkValidationIssueKind::ContainerPathMissing => "container_path_missing",
        LinkValidationIssueKind::ContainerPathMismatch => "container_path_mismatch",
        LinkValidationIssueKind::ContainerPathUidMismatch => "container_path_uid_mismatch",
        LinkValidationIssueKind::DuplicateIncomingRecord => "duplicate_incoming_record",
        LinkValidationIssueKind::DuplicateOutgoingRecord => "duplicate_outgoing_record",
        LinkValidationIssueKind::MetadataInvalid => "metadata_invalid",
        LinkValidationIssueKind::MaterializedSymlinkMissing => "materialized_symlink_missing",
        LinkValidationIssueKind::MaterializedSymlinkMismatch => "materialized_symlink_mismatch",
    }
}

fn print_validation_report(report: &LinkValidationReport) {
    println!("validation:");
    println!("  valid: {}", report.valid.len());
    println!("  broken: {}", report.broken.len());
    println!("  unavailable: {}", report.unavailable.len());
    println!(
        "  ignored_current_records: {}",
        report.ignored_current_records
    );
    println!(
        "  ignored_corresponding_records: {}",
        report.ignored_corresponding_records
    );
    println!("  ignored_containers: {}", report.ignored_containers);
    for issue in &report.broken {
        println!(
            "  broken[{}]: {} key={} corresponding={} expected={} actual={}",
            validation_issue_kind(issue.kind),
            issue.current_container_uid,
            issue.current_key,
            issue.corresponding_container_uid,
            issue.expected.as_deref().unwrap_or("-"),
            issue.actual.as_deref().unwrap_or("-"),
        );
    }
    for unavailable in &report.unavailable {
        println!(
            "  unavailable: {} ({}) at {}",
            unavailable.reason,
            unavailable.container_uid,
            unavailable.container_path.display()
        );
    }
}

fn ensure_validation_success<'a>(
    reports: impl IntoIterator<Item = &'a LinkValidationReport>,
) -> Result<()> {
    let mut broken = 0;
    let mut unavailable = 0;
    for report in reports {
        broken += report.broken.len();
        unavailable += report.unavailable.len();
    }
    if broken > 0 {
        return Err(ValidationCommandError::Broken(broken).into());
    }
    if unavailable > 0 {
        return Err(ValidationCommandError::Unavailable(unavailable).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

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
        let cli =
            Cli::try_parse_from(["kcm", "--log-level", "debug", "container", "info"]).unwrap();
        assert!(matches!(cli.log_level, Some(LogLevel::Debug)));
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
