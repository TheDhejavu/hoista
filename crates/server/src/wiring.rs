//! Builds the engine with its real adapters (store, vaults, providers, builds),
//! for the daemon and for CLI commands that open the state directly.

use crate::infrastructure::token_store::StoredVault;
use crate::infrastructure::{
    build::LocalBuilder,
    credentials::NativeVault,
    database_export::PostgresExporter,
    inspection::LocalInspector,
    providers::{adapters, http::ProviderHttp},
    store::SqliteStore,
};
use crate::{
    application::{engine::Engine, ports::catalog::EngineError},
    domain::model::VaultPlace,
};
use hoista_system::paths::StatePaths;
use std::sync::Arc;

/// The engine on this computer, with the keychain vault. Reads no credentials and
/// starts no workers.
///
/// # Errors
/// Fails when the provider HTTP clients can't be built.
pub fn engine(store: SqliteStore, paths: &StatePaths) -> Result<Engine, EngineError> {
    let vault = StoredVault::new(
        Arc::new(NativeVault::default().traced(&paths.logs())),
        VaultPlace::Keychain,
        Arc::from(chosen_store()?),
    );
    let info = vault.info(None);
    Ok(build(
        store,
        paths,
        with_references(Arc::new(vault)),
        ProviderHttp::new()?,
    )?
    .with_vault_info(info)
    .with_owner_check(Arc::new(
        crate::infrastructure::owner_check::SystemOwnerCheck,
    )))
}

/// The token store `HOISTA_TOKEN_STORE` names, on a computer as on a remote.
fn chosen_store() -> Result<Box<dyn crate::infrastructure::token_store::TokenStore>, EngineError> {
    crate::infrastructure::token_store::token_store(
        std::env::var(crate::infrastructure::token_store::TOKEN_STORE_VARIABLE)
            .ok()
            .as_deref(),
    )
}

/// The key source `HOISTA_KEY_SOURCE` names, by its name.
fn chosen_key_source() -> Result<&'static str, EngineError> {
    Ok(crate::infrastructure::token_store::key_source(
        std::env::var(crate::infrastructure::token_store::KEY_SOURCE_VARIABLE)
            .ok()
            .as_deref(),
    )?
    .name())
}

/// Hoista on a server reachable at `public_url`: provider tokens sealed in the
/// state folder under `sealed_key` (kept as a provider secret, never in the
/// folder), and the owner confirms with a passkey.
///
/// # Errors
/// Rejects a malformed key and reports TLS client failures.
pub fn cloud_engine(
    store: SqliteStore,
    paths: &StatePaths,
    public_url: &str,
    sealed_key: &str,
) -> Result<Engine, EngineError> {
    let sealed = crate::infrastructure::sealed_store::SealedStore::new(
        paths.root().join("vault.sealed"),
        sealed_key,
    )
    .map_err(|_| EngineError::Invalid("HOISTA_SEALED_KEY must be 32 bytes of base64."))?;
    let vault = StoredVault::new(
        Arc::new(NativeVault::in_store("engine", Arc::new(sealed))),
        VaultPlace::SealedFile,
        Arc::from(chosen_store()?),
    );
    let info = vault.info(Some(chosen_key_source()?));
    Ok(build(
        store,
        paths,
        with_references(Arc::new(vault)),
        ProviderHttp::new()?,
    )?
    .with_vault_info(info)
    .with_public_url(public_url.to_owned())
    .with_owner_check(Arc::new(
        crate::application::ports::owner_check::PasskeyCheck,
    )))
}

/// Hoista on a server with its credential keeper: this process holds
/// handles, never provider tokens; the keeper signs provider requests in.
///
/// # Errors
/// Reports TLS client failures.
pub fn kept_cloud_engine(
    store: SqliteStore,
    paths: &StatePaths,
    public_url: &str,
    keeper: Arc<crate::infrastructure::keeper::KeeperClient>,
) -> Result<Engine, EngineError> {
    // The keeper holds the vault; this process only says where it is.
    let info = crate::domain::model::VaultInfo {
        kept_in: VaultPlace::SealedFile,
        token_store: chosen_store()?.name().to_owned(),
        key_source: Some(chosen_key_source()?.to_owned()),
        references: true,
    };
    Ok(build(
        store,
        paths,
        Arc::new(crate::infrastructure::keeper::HandleVault::new(
            keeper.clone(),
        )),
        ProviderHttp::new()?.with_keeper(keeper),
    )?
    .with_vault_info(info)
    .with_public_url(public_url.to_owned())
    .with_owner_check(Arc::new(
        crate::application::ports::owner_check::PasskeyCheck,
    )))
}

/// The vault the keeper holds on a server: sealed under `sealed_key`, with
/// 1Password references read where it runs.
///
/// # Errors
/// Rejects a malformed key.
pub fn keeper_vault(
    paths: &StatePaths,
    sealed_key: &str,
) -> Result<Arc<dyn crate::application::ports::vault::TokenVault>, EngineError> {
    let sealed = crate::infrastructure::sealed_store::SealedStore::new(
        paths.root().join("keeper").join("vault.sealed"),
        sealed_key,
    )
    .map_err(|_| EngineError::Invalid("HOISTA_SEALED_KEY must be 32 bytes of base64."))?;
    // References are read by the keeper itself, after its token store.
    Ok(Arc::new(NativeVault::in_store("engine", Arc::new(sealed))))
}

/// Any entry may be a 1Password reference instead of a token.
fn with_references(
    vault: Arc<dyn crate::application::ports::vault::TokenVault>,
) -> Arc<dyn crate::application::ports::vault::TokenVault> {
    Arc::new(crate::infrastructure::onepassword::ReferencedVault::new(
        vault,
    ))
}

fn build(
    store: SqliteStore,
    paths: &StatePaths,
    vault: Arc<dyn crate::application::ports::vault::TokenVault>,
    http: ProviderHttp,
) -> Result<Engine, EngineError> {
    let http = http.with_tools(paths.root().join("tools"));
    let backups = Arc::new(crate::infrastructure::backups::LocalBackups::new(paths));
    Ok(Engine::new(
        Arc::new(store),
        vault,
        Arc::new(LocalInspector),
        adapters(&http),
        Arc::new(LocalBuilder::new(paths.root())),
    )
    .with_database_exporter(Arc::new(PostgresExporter::new(paths.root())))
    .with_backup_folder(backups)
    .with_transfer_seal(Arc::new(
        crate::infrastructure::transfer_seal::LocalTransferSeal::default(),
    ))
    .with_sign_in_page(Arc::new(crate::views::ProviderSignInPage)))
}

/// Checks the system keychain can store, read and remove an entry.
///
/// # Errors
/// Reports a keychain that refused one of those steps.
pub fn check_system_keychain() -> Result<(), crate::application::CredentialError> {
    crate::application::check_keychain(&crate::infrastructure::credentials::SystemKeychain)
}

/// Which hosts this process contacted, for diagnostics.
#[must_use]
pub fn network_report() -> crate::application::ports::network::NetworkReport {
    crate::infrastructure::providers::network_history::report()
}

/// Saves a diagnostic report beside Hoista's state.
///
/// # Errors
/// Reports a folder that can't be written or a report over its size limit.
pub fn save_diagnostic_report(
    paths: &StatePaths,
    report: &crate::application::observe::diagnostic_report::DiagnosticReport,
) -> std::io::Result<crate::application::observe::diagnostic_report::DiagnosticReceipt> {
    crate::infrastructure::diagnostic_report::save(paths, report)
}

/// A provider's logo, as SVG.
#[must_use]
pub fn provider_logo(provider: crate::Provider) -> &'static str {
    crate::infrastructure::providers::logo(provider)
}

/// Sets up a coding tool to use Hoista: its skills and, for Claude Code, the
/// rule that lets Hoista's tools run. `None` when there's no home folder.
#[must_use]
pub fn set_up_tool(
    tool: crate::application::agents::tool::Tool,
) -> Option<std::io::Result<Vec<String>>> {
    let home = crate::infrastructure::agent_setup::home()?;
    Some(crate::infrastructure::agent_setup::set_up(tool, &home))
}
