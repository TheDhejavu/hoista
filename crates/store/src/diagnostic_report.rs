//! Saves diagnostic reports on this computer. Reports are already stripped of
//! secrets; nothing here uploads them.

use crate::{
    application::observe::diagnostic_report::{DiagnosticReceipt, DiagnosticReport},
    paths::{StatePaths, private_directory},
};
use sha2::{Digest, Sha256};
use std::io::Write;

const MAX_REPORT_BYTES: usize = 2 * 1024 * 1024;
const MAX_SAVED_REPORTS: usize = 50;

/// Saves one report as a new file, whole or not at all.
///
/// # Errors
/// Fails when the report is over 2 MiB, the folder can't be made private, or
/// 50 reports are already saved.
pub fn save(paths: &StatePaths, report: &DiagnosticReport) -> std::io::Result<DiagnosticReceipt> {
    let bytes = serde_json::to_vec_pretty(report)?;
    if bytes.len() > MAX_REPORT_BYTES {
        return Err(std::io::Error::other("Diagnostic report exceeds 2 MiB."));
    }
    let directory = paths.root().join("diagnostics");
    private_directory(&directory)?;
    if std::fs::read_dir(&directory)?
        .take(MAX_SAVED_REPORTS)
        .count()
        >= MAX_SAVED_REPORTS
    {
        return Err(std::io::Error::other(
            "Remove old diagnostic reports before saving more.",
        ));
    }
    let target = directory.join(format!("report-{}.json", uuid::Uuid::new_v4()));
    let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(&target)
        .map_err(|error| error.error)?;
    #[cfg(unix)]
    std::fs::File::open(&directory)?.sync_all()?;
    Ok(DiagnosticReceipt {
        path: target.to_string_lossy().into_owned(),
        bytes: bytes.len() as u32,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
    })
}
