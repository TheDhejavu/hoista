//! Provider accounts: connecting (token or browser sign-in), choosing what to import,
//! rescanning after a change, and recovering a key.

pub(crate) mod browse;
pub(crate) mod choices;
pub(crate) mod enrollment;
pub(crate) mod ignored;
pub(crate) mod import_suggestions;
pub(crate) mod key_recovery;
pub(crate) mod oauth;
pub(crate) mod rescan;
pub(crate) mod sign_in;

mod service;
pub use oauth::ACCESS_TOKEN_SECONDS;
pub use service::AccountService;
