use cronos_model::*;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use std::collections::HashMap;

pub struct ContextGraph {
    graph: DiGraph<EntityId, EdgeInfo>,
    entity_index: HashMap<EntityId, NodeIndex>,
}

#[derive(Debug, Clone)]
pub struct EdgeInfo {
    pub edge_id: EdgeId,
    pub relation: Relation,
    pub strength: f32,
}

impl ContextGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            entity_index: HashMap::new(),
        }
    }

    /// Rebuild graph from storage data
    pub fn rebuild(entities: &[Entity], edges: &[Edge]) -> Self {
        let mut g = Self::new();
        for entity in entities {
            g.add_entity(entity.id);
        }
        for edge in edges {
            g.add_edge(edge);
        }
        g
    }

    pub fn add_entity(&mut self, id: EntityId) -> NodeIndex {
        if let Some(&idx) = self.entity_index.get(&id) {
            return idx;
        }
        let idx = self.graph.add_node(id);
        self.entity_index.insert(id, idx);
        idx
    }

    pub fn add_edge(&mut self, edge: &Edge) {
        let from_idx = self.add_entity(edge.from);
        let to_idx = self.add_entity(edge.to);
        // Check if edge already exists and update
        let existing = self
            .graph
            .edges_connecting(from_idx, to_idx)
            .find(|e| e.weight().edge_id == edge.id);
        if let Some(e) = existing {
            let idx = e.id();
            if let Some(w) = self.graph.edge_weight_mut(idx) {
                w.strength = edge.strength;
            }
        } else {
            self.graph.add_edge(
                from_idx,
                to_idx,
                EdgeInfo {
                    edge_id: edge.id,
                    relation: edge.relation.clone(),
                    strength: edge.strength,
                },
            );
        }
    }

    /// Get all entities connected to the given entity within `depth` hops
    pub fn related(&self, entity_id: &EntityId, depth: u8) -> Vec<EntityId> {
        let Some(&start) = self.entity_index.get(entity_id) else {
            return vec![];
        };

        let mut visited = HashMap::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back((start, 0u8));
        visited.insert(start, 0u8);

        while let Some((node, d)) = queue.pop_front() {
            if d >= depth {
                continue;
            }
            for neighbor in self
                .graph
                .neighbors_directed(node, petgraph::Direction::Outgoing)
            {
                if let std::collections::hash_map::Entry::Vacant(e) = visited.entry(neighbor) {
                    e.insert(d + 1);
                    queue.push_back((neighbor, d + 1));
                }
            }
            for neighbor in self
                .graph
                .neighbors_directed(node, petgraph::Direction::Incoming)
            {
                if let std::collections::hash_map::Entry::Vacant(e) = visited.entry(neighbor) {
                    e.insert(d + 1);
                    queue.push_back((neighbor, d + 1));
                }
            }
        }

        visited
            .into_iter()
            .filter(|(idx, _)| *idx != start)
            .map(|(idx, _)| self.graph[idx])
            .collect()
    }

    pub fn entity_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    pub fn has_entity(&self, id: &EntityId) -> bool {
        self.entity_index.contains_key(id)
    }
}

impl Default for ContextGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_query_entities() {
        let mut g = ContextGraph::new();
        let id1 = EntityId::new();
        let id2 = EntityId::new();
        g.add_entity(id1);
        g.add_entity(id2);
        assert_eq!(g.entity_count(), 2);
        assert!(g.has_entity(&id1));
    }

    #[test]
    fn add_entity_is_idempotent() {
        let mut g = ContextGraph::new();
        let id = EntityId::new();
        g.add_entity(id);
        g.add_entity(id);
        assert_eq!(g.entity_count(), 1);
    }

    #[test]
    fn related_traversal() {
        let mut g = ContextGraph::new();
        let file = EntityId::new();
        let project = EntityId::new();
        let repo = EntityId::new();

        let edge1 = Edge {
            id: EdgeId::new(),
            from: file,
            to: project,
            relation: Relation::BelongsTo,
            strength: 0.8,
            created_at: 1000,
            last_reinforced: 1000,
        };
        let edge2 = Edge {
            id: EdgeId::new(),
            from: project,
            to: repo,
            relation: Relation::Contains,
            strength: 0.9,
            created_at: 1000,
            last_reinforced: 1000,
        };
        g.add_edge(&edge1);
        g.add_edge(&edge2);

        let related = g.related(&file, 1);
        assert_eq!(related.len(), 1);
        assert!(related.contains(&project));

        let related = g.related(&file, 2);
        assert_eq!(related.len(), 2);
        assert!(related.contains(&project));
        assert!(related.contains(&repo));
    }

    #[test]
    fn rebuild_from_data() {
        let e1 = Entity {
            id: EntityId::new(),
            kind: EntityKind::File,
            name: "a".into(),
            attributes: Default::default(),
            first_seen: 0,
            last_seen: 0,
        };
        let e2 = Entity {
            id: EntityId::new(),
            kind: EntityKind::Project,
            name: "b".into(),
            attributes: Default::default(),
            first_seen: 0,
            last_seen: 0,
        };
        let edge = Edge {
            id: EdgeId::new(),
            from: e1.id,
            to: e2.id,
            relation: Relation::BelongsTo,
            strength: 0.5,
            created_at: 0,
            last_reinforced: 0,
        };
        let g = ContextGraph::rebuild(&[e1, e2], &[edge]);
        assert_eq!(g.entity_count(), 2);
        assert_eq!(g.edge_count(), 1);
    }
}
