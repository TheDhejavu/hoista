//! Where the person has moved things on the project map. Only positions live
//! here; the resources and links are kept with the project.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The most positions a map keeps.
const MAX_POSITIONS: usize = 1000;
/// The longest node id accepted.
const MAX_NODE_ID_BYTES: usize = 512;
/// How far from the centre a node may be placed, either way.
const MAX_COORDINATE: f64 = 1_000_000.0;
/// The largest whole number JavaScript holds exactly (`Number.MAX_SAFE_INTEGER`).
const MAX_REVISION: i64 = 9_007_199_254_740_991;

/// Where one node sits on the map.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct MapPosition {
    /// The node's id, as the project map gives it.
    pub node_id: String,
    /// Across.
    pub x: f64,
    /// Down.
    pub y: f64,
}

/// A project map's saved positions. No positions means the map lays itself out.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct MapLayout {
    /// Goes up by one on every save. Send back the one you read; a save made
    /// on an older one is refused, so two windows can't overwrite each other.
    #[ts(type = "number")]
    pub revision: i64,
    /// Every saved position. Saving an empty list goes back to the automatic layout.
    pub positions: Vec<MapPosition>,
}

impl MapLayout {
    /// Whether this is a layout Hoista will save: sensible coordinates, each
    /// node at most once, and a revision JavaScript can hold exactly.
    pub fn valid(&self) -> bool {
        let mut ids = std::collections::HashSet::new();
        (0..=MAX_REVISION).contains(&self.revision)
            && self.positions.len() <= MAX_POSITIONS
            && self.positions.iter().all(|position| {
                !position.node_id.is_empty()
                    && position.node_id.len() <= MAX_NODE_ID_BYTES
                    && !position.node_id.chars().any(char::is_control)
                    && ids.insert(&position.node_id)
                    && position.x.is_finite()
                    && position.y.is_finite()
                    && position.x.abs() <= MAX_COORDINATE
                    && position.y.abs() <= MAX_COORDINATE
            })
    }
}
