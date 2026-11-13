//! Local diagnostics for `hoist doctor`. Checks that make no provider request say so.

use crate::{
    application::StateStore,
    contracts::{CheckStatus, DoctorCheck, DoctorReport},
};
use tokio::sync::OwnedSemaphorePermit;

/// Checks state and native credential access without contacting any provider.
/// The permit stays with the blocking worker even if its requester disconnects.
pub async fn inspect(
    store: &impl StateStore,
    mode: &str,
    permit: OwnedSemaphorePermit,
) -> DoctorReport {
    let state = match store.profile().await {
        Ok(_) => DoctorCheck {
            name: "sqlite".into(),
            status: CheckStatus::Ok,
            message: "Local state is readable; migrations are current.".into(),
        },
        Err(error) => DoctorCheck {
            name: "sqlite".into(),
            status: CheckStatus::ActionNeeded,
            message: error.to_string(),
        },
    };
    let keychain = match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        crate::wiring::check_system_keychain()
    })
    .await
    {
        Ok(Ok(())) => DoctorCheck {
            name: "keychain".into(),
            status: CheckStatus::Ok,
            message: "System keychain write, read and cleanup succeeded.".into(),
        },
        Ok(Err(error)) => DoctorCheck {
            name: "keychain".into(),
            status: CheckStatus::ActionNeeded,
            message: error.to_string(),
        },
        Err(_) => DoctorCheck {
            name: "keychain".into(),
            status: CheckStatus::ActionNeeded,
            message: "Keychain worker failed. Run hoist doctor again.".into(),
        },
    };
    DoctorReport {
        network: Some(crate::wiring::network_report()),
        accounts: None,
        version: crate::VERSION.into(),
        mode: mode.into(),
        checks: vec![
            state,
            keychain,
            DoctorCheck {
                name: "provider_accounts".into(),
                status: CheckStatus::NotChecked,
                message: "No provider request was made by this local diagnostic. Check them with hoist accounts check, or in the app.".into(),
            },
            DoctorCheck {
                name: "network".into(),
                status: CheckStatus::NotChecked,
                message: "Network reachability was not tested. Provider requests use the published provider allow-list.".into(),
            },
        ],
    }
}

/// Adds the account checks, which call each provider once and change nothing saved.
pub async fn accounts(
    mut report: DoctorReport,
    engine: &crate::application::engine::Engine,
) -> DoctorReport {
    use crate::application::engine::{AccountDiagnosticStatus, Actor};
    report
        .checks
        .retain(|check| check.name != "provider_accounts");
    match engine
        .observe()
        .diagnose_accounts(&Actor::Person, None)
        .await
    {
        Ok(page) => {
            let failed = page.items.iter().any(|item| {
                !matches!(
                    item.status,
                    AccountDiagnosticStatus::Verified
                        | AccountDiagnosticStatus::Disconnected
                        | AccountDiagnosticStatus::NotChecked
                )
            });
            let incomplete = page.next_cursor.is_some()
                || page
                    .items
                    .iter()
                    .any(|item| item.status == AccountDiagnosticStatus::NotChecked);
            report.checks.push(DoctorCheck {
                name: "provider_accounts".into(),
                status: if failed {
                    CheckStatus::ActionNeeded
                } else if incomplete || page.items.is_empty() {
                    CheckStatus::NotChecked
                } else {
                    CheckStatus::Ok
                },
                message: if failed {
                    "Some account probes need attention. See individual account diagnostics."
                } else if incomplete {
                    "Account checks are incomplete. Retry or continue through /doctor/accounts using next_cursor."
                } else if page.items.is_empty() {
                    "No provider accounts are configured."
                } else {
                    "Account probes completed. Disconnected accounts were skipped; write permissions are not comprehensively verified."
                }
                .into(),
            });
            report.accounts = Some(page);
        }
        Err(_) => report.checks.push(DoctorCheck {
            name: "provider_accounts".into(),
            status: CheckStatus::ActionNeeded,
            message: "Account diagnostics could not run. Check local state or retry after the current diagnostic finishes.".into(),
        }),
    }
    report.network = Some(crate::wiring::network_report());
    report.checks.retain(|check| check.name != "network");
    report.checks.push(DoctorCheck {
        name: "network".into(),
        status: CheckStatus::NotChecked,
        message: "See network for owned HTTP attempts and the published API hosts. Coverage is partial; this is not a comprehensive reachability test.".into(),
    });
    report
}
