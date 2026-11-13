//! What the use cases need from outside, as traits: the catalog, provider adapters,
//! the credential keeper, builds, database exports and the owner check. Other crates
//! implement them; the use cases never name a database, HTTP library or provider API.

pub mod backups;
pub mod build;
pub mod catalog;
pub mod database_export;
pub mod keeper;
pub mod network;
pub mod owner_check;
pub mod providers;
pub mod sign_in_page;
pub mod transfer;
pub mod vault;
