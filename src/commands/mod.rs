use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use kako_craft_lib::container::{
    Container, ContainerMetadata, EntryKey, LinkContainer, LocalContainer, open_container,
};

use crate::cli::*;

pub(crate) mod interaction;
pub(crate) mod render;

use interaction::*;
use render::*;

pub(crate) fn run(cli: Cli) -> Result<()> {
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
