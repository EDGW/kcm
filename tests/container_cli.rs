#![cfg(unix)]

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use kako_craft_lib::container::open_container;
use serde_json::Value;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "kcm-{name}-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn join(&self, path: impl AsRef<Path>) -> PathBuf {
        self.0.join(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("failed to remove {}: {error}", self.0.display());
        }
    }
}

fn words(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn run(args: &[OsString], input: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kcm"));
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = command.spawn().unwrap();
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}

fn container_args(path: &Path, operation: &[&str]) -> Vec<OsString> {
    let mut args = vec![OsString::from("container"), path.as_os_str().to_owned()];
    args.extend(operation.iter().map(OsString::from));
    args
}

fn success(path: &Path, operation: &[&str]) -> Output {
    let args = container_args(path, operation);
    let output = run(&args, None);
    assert!(
        output.status.success(),
        "kcm {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn success_with_input(path: &Path, operation: &[&str], input: &str) -> Output {
    let args = container_args(path, operation);
    let output = run(&args, Some(input));
    assert!(
        output.status.success(),
        "kcm {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn failure(path: &Path, operation: &[&str], code: i32) -> Output {
    let args = container_args(path, operation);
    let output = run(&args, None);
    assert_eq!(
        output.status.code(),
        Some(code),
        "kcm {args:?} returned unexpected status; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn link(path: &Path, linker_key: &str, target: &Path, target_key: &str, option: Option<&str>) {
    let mut args = vec![
        OsString::from("container"),
        path.as_os_str().to_owned(),
        OsString::from("link"),
        OsString::from(linker_key),
        target.as_os_str().to_owned(),
        OsString::from(target_key),
    ];
    if let Some(option) = option {
        args.push(OsString::from(option));
    }
    let output = run(&args, None);
    assert!(
        output.status.success(),
        "link failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn complete_local_and_link_command_surface_works_end_to_end() {
    let root = TestDirectory::new("command-surface");
    let local = root.join("local");
    let links = root.join("links");

    success(&local, &["init", "local"]);
    let local_info = json(&success(&local, &["info", "--json"]));
    assert_eq!(local_info["kind"], "local");
    assert!(local_info["prefer_relative"].is_null());
    success(&local, &["add", "entry", "--content", "one"]);
    success(&local, &["update", "entry", "--content", "two"]);
    assert_eq!(stdout(&success(&local, &["read", "entry"])), "two");
    assert_eq!(stdout(&success(&local, &["get", "entry"])), "two");
    assert!(stdout(&success(&local, &["path", "entry"])).ends_with("local/entry\n"));
    assert_eq!(stdout(&success(&local, &["list"])), "entry\n");

    success(&local, &["copy", "entry", "copy"]);
    success(&local, &["cp", "copy", "copy-2"]);
    success(&local, &["rename", "copy", "renamed"]);
    success(&local, &["move", "renamed", "moved"]);
    success(&local, &["mv", "moved", "moved-again"]);
    success(&local, &["delete", "moved-again"]);
    success(&local, &["rm", "copy-2"]);
    success(&local, &["add", "batch-a", "--content", "a"]);
    success(&local, &["add", "batch-b", "--content", "b"]);
    success(&local, &["remove", "batch-a", "batch-b"]);

    success(&links, &["init", "link"]);
    assert_eq!(
        json(&success(&links, &["info", "--json"]))["prefer_relative"],
        true
    );
    success(&links, &["local-add", "own", "--content", "first"]);
    success(&links, &["local-update", "own", "--content", "second"]);
    assert_eq!(stdout(&success(&links, &["local-read", "own"])), "second");
    assert!(stdout(&success(&links, &["local-path", "own"])).ends_with("links/own\n"));
    success(&links, &["local-copy", "own", "own-copy"]);
    success(&links, &["local-rename", "own-copy", "own-renamed"]);
    success(&links, &["local-remove", "own-renamed"]);
    assert_eq!(stdout(&success(&links, &["local-list"])), "own\n");

    link(&links, "ref", &local, "entry", None);
    assert_eq!(stdout(&success(&links, &["read", "ref"])), "two");
    assert!(stdout(&success(&links, &["path", "ref"])).ends_with("links/ref\n"));
    assert_eq!(stdout(&success(&links, &["link-list"])), "ref\n");
    assert!(stdout(&success(&links, &["list"])).contains("ref\n"));
    assert_eq!(
        json(&success(&links, &["link-info", "ref", "--json"]))["link_info"]["kind"],
        "to"
    );
    success(&links, &["link-copy", "ref", "ref-copy"]);
    success(&links, &["link-rename", "ref-copy", "ref-renamed"]);
    success(&links, &["unlink", "ref-renamed"]);
    success(&links, &["link-remove", "ref"]);
    success(&links, &["check"]);

    let wrong_local = failure(&links, &["add", "wrong", "--content", "x"], 1);
    assert!(stderr(&wrong_local).contains("use a local-* operation"));
    let wrong_link = failure(&local, &["link-list"], 1);
    assert!(stderr(&wrong_link).contains("requires a link container"));
}

#[test]
fn validation_lists_check_autofix_and_exit_codes_follow_the_contract() {
    let root = TestDirectory::new("validation");
    let target = root.join("target");
    let first = root.join("first");
    let second = root.join("second");
    let unrelated = root.join("unrelated");

    success(&target, &["init", "local"]);
    success(&target, &["add", "entry", "--content", "payload"]);
    for path in [&first, &second, &unrelated] {
        success(path, &["init", "link"]);
    }
    success(&first, &["local-add", "own", "--content", "local"]);
    link(&first, "ref-1", &target, "entry", None);
    link(&second, "ref-2", &target, "entry", None);

    let mut args = container_args(&target, &["link-info", "entry", "--validate-with"]);
    args.extend([
        first.as_os_str().to_owned(),
        second.as_os_str().to_owned(),
        unrelated.as_os_str().to_owned(),
        OsString::from("--json"),
    ]);
    let output = run(&args, None);
    assert!(output.status.success(), "{}", stderr(&output));
    let value = json(&output);
    assert_eq!(value["link_info"]["linkers"].as_array().unwrap().len(), 2);
    let source_order = value["link_info"]["linkers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|source| {
            (
                source["container_uid"].as_str().unwrap(),
                source["linker_key"].as_str().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert!(source_order.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(value["validation"]["valid"].as_array().unwrap().len(), 2);
    assert_eq!(value["validation"]["ignored"]["containers"], 1);

    let plain = json(&success(&first, &["list", "--json"]));
    assert!(plain.as_array().unwrap().iter().all(Value::is_string));
    let verbose = json(&success(&first, &["list", "-v", "--json"]));
    assert!(verbose[0].get("size").is_some());
    assert!(verbose[0].get("filepath").is_none());
    let full = json(&success(
        &first,
        &[
            "list",
            "-vv",
            "--link-info",
            "--validate-with",
            target.to_str().unwrap(),
            "--json",
        ],
    ));
    assert!(
        full.as_array()
            .unwrap()
            .iter()
            .all(|entry| entry.get("filepath").is_some())
    );
    assert!(
        full.as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["key"] == "ref-1" && entry["validation"]["status"] == "verified")
    );
    assert_eq!(json(&success(&first, &["local-list", "--json"]))[0], "own");
    let local_verbose = json(&success(&first, &["local-list", "-v", "--json"]));
    assert!(local_verbose[0].get("size").is_some());
    assert_eq!(json(&success(&first, &["link-list", "--json"]))[0], "ref-1");
    let link_full = json(&success(
        &first,
        &["link-list", "-vv", "--link-info", "--json"],
    ));
    assert_eq!(link_full[0]["validation"]["status"], "verified");
    assert!(link_full[0].get("container_uid").is_some());

    fs::remove_file(first.join("ref-1")).unwrap();
    let broken = failure(&first, &["list", "--link-info", "--json"], 1);
    let broken_json = json(&broken);
    assert_eq!(broken_json.as_array().unwrap().len(), 2);
    assert!(
        broken_json
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["key"] == "ref-1" && entry["validation"]["status"] == "broken" })
    );

    let repaired = success_with_input(
        &first,
        &["check", "--validate-with", target.to_str().unwrap()],
        "1\n",
    );
    assert!(stderr(&repaired).contains("create missing symlink 'ref-1'"));
    assert_eq!(stdout(&success(&first, &["read", "ref-1"])), "payload");

    fs::remove_file(first.join("ref-1")).unwrap();
    let skipped_args = container_args(&first, &["read", "ref-1", "--auto-fix"]);
    let skipped = run(&skipped_args, Some("2\n"));
    assert_eq!(skipped.status.code(), Some(1));
    assert!(stderr(&skipped).contains("operation was not retried"));
    assert!(!first.join("ref-1").exists());
    success_with_input(&first, &["check"], "1\n");

    let current = open_container(&first).unwrap();
    let current_lock = current.writer().unwrap();
    let locked_current = failure(&first, &["link-info", "ref-1", "--json"], 3);
    assert!(stderr(&locked_current).contains("locked by another writer"));
    drop(current_lock);

    let corresponding = open_container(&first).unwrap();
    let corresponding_lock = corresponding.writer().unwrap();
    let locked_args = vec![
        OsString::from("container"),
        target.as_os_str().to_owned(),
        OsString::from("link-info"),
        OsString::from("entry"),
        OsString::from("--validate-with"),
        first.as_os_str().to_owned(),
    ];
    let locked = run(&locked_args, None);
    assert_eq!(locked.status.code(), Some(3));
    assert!(stdout(&locked).contains("unavailable: 1"));
    drop(corresponding_lock);
    let available = run(&locked_args, None);
    assert!(available.status.success(), "{}", stderr(&available));
    assert!(stdout(&available).contains("valid: 1"));

    let self_validation = failure(
        &target,
        &[
            "link-info",
            "entry",
            "--validate-with",
            target.to_str().unwrap(),
        ],
        2,
    );
    assert!(stderr(&self_validation).contains("cannot validate a container against itself"));

    let orphan = first.join("orphan");
    symlink("../target/entry", &orphan).unwrap();
    let orphan_repair = success_with_input(&first, &["check"], "1\n");
    assert!(stderr(&orphan_repair).contains("delete unrecorded symlink 'orphan'"));
    assert!(!orphan.exists());

    fs::remove_file(first.join("ref-1")).unwrap();
    symlink("../target/missing", first.join("ref-1")).unwrap();
    let incorrect_repair = success_with_input(&first, &["check"], "1\n");
    assert!(stderr(&incorrect_repair).contains("replace symlink 'ref-1'"));
    assert_eq!(stdout(&success(&first, &["read", "ref-1"])), "payload");
}

#[test]
fn relative_absolute_and_nested_link_paths_match_the_saved_policy() {
    let root = TestDirectory::new("path-policy");
    let target = root.join("target");
    let relative = root.join("relative");
    let absolute = root.join("absolute");

    success(&target, &["init", "local"]);
    success(&target, &["add", "entry", "--content", "data"]);
    success(&relative, &["init", "link"]);
    success(&absolute, &["init", "link", "--absolute"]);
    assert_eq!(
        json(&success(&relative, &["info", "--json"]))["prefer_relative"],
        true
    );
    assert_eq!(
        json(&success(&absolute, &["info", "--json"]))["prefer_relative"],
        false
    );

    link(&relative, "default", &target, "entry", None);
    link(
        &relative,
        "forced-absolute",
        &target,
        "entry",
        Some("--absolute"),
    );
    link(
        &relative,
        "nested/ref",
        &target,
        "entry",
        Some("--relative"),
    );
    link(&absolute, "default", &target, "entry", None);
    link(
        &absolute,
        "forced-relative",
        &target,
        "entry",
        Some("--relative"),
    );

    let relative_metadata: Value =
        serde_json::from_slice(&fs::read(relative.join(".kcl/outgoing-links.json")).unwrap())
            .unwrap();
    assert!(
        Path::new(
            relative_metadata["links"]["default"]["container_path"]
                .as_str()
                .unwrap()
        )
        .is_relative()
    );
    assert!(
        Path::new(
            relative_metadata["links"]["forced-absolute"]["container_path"]
                .as_str()
                .unwrap()
        )
        .is_absolute()
    );
    assert!(
        fs::read_link(relative.join("default"))
            .unwrap()
            .is_relative()
    );
    assert!(
        fs::read_link(relative.join("forced-absolute"))
            .unwrap()
            .is_absolute()
    );
    assert_eq!(
        fs::canonicalize(relative.join("nested/ref")).unwrap(),
        fs::canonicalize(target.join("entry")).unwrap()
    );

    let absolute_metadata: Value =
        serde_json::from_slice(&fs::read(absolute.join(".kcl/outgoing-links.json")).unwrap())
            .unwrap();
    assert!(
        Path::new(
            absolute_metadata["links"]["default"]["container_path"]
                .as_str()
                .unwrap()
        )
        .is_absolute()
    );
    assert!(
        Path::new(
            absolute_metadata["links"]["forced-relative"]["container_path"]
                .as_str()
                .unwrap()
        )
        .is_relative()
    );

    success(
        &relative,
        &["link-copy", "forced-absolute", "absolute-copy"],
    );
    success(
        &relative,
        &["link-rename", "absolute-copy", "absolute-renamed"],
    );
    let after: Value =
        serde_json::from_slice(&fs::read(relative.join(".kcl/outgoing-links.json")).unwrap())
            .unwrap();
    assert!(
        Path::new(
            after["links"]["absolute-renamed"]["container_path"]
                .as_str()
                .unwrap()
        )
        .is_absolute()
    );
    assert!(
        fs::read_link(relative.join("absolute-renamed"))
            .unwrap()
            .is_absolute()
    );
}

#[test]
fn log_levels_filter_events_and_never_pollute_json_stdout() {
    let root = TestDirectory::new("logging");
    let local = root.join("local");

    let initialized = success(&local, &["init", "local"]);
    assert!(initialized.stderr.is_empty());
    let added = success(&local, &["add", "entry", "--content", "secret-content"]);
    assert!(added.stderr.is_empty());

    let info_args = vec![
        OsString::from("--log-level"),
        OsString::from("info"),
        OsString::from("container"),
        local.as_os_str().to_owned(),
        OsString::from("update"),
        OsString::from("entry"),
        OsString::from("--content"),
        OsString::from("changed"),
    ];
    let info = run(&info_args, None);
    assert!(info.status.success());
    assert!(stderr(&info).contains("INFO"));
    assert!(!stderr(&info).contains("secret-content"));

    let debug_args = vec![
        OsString::from("container"),
        local.as_os_str().to_owned(),
        OsString::from("list"),
        OsString::from("--json"),
        OsString::from("--log-level"),
        OsString::from("debug"),
    ];
    let debug = run(&debug_args, None);
    assert!(debug.status.success());
    json(&debug);
    assert!(stderr(&debug).contains("DEBUG"));
    assert!(stderr(&debug).contains("listed container entries"));
    assert!(!stderr(&debug).contains("TRACE"));

    let trace_args = vec![
        OsString::from("--log-level"),
        OsString::from("trace"),
        OsString::from("container"),
        local.as_os_str().to_owned(),
        OsString::from("list"),
        OsString::from("--json"),
    ];
    let trace = run(&trace_args, None);
    assert!(trace.status.success());
    json(&trace);
    assert!(stderr(&trace).contains("TRACE"));
    assert!(stderr(&trace).contains("acquired file lock"));

    for level in ["warn", "error"] {
        let args = vec![
            OsString::from("--log-level"),
            OsString::from(level),
            OsString::from("container"),
            local.as_os_str().to_owned(),
            OsString::from("info"),
            OsString::from("--json"),
        ];
        let output = run(&args, None);
        assert!(output.status.success());
        json(&output);
        assert!(output.stderr.is_empty());
    }

    let invalid = run(
        &[
            OsString::from("--log-level"),
            OsString::from("silent"),
            OsString::from("container"),
            OsString::from("info"),
        ],
        None,
    );
    assert_eq!(invalid.status.code(), Some(2));
}

#[test]
fn help_exposes_groups_aliases_and_every_subcommand_detail() {
    let help = run(&words(&["container", "--help"]), None);
    assert!(help.status.success());
    let help = stdout(&help);
    for group in [
        "General container commands:",
        "Local container commands:",
        "Link container commands:",
    ] {
        assert!(help.contains(group));
    }
    for command in [
        "info",
        "list",
        "init",
        "read",
        "path",
        "linkinfo",
        "check",
        "add",
        "update",
        "remove",
        "rename",
        "copy",
        "local-list",
        "local-add",
        "local-update",
        "local-read",
        "local-path",
        "local-remove",
        "local-rename",
        "local-copy",
        "link",
        "link-list",
        "link-copy",
        "link-rename",
        "link-remove",
    ] {
        assert!(help.contains(&format!("  {command}")));
        let detail = run(&words(&["container", "help", command]), None);
        assert!(
            detail.status.success(),
            "detailed help failed for {command}: {}",
            stderr(&detail)
        );
    }
    assert!(help.contains("alias: link-info"));
    assert!(help.contains("alias: unlink"));
}
