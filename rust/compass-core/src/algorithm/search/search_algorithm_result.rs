use chrono::{DateTime, Local};

use crate::{
    algorithm::search::{edge_traversal::EdgeTraversal, search_tree_branch::SearchTreeBranch},
    model::graph::vertex_id::VertexId,
};
use std::{collections::HashMap, time::Duration};

pub struct SearchAlgorithmResult {
    pub route: Vec<EdgeTraversal>,
    pub tree: HashMap<VertexId, SearchTreeBranch>,
    pub search_start_time: DateTime<Local>,
    pub search_runtime: Duration,
    pub route_runtime: Duration,
    pub total_runtime: Duration,
}
