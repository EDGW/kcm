//! Locator-aware command dispatch for the Container command family.

use anyhow::Result;
use kako_craft_lib::container::{StorageClass, open_container};
use kako_craft_lib::locator::resolve_container;

use crate::cli::*;

pub(crate) mod interaction;
mod io;
mod lifecycle;
mod links;
mod local;
pub(crate) mod render;
pub(crate) mod tests;

use interaction::*;
use io::*;
use lifecycle::*;
use links::*;
use local::*;
use render::*;
use tests::run_container_tests;

/// Dispatches one parsed Container invocation through library locator resolution.
///
/// # Arguments
///
/// * `command` - Container locator and exactly one operation.
/// * `yes` - Whether storage-property warnings are accepted without prompting.
///
/// # Returns
///
/// `Ok(())` after the selected command completes and emits its output.
///
/// # Errors
///
/// Returns any locator, Container, validation, I/O, rendering, or interaction error.
pub(crate) fn run(command: ContainerCommand, yes: bool) -> Result<()> {
    let pwd = std::env::current_dir()?;
    let operation = match command.operation {
        ContainerOperation::Init(args) => return initialize(&command.locator, &pwd, args),
        ContainerOperation::Tests(command) => return run_container_tests(command),
        operation => operation,
    };
    let container = resolve_container(&command.locator, &pwd)?;
    let path = container.root_path();
    match operation {
        ContainerOperation::Info(format) => print_info(&path, format.json),
        ContainerOperation::List(options) => print_container_list(&path, options),
        ContainerOperation::Init(_) | ContainerOperation::Tests(_) => unreachable!("handled above"),
        ContainerOperation::Add(args) => put(container.as_ref(), args, true),
        ContainerOperation::Update(args) => put(container.as_ref(), args, false),
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
        ContainerOperation::Remove(args) => remove(container.as_ref(), args.entries),
        ContainerOperation::Rename(args) => rename(container.as_ref(), args),
        ContainerOperation::Copy(args) => copy(container.as_ref(), args),
        ContainerOperation::LocalList(options) => print_local_list(&path, options),
        ContainerOperation::LocalAdd(args) => {
            warn_forced(&path, &args.entry, StorageClass::Local, yes)?;
            local_put(&path, args, true)
        }
        ContainerOperation::LocalUpdate(args) => {
            warn_forced(&path, &args.entry, StorageClass::Local, yes)?;
            local_put(&path, args, false)
        }
        ContainerOperation::LocalRead(args) => {
            output_data(local_read(&path, &args.entry)?, args.output)
        }
        ContainerOperation::LocalPath(args) => {
            println!("{}", local_filepath(&path, &args.entry)?.display());
            Ok(())
        }
        ContainerOperation::LocalRemove(args) => local_remove(&path, args.entries),
        ContainerOperation::LocalRename(args) => {
            warn_forced(&path, &args.to, StorageClass::Local, yes)?;
            local_rename(&path, args)
        }
        ContainerOperation::LocalCopy(args) => {
            warn_forced(&path, &args.to, StorageClass::Local, yes)?;
            local_copy(&path, args)
        }
        ContainerOperation::Link(args) => {
            warn_forced(&path, &args.linker_key, StorageClass::Link, yes)?;
            link(&path, &pwd, args)
        }
        ContainerOperation::LinkList(options) => print_link_list(&path, options),
        ContainerOperation::LinkCopy(args) => {
            warn_forced(&path, &args.to, StorageClass::Link, yes)?;
            link_copy(&path, args)
        }
        ContainerOperation::LinkRename(args) => {
            warn_forced(&path, &args.to, StorageClass::Link, yes)?;
            link_rename(&path, args)
        }
        ContainerOperation::LinkRemove(args) => unlink(&path, args),
        ContainerOperation::LinkInfo(args) => link_info(&path, args),
    }
}
