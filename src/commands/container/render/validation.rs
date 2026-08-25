//! Validation status classification, structured serialization, and report rendering.

use anyhow::Result;
use kako_craft_lib::container::{
    LinkValidationIssue, LinkValidationIssueKind, LinkValidationReport,
};
use serde_json::json;

use crate::ValidationCommandError;

/// Selects the stable aggregate status label for a validation report.
///
/// # Arguments
///
/// * `report` - Report whose broken, unavailable, valid, and ignored results are prioritized.
///
/// # Returns
///
/// `broken` when persistent issues exist, otherwise `unavailable` when peers are inaccessible,
/// otherwise `verified` when at least one match exists, and `ignored` for an empty/ignored report.
pub(super) fn validation_status(report: &LinkValidationReport) -> &'static str {
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

/// Serializes every validation category and ignored counter into the stable CLI JSON schema.
///
/// # Arguments
///
/// * `report` - Complete library report to serialize without consuming it.
///
/// # Returns
///
/// A JSON object containing `status`, full `valid`, `broken`, and `unavailable` arrays, plus the
/// three ignored counters.
pub(crate) fn validation_json(report: &LinkValidationReport) -> serde_json::Value {
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

/// Serializes one persistent inconsistency with all optional comparison context preserved.
///
/// # Arguments
///
/// * `issue` - Broken validation item to encode.
///
/// # Returns
///
/// A JSON object containing its stable kind label, both container contexts, both optional keys,
/// and optional expected and actual values.
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

/// Maps a library validation issue kind to its stable snake-case CLI identifier.
///
/// # Arguments
///
/// * `kind` - Exhaustive library inconsistency category to translate.
///
/// # Returns
///
/// The static snake-case label used by JSON output and interactive issue descriptions.
pub(crate) fn validation_issue_kind(kind: LinkValidationIssueKind) -> &'static str {
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

/// Prints a complete human-readable validation summary and detailed unresolved items.
///
/// # Arguments
///
/// * `report` - Report whose category counts, ignored counters, broken issues, and unavailable peers
///   are written to stdout.
///
/// # Returns
///
/// Returns after all lines are emitted. Standard `println!` handling applies if stdout is closed.
pub(super) fn print_validation_report(report: &LinkValidationReport) {
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

/// Converts aggregate broken or unavailable report counts into the CLI's typed command error.
///
/// # Arguments
///
/// * `reports` - Any iterable of borrowed reports included in the current rendered command.
///
/// # Returns
///
/// `Ok(())` when every report has zero broken and unavailable items.
///
/// # Errors
///
/// Returns [`ValidationCommandError::Broken`] with the total broken count when nonzero; otherwise
/// returns [`ValidationCommandError::Unavailable`] with the total unavailable count when nonzero.
pub(super) fn ensure_validation_success<'a>(
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
