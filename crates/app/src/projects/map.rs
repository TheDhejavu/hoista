//! A project's map, built from what Hoista has recorded. It says when the provider's
//! relationships aren't fully known.

use crate::engine::Actor;
use crate::projects::ProjectService;
use crate::{
    domain::{domains::CustomDomain, model::*},
    ports::catalog::EngineError,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A project's map: its resources and links as recorded, and the graph to draw.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct ProjectMap {
    /// Saved canvas positions; positions of unknown or removed nodes are kept but not drawn.
    pub layout: crate::domain::map_layout::MapLayout,
    /// Linked source, or the unlinked state.
    pub source: Source,
    /// The project's resources.
    pub nodes: Vec<Resource>,
    /// Recorded resource links.
    pub edges: Vec<Link>,
    /// Source, resource and custom-domain nodes with typed edges.
    pub graph: MapGraph,
}

/// The graph to draw, and whether its relationships are fully known.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct MapGraph {
    /// Nodes. Ids are prefixed by node type, so a source, a resource and a domain never clash.
    pub nodes: Vec<MapNode>,
    /// Relationships recorded locally, including ones no longer seen.
    pub edges: Vec<MapEdge>,
    /// False until the provider's relationships have all been discovered.
    pub relationships_complete: bool,
}

/// A graph node: a source, a resource or a domain.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MapNode {
    /// Linked deployment source. Unlinked imports have no source node.
    Source {
        /// Node id.
        id: String,
        /// The linked folder or repository.
        source: Source,
    },
    /// A resource in the project, or a shared one directly linked to it.
    Resource {
        /// Node id.
        id: String,
        /// The resource: provider, kind, status and project.
        resource: Resource,
        /// Latest recorded daily usage; not live traffic, and null rather than zero when unknown.
        latest_usage: Option<Usage>,
        /// Last recorded release schedules; null means unknown, empty means none.
        schedules: Option<Vec<String>>,
    },
    /// A custom hostname, with its DNS and certificate status and when they were checked.
    Domain {
        /// Node id.
        id: String,
        /// Where it is attached, and what the provider last reported.
        domain: CustomDomain,
    },
}

/// Meaning of a graph edge.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum MapEdgeKind {
    /// Source has a recorded release for this runtime resource.
    Deploys,
    /// A recorded custom hostname routes to the resource.
    Route,
    /// A recorded environment variable or binding that points at another resource.
    Dependency,
}

/// Where a relationship came from. Says nothing about whether it works now.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum MapEdgeOrigin {
    /// Recorded deployment attempt, including attempts that later failed.
    Deployment,
    /// A saved custom-domain attachment.
    CustomDomain,
    /// Recorded by the person or an agent.
    Manual,
    /// Discovered at the provider; may no longer be there.
    Derived,
}

/// A graph edge, with where it came from and whether it is still seen.
#[derive(Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct MapEdge {
    /// Edge id.
    pub id: String,
    /// Source node identifier.
    pub from: String,
    /// Destination node identifier.
    pub to: String,
    /// What the edge means.
    pub kind: MapEdgeKind,
    /// Variable or binding name, hostname, or deployment label.
    pub via: String,
    /// Where it came from.
    pub origin: MapEdgeOrigin,
    /// The provider no longer shows this relationship; it is kept for review.
    pub missing: bool,
    /// The deployment behind a `deploys` edge; null for other edges.
    pub deploy_id: Option<String>,
}

impl<'a> ProjectService<'a> {
    /// Builds a project's map from recorded state, without calling providers or writing.
    ///
    /// # Errors
    /// Rejects missing projects, invalid relationships, missing permission and catalog limits.
    pub async fn project_map(self, actor: &Actor, id: &str) -> Result<ProjectMap, EngineError> {
        self.engine.authorize(actor, "apps:read").await?;
        let project = self
            .engine
            .repository
            .project(id)
            .await?
            .ok_or(EngineError::NotFound)?;
        let (resources, links) = self.engine.projects().project_map_resources(id).await?;
        let source_id = format!("source:{id}");
        let linked = !matches!(project.source, Source::Unlinked);
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        if linked {
            nodes.push(MapNode::Source {
                id: source_id.clone(),
                source: project.source.clone(),
            });
        }
        for resource in &resources {
            let deploy = match &resource.project_id {
                Some(owner) => {
                    self.engine
                        .repository
                        .latest_resource_deploy(owner, &resource.id)
                        .await?
                }
                None => None,
            };
            let release = deploy.as_ref().and_then(|deploy| {
                deploy
                    .releases
                    .iter()
                    .find(|release| release.resource_id == resource.id)
            });
            let schedules = release.and_then(|release| release.release.schedules.clone());
            let node_id = format!("resource:{}", resource.id);
            // A linked folder is not evidence that it deployed every imported/shared resource.
            if linked
                && resource.project_id.as_deref() == Some(id)
                && release.is_some()
                && deploy
                    .as_ref()
                    .is_some_and(|deploy| deploy.source == project.source)
                && matches!(
                    resource.kind,
                    ResourceKind::Web | ResourceKind::Service | ResourceKind::Job
                )
            {
                edges.push(MapEdge {
                    id: format!("deploys:{}", resource.id),
                    from: source_id.clone(),
                    to: node_id.clone(),
                    kind: MapEdgeKind::Deploys,
                    via: "deploys".into(),
                    origin: MapEdgeOrigin::Deployment,
                    missing: false,
                    deploy_id: deploy.as_ref().map(|deploy| deploy.id.clone()),
                });
            }
            nodes.push(MapNode::Resource {
                id: node_id,
                resource: resource.clone(),
                latest_usage: self
                    .engine
                    .repository
                    .latest_resource_usage(&resource.id)
                    .await?,
                schedules,
            });
        }
        let visible: std::collections::BTreeSet<_> = resources
            .iter()
            .map(|resource| resource.id.as_str())
            .collect();
        for domain in self
            .engine
            .environments()
            .all_domains()
            .await?
            .into_iter()
            .filter(|domain| visible.contains(domain.resource_id.as_str()))
        {
            let node_id = format!("domain:{}", domain.id);
            edges.push(MapEdge {
                id: format!("route:{}", domain.id),
                from: node_id.clone(),
                to: format!("resource:{}", domain.resource_id),
                kind: MapEdgeKind::Route,
                via: domain.hostname.clone(),
                origin: MapEdgeOrigin::CustomDomain,
                missing: false,
                deploy_id: None,
            });
            nodes.push(MapNode::Domain {
                id: node_id,
                domain,
            });
        }
        for link in &links {
            edges.push(MapEdge {
                id: format!("link:{}", link.id),
                from: format!("resource:{}", link.from_resource),
                to: format!("resource:{}", link.to_resource),
                kind: MapEdgeKind::Dependency,
                via: link.via.clone(),
                origin: if link.derived {
                    MapEdgeOrigin::Derived
                } else {
                    MapEdgeOrigin::Manual
                },
                missing: link.missing,
                deploy_id: None,
            });
        }
        self.engine.authorize(actor, "apps:read").await?;
        Ok(ProjectMap {
            layout: self.engine.repository.map_layout(id).await?,
            source: project.source,
            nodes: resources,
            edges: links,
            graph: MapGraph {
                nodes,
                edges,
                relationships_complete: false,
            },
        })
    }
}

impl<'a> ProjectService<'a> {
    /// Saves map positions only. Watched projects can be laid out too: nothing changes at the
    /// provider.
    ///
    /// # Errors
    /// Rejects invalid coordinates, unknown nodes, stale revisions or missing authorization.
    pub async fn save_map_layout(
        self,
        actor: &Actor,
        project_id: &str,
        layout: crate::domain::map_layout::MapLayout,
    ) -> Result<crate::domain::map_layout::MapLayout, EngineError> {
        let _mutation = self.engine.mutation.lock().await;
        self.engine.authorize(actor, "apps:write").await?;
        if !layout.valid() {
            return Err(EngineError::Invalid(
                "Use unique map node IDs with bounded finite coordinates.",
            ));
        }
        let map = self
            .engine
            .projects()
            .project_map(actor, project_id)
            .await?;
        let ids: std::collections::HashSet<_> = map
            .graph
            .nodes
            .iter()
            .map(|node| match node {
                MapNode::Source { id, .. }
                | MapNode::Resource { id, .. }
                | MapNode::Domain { id, .. } => id.as_str(),
            })
            .collect();
        if layout
            .positions
            .iter()
            .any(|p| !ids.contains(p.node_id.as_str()))
        {
            return Err(EngineError::Conflict(
                "A map node is no longer present. Refresh the map before saving positions.",
            ));
        }
        self.engine
            .repository
            .put_map_layout(actor.id(), project_id, layout)
            .await
    }
}
