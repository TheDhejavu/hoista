//! Projects and their resources: the active project, links to sources, the overview,
//! the map, deleting, and what needs the person's attention.

pub(crate) mod active;
pub(crate) mod attention;
pub(crate) mod bindings;
pub(crate) mod deletion;
pub mod description;
pub(crate) mod map;
pub(crate) mod overview;
pub(crate) mod resource;

mod service;
pub use service::ProjectService;
