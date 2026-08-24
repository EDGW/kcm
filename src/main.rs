use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use kako_craft_lib::container::{
    BrokenLinkError, Container, ContainerEntryInfo, ContainerMetadata, ContainerWriteGuard,
    EntryKey, LinkCheckIssue, LinkCheckKind, LinkContainer, LinkInfo, LocalContainer,
    open_container,
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
  linkinfo      Show the link metadata recorded for an entry
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
    #[command(subcommand)]
    command: Command,
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
    Check,
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
    #[command(name = "linkinfo", alias = "link-info")]
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

    /// Validate the reciprocal metadata against this other container.
    #[arg(long, value_name = "PATH")]
    validate_with: Option<PathBuf>,

    /// Emit machine-readable JSON.
    #[arg(long)]
    json: bool,

    /// Interactively check and repair a broken link, then retry.
    #[arg(long)]
    auto_fix: bool,
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
    if let Err(error) = run(Cli::parse()) {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

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
        ContainerOperation::Check => check(&path),
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

        ContainerOperation::LocalList(options) => {
            let container = open_link(&path)?;
            print_entry_list(container.local_list_info()?, &options)
        }
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
    let entries = with_auto_fix(path, options.auto_fix, || open_container(path)?.list_info())?;
    print_entry_list(entries, &options)
}

fn print_link_list(path: &Path, options: ListOptions) -> Result<()> {
    let entries = with_auto_fix(path, options.auto_fix, || open_link(path)?.link_list_info())?;
    print_entry_list(entries, &options)
}

fn print_entry_list(entries: Vec<ContainerEntryInfo>, options: &ListOptions) -> Result<()> {
    if options.verbose == 0 {
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
            .map(|entry| entry_json(entry, options.verbose))
            .collect::<Vec<_>>();
        println!("{}", serde_json::to_string_pretty(&values)?);
    } else {
        for entry in &entries {
            println!("{}", entry_text(entry, options.verbose));
        }
    }
    Ok(())
}

fn entry_kind(link: &LinkInfo) -> &'static str {
    match link {
        LinkInfo::None => "local",
        LinkInfo::LinkTo { .. } => "link-to",
        LinkInfo::LinkFrom { .. } => "local-linked-from",
    }
}

fn entry_text(entry: &ContainerEntryInfo, detail: u8) -> String {
    let summary = match &entry.link {
        LinkInfo::None => format!("{}\ttype=local", entry.key),
        LinkInfo::LinkTo {
            target_key,
            container_path,
            ..
        } => format!(
            "{}\ttype=link-to\ttarget={}:{}",
            entry.key,
            container_path.display(),
            target_key
        ),
        LinkInfo::LinkFrom { linkers } => format!(
            "{}\ttype=local-linked-from\tsources={}",
            entry.key,
            linkers.len()
        ),
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
    match &entry.link {
        LinkInfo::LinkTo { container_uid, .. } => {
            full.push_str(&format!("\ttarget_uid={container_uid}"));
        }
        LinkInfo::LinkFrom { linkers } => {
            for (index, linker) in linkers.iter().enumerate() {
                full.push_str(&format!(
                    "\tsource[{index}]={}:{}",
                    linker.linker_uid, linker.linker_key
                ));
            }
        }
        LinkInfo::None => {}
    }
    full
}

fn entry_json(entry: &ContainerEntryInfo, detail: u8) -> serde_json::Value {
    let mut value = json!({
        "key": entry.key,
        "type": entry_kind(&entry.link),
    });
    let object = value.as_object_mut().expect("entry JSON is an object");
    match &entry.link {
        LinkInfo::LinkTo {
            target_key,
            container_path,
            container_uid,
        } => {
            object.insert("target_key".into(), json!(target_key));
            object.insert("container_path".into(), json!(container_path));
            if detail >= 2 {
                object.insert("container_uid".into(), json!(container_uid));
            }
        }
        LinkInfo::LinkFrom { linkers } => {
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
        LinkInfo::None => {}
    }
    if detail >= 2 {
        object.insert("filepath".into(), json!(entry.filepath));
        object.insert("size".into(), json!(entry.size));
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

fn check(path: &Path) -> Result<()> {
    let container = open_container(path)?;
    let mut writer = container.writer()?;
    interact_check(writer.as_mut())
}

fn interact_check(writer: &mut dyn ContainerWriteGuard) -> Result<()> {
    let issues = writer.check()?;
    if issues.is_empty() {
        println!("no link consistency issues found");
        return Ok(());
    }

    println!("found {} link consistency issue(s)", issues.len());
    for (index, issue) in issues.iter().enumerate() {
        println!(
            "\n[{}/{}] {}: {}",
            index + 1,
            issues.len(),
            issue.key,
            describe_issue(issue)
        );
        if let Some(expected) = &issue.expected {
            println!("  expected: {}", expected.display());
        }
        if let Some(actual) = &issue.actual {
            println!("  actual:   {}", actual.display());
        }

        let action = check_action(issue);
        println!("  proposed action: {action}");

        loop {
            print!("  apply this action? [y]es/[s]kip: ");
            io::stdout().flush()?;
            let mut answer = String::new();
            io::stdin().read_line(&mut answer)?;
            match answer.trim().to_ascii_lowercase().as_str() {
                "s" | "skip" => break,
                "y" | "yes" => match writer.repair_check(issue) {
                    Ok(()) => {
                        println!("  completed: {action}");
                        break;
                    }
                    Err(error) => {
                        eprintln!("  action failed: {error:#}");
                        println!("  choose yes to retry the action, or skip");
                    }
                },
                _ => println!("  please enter y or s"),
            }
        }
    }
    Ok(())
}

fn with_auto_fix<T>(
    path: &Path,
    auto_fix: bool,
    mut operation: impl FnMut() -> Result<T>,
) -> Result<T> {
    match operation() {
        Ok(value) => Ok(value),
        Err(error) if auto_fix && error.downcast_ref::<BrokenLinkError>().is_some() => {
            eprintln!("operation found a broken link: {error:#}");
            let container = open_link(path)?;
            {
                let mut writer = container.writer()?;
                interact_check(&mut writer)?;
            }
            operation().context("operation still failed after interactive link check")
        }
        Err(error) => Err(error),
    }
}

fn describe_issue(issue: &LinkCheckIssue) -> &'static str {
    match issue.kind {
        LinkCheckKind::MissingSymlink => "metadata entry is missing its symlink",
        LinkCheckKind::IncorrectSymlink => "metadata entry has an incorrect symlink",
        LinkCheckKind::UnrecordedSymlink => "filesystem symlink is not recorded in metadata",
        LinkCheckKind::BrokenTarget => "recorded target container or entry is broken",
    }
}

fn check_action(issue: &LinkCheckIssue) -> &'static str {
    match issue.kind {
        LinkCheckKind::MissingSymlink => "create the missing symlink from recorded metadata",
        LinkCheckKind::IncorrectSymlink => "replace the incorrect symlink with the recorded target",
        LinkCheckKind::UnrecordedSymlink => "delete the unrecorded symlink",
        LinkCheckKind::BrokenTarget => "delete the broken outgoing-link record and its symlink",
    }
}

fn remove(container: &dyn Container, entries: Vec<EntryKey>) -> Result<()> {
    let mut writer = container.writer()?;
    for entry in entries {
        writer.remove(&entry)?;
    }
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
    let info = with_auto_fix(path, args.auto_fix, || {
        let container = open_container(path)?;
        let guard = container.writer()?;
        let info = guard.link_info(&args.entry)?;
        if let Some(other_path) = &args.validate_with {
            let other = open_container(other_path)?;
            guard.validate_link(&args.entry, other.as_ref())?;
        }
        Ok(info)
    })?;

    if args.json {
        let value = match info {
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
                "linkers": linkers.into_iter().map(|linker| json!({
                    "linker_key": linker.linker_key,
                    "container_uid": linker.linker_uid,
                })).collect::<Vec<_>>(),
            }),
        };
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        match info {
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
        let Command::Container(command) = cli.command;
        assert_eq!(command.path, PathBuf::from("."));
        assert!(matches!(command.operation, ContainerOperation::Info(_)));
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
        ])
        .unwrap();
        let Command::Container(command) = cli.command;
        let ContainerOperation::LinkList(options) = command.operation else {
            panic!("expected link-list");
        };
        assert_eq!(options.verbose, 2);
        assert!(options.json);
        assert!(options.auto_fix);
    }

    #[test]
    fn check_actions_are_specific() {
        let issue = LinkCheckIssue {
            key: "missing".into(),
            kind: LinkCheckKind::MissingSymlink,
            expected: None,
            actual: None,
        };
        assert_eq!(
            check_action(&issue),
            "create the missing symlink from recorded metadata"
        );
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
