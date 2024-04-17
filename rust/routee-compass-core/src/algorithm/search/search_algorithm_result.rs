use super::{edge_traversal::EdgeTraversal, search_tree_branch::SearchTreeBranch};
use crate::model::road_network::vertex_id::VertexId;
use std::collections::HashMap;

pub struct SearchAlgorithmResult {
    pub trees: Vec<HashMap<VertexId, SearchTreeBranch>>,
    pub routes: Vec<Vec<EdgeTraversal>>,
    pub iterations: u64,
}
