//! Focused stage-one CLI integration tests with self-contained temporary resources.

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use kako_craft_lib::container::{ConfigRule, ConfigurableContainer};
use kako_craft_lib::destination::{CatalogDestination, Subcontainer};
use kako_craft_lib::locator::{ContainerLocator, ContainerPath, DestinationLocator};
use serde_json::Value;
use uuid::Uuid;

/// Unique temporary root removed on success, assertion unwind, and early return.
struct TestDirectory {
    /// Root below the operating-system temporary directory.
    path: PathBuf,
}

impl TestDirectory {
    /// Creates one isolated empty test directory.
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "kcm-stage-one-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        fs::create_dir(&path).expect("failed to create CLI test root");
        Self { path }
    }

    /// Joins a relative path below this test root.
    fn join(&self, path: impl AsRef<Path>) -> PathBuf {
        self.path.join(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            panic!(
                "failed to remove CLI test root {}: {error}",
                self.path.display()
            );
        }
    }
}

/// Runs kcm with captured streams and optional standard input.
fn run(arguments: impl IntoIterator<Item = OsString>, input: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kcm"));
    command
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = command.spawn().expect("failed to spawn kcm");
    if let Some(input) = input {
        child
            .stdin
            .take()
            .expect("missing child stdin")
            .write_all(input.as_bytes())
            .expect("failed to write child stdin");
    }
    child.wait_with_output().expect("failed to wait for kcm")
}

/// Runs kcm and requires a successful status.
fn success(arguments: impl IntoIterator<Item = OsString>) -> Output {
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    let output = run(arguments.clone(), None);
    assert!(
        output.status.success(),
        "kcm {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Converts borrowed string arguments into owned process arguments.
fn words(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

/// Prefixes a Container operation with its locator.
fn container(locator: &ContainerLocator, operation: &[&str]) -> Vec<OsString> {
    let mut arguments = vec![OsString::from("container"), locator.to_string().into()];
    arguments.extend(operation.iter().map(OsString::from));
    arguments
}

/// Parses captured stdout as JSON.
fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON ({error}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[test]
fn minecraft_destination_init_uses_the_library_provider() {
    let directory = TestDirectory::new();
    let root = directory.join("minecraft");

    let initialized = success([
        OsString::from("destination"),
        root.as_os_str().to_owned(),
        OsString::from("init"),
        OsString::from("minecraft"),
    ]);
    let output = String::from_utf8(initialized.stdout).unwrap();
    assert!(output.contains("kind: minecraft\n"));

    let destination_metadata: Value =
        serde_json::from_slice(&fs::read(root.join(".kcl/destination.json")).unwrap()).unwrap();
    assert_eq!(destination_metadata["kind"], "minecraft");
    let cache_metadata: Value =
        serde_json::from_slice(&fs::read(root.join(".kcl/mod-cache/.kcl/container.json")).unwrap())
            .unwrap();
    assert_eq!(cache_metadata["kind"], "local");

    let listing = success([
        OsString::from("destination"),
        root.as_os_str().to_owned(),
        OsString::from("list"),
        OsString::from("--json"),
    ]);
    assert_eq!(json(&listing).as_array().unwrap().len(), 4);
}

#[test]
fn locator_destination_and_configurable_crud_work_end_to_end() {
    let directory = TestDirectory::new();
    let destination_root = directory.join("destination");
    let destination = CatalogDestination::new(&destination_root).unwrap();
    destination
        .add_container("cache", "local", None, false)
        .unwrap();
    destination
        .add_container("version", "configurable", None, false)
        .unwrap();
    destination
        .add_container("manual-target", "local", None, false)
        .unwrap();
    destination
        .add_container("group/nested", "local", None, false)
        .unwrap();
    let cache = destination.open_container("cache").unwrap();
    let version = destination.open_container("version").unwrap();
    let configurable =
        ConfigurableContainer::from_metadata(version.root_path(), version.metadata()).unwrap();
    let destination_locator = DestinationLocator::new(destination_root.to_string_lossy()).unwrap();
    let cache_locator = ContainerLocator::new(
        Some(destination_locator.clone()),
        ContainerPath::new("cache").unwrap(),
    );
    let manual_target_locator = ContainerLocator::new(
        Some(destination_locator.clone()),
        ContainerPath::new("manual-target").unwrap(),
    );
    configurable
        .set_rules(vec![ConfigRule::sha1(
            "/mods/**",
            cache_locator,
            cache.uid().unwrap(),
        )])
        .unwrap();
    let version_locator = ContainerLocator::new(
        Some(destination_locator),
        ContainerPath::new("version").unwrap(),
    );

    let listing = success([
        OsString::from("destination"),
        destination_root.as_os_str().to_owned(),
        OsString::from("list"),
        OsString::from("--json"),
    ]);
    let members = json(&listing);
    assert_eq!(members.as_array().unwrap().len(), 4);

    let nested_listing = success([
        OsString::from("destination"),
        destination_root.as_os_str().to_owned(),
        OsString::from("list"),
        OsString::from("group/"),
        OsString::from("--json"),
    ]);
    let nested_members = json(&nested_listing);
    assert_eq!(nested_members.as_array().unwrap().len(), 1);

    let recursive_listing = success([
        OsString::from("destination"),
        destination_root.as_os_str().to_owned(),
        OsString::from("list"),
        OsString::from("-R"),
        OsString::from("--json"),
    ]);
    let recursive_members = json(&recursive_listing);
    assert_eq!(recursive_members.as_array().unwrap().len(), 5);
    assert!(recursive_members.as_array().unwrap().iter().any(|member| {
        member["member_type"] == "container" && member["logical_path"] == "group/nested"
    }));

    let add = success(container(
        &version_locator,
        &["add", "mods/a.jar", "--content", "payload"],
    ));
    assert!(add.stderr.is_empty(), "logs must be disabled by default");
    assert_eq!(
        String::from_utf8(success(container(&version_locator, &["read", "mods/a.jar"])).stdout)
            .unwrap(),
        "payload"
    );
    success(container(
        &version_locator,
        &["rename", "mods/a.jar", "outside.jar"],
    ));
    success(container(
        &version_locator,
        &["update", "outside.jar", "--content", "local-now"],
    ));
    success(container(
        &version_locator,
        &["copy", "outside.jar", "temporary-copy.jar"],
    ));
    assert_eq!(
        String::from_utf8(
            success(container(&version_locator, &["read", "temporary-copy.jar"])).stdout
        )
        .unwrap(),
        "local-now"
    );
    success(container(
        &version_locator,
        &["remove", "temporary-copy.jar"],
    ));
    assert_eq!(
        String::from_utf8(success(container(&version_locator, &["list"])).stdout).unwrap(),
        "outside.jar\n"
    );

    let warning = run(
        container(
            &version_locator,
            &["local-add", "mods/warned.jar", "--content", "forced"],
        ),
        Some("n\n"),
    );
    assert!(!warning.status.success());
    assert!(
        String::from_utf8_lossy(&warning.stderr)
            .contains("next ordinary write/update will use Link")
    );

    let mut forced = words(&["-y"]);
    forced.extend(container(
        &version_locator,
        &["local-add", "mods/forced.jar", "--content", "forced"],
    ));
    success(forced);

    success(container(
        &manual_target_locator,
        &["add", "source.txt", "--content", "manual"],
    ));
    let manual_target_text = manual_target_locator.to_string();
    let link_warning = run(
        container(
            &version_locator,
            &["link", "manual-link", &manual_target_text, "source.txt"],
        ),
        Some("n\n"),
    );
    assert!(!link_warning.status.success());
    assert!(String::from_utf8_lossy(&link_warning.stderr).contains("next ordinary write/update"));
    let mut forced_link = words(&["-y"]);
    forced_link.extend(container(
        &version_locator,
        &["link", "manual-link", &manual_target_text, "source.txt"],
    ));
    success(forced_link);
    assert_eq!(
        String::from_utf8(success(container(&version_locator, &["read", "manual-link"])).stdout)
            .unwrap(),
        "manual"
    );
    success(container(&version_locator, &["link-remove", "manual-link"]));

    let mut logged = words(&["--log-level", "info"]);
    logged.extend(container(
        &version_locator,
        &["add", "mods/logged.jar", "--content", "logged"],
    ));
    let logged = success(logged);
    assert!(String::from_utf8_lossy(&logged.stderr).contains("selected configurable route"));

    let mut logged_json = words(&["--log-level", "info"]);
    logged_json.extend(container(&version_locator, &["list", "--json"]));
    let logged_json = success(logged_json);
    assert!(json(&logged_json).is_array());
}

#[test]
fn fake_locator_initializes_without_legacy_path_parsing() {
    let directory = TestDirectory::new();
    let path = directory.join("plain");
    let locator: ContainerLocator = format!(":{}", path.display()).parse().unwrap();
    success(container(&locator, &["init", "local"]));
    success(container(&locator, &["add", "entry", "--content", "data"]));
    assert_eq!(
        String::from_utf8(success(container(&locator, &["read", "entry"])).stdout).unwrap(),
        "data"
    );

    let output = run(
        [
            OsString::from("container"),
            path.as_os_str().to_owned(),
            OsString::from("info"),
        ],
        None,
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("expected 2 locator fields"));
}
