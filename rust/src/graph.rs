use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::PathBuf;

use pyo3::prelude::*;

use anyhow::Result;
use bincode;
use pyo3::types::PyType;
use rstar::{PointDistance, RTreeObject, AABB};
use serde::{Deserialize, Serialize};

#[pyclass]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Node {
    #[pyo3(get)]
    pub id: u32,
    #[pyo3(get)]
    pub x: isize,
    #[pyo3(get)]
    pub y: isize,
}

impl RTreeObject for Node {
    type Envelope = AABB<[isize; 2]>;

    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners([self.x, self.y], [self.x, self.y])
    }
}

impl PointDistance for Node {
    fn distance_2(&self, point: &[isize; 2]) -> isize {
        let dx = self.x - point[0];
        let dy = self.y - point[1];
        dx * dx + dy * dy
    }
}

#[pymethods]
impl Node {
    #[new]
    pub fn new(id: u32, x: isize, y: isize) -> Self {
        Node { id, x, y }
    }
}

#[pyclass]
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Serialize, Deserialize)]
pub struct Link {
    #[pyo3(get)]
    pub start_node: Node,
    #[pyo3(get)]
    pub end_node: Node,
    #[pyo3(get)]
    pub road_class: u8,
    #[pyo3(get)]
    pub time_seconds: u32,
    #[pyo3(get)]
    pub distance_centimeters: u32,
    #[pyo3(get)]
    pub grade: i16,
    #[pyo3(get)]
    pub weight_limit_lbs: Option<u32>,
    #[pyo3(get)]
    pub height_limit_inches: Option<u16>,
    #[pyo3(get)]
    pub width_limit_inches: Option<u16>,
    #[pyo3(get)]
    pub length_limit_inches: Option<u16>,
}

#[pymethods]
impl Link {
    #[new]
    pub fn new(
        start_node: Node,
        end_node: Node,
        road_class: u8,
        time_seconds: u32,
        distance_centimeters: u32,
        grade: i16,
        weight_limit_lbs: Option<u32>,
        height_limit_inches: Option<u16>,
        width_limit_inches: Option<u16>,
        length_limit_inches: Option<u16>,
    ) -> Self {
        Link {
            start_node,
            end_node,
            road_class,
            time_seconds,
            distance_centimeters,
            grade,
            weight_limit_lbs,
            height_limit_inches,
            width_limit_inches,
            length_limit_inches,
        }
    }
    pub fn transpose(&self) -> Self {
        Link {
            start_node: self.end_node,
            end_node: self.start_node,
            road_class: self.road_class,
            time_seconds: self.time_seconds,
            distance_centimeters: self.distance_centimeters,
            grade: self.grade,
            weight_limit_lbs: self.weight_limit_lbs,
            height_limit_inches: self.height_limit_inches,
            width_limit_inches: self.width_limit_inches,
            length_limit_inches: self.length_limit_inches,
        }
    }
}

#[pyclass]
#[derive(Serialize, Deserialize, Clone)]
pub struct Graph {
    #[pyo3(get)]
    pub adjacency_list: HashMap<Node, HashSet<Link>>,
}

impl Graph {
    pub fn neighbors(&self, node: &Node) -> Option<&HashSet<Link>> {
        self.adjacency_list.get(node)
    }
    pub fn to_binary(&self) -> Vec<u8> {
        bincode::serialize(&self).unwrap()
    }
    pub fn from_binary(binary: &[u8]) -> Self {
        bincode::deserialize(binary).unwrap()
    }

    pub fn to_file(&self, filename: &str) -> Result<()> {
        let path = PathBuf::from(filename);
        let mut file = std::fs::File::create(path)?;
        bincode::serialize_into(&mut file, &self)?;
        Ok(())
    }

    pub fn from_file(filename: &str) -> Result<Self> {
        let path = PathBuf::from(filename);
        let file = std::fs::File::open(path)?;
        let graph = bincode::deserialize_from(file)?;
        Ok(graph)
    }
}

#[pymethods]
impl Graph {
    #[new]
    pub fn new() -> Self {
        Graph {
            adjacency_list: HashMap::new(),
        }
    }

    pub fn get_transpose(&self) -> Graph {
        let mut transpose = Graph::new();
        for links in self.adjacency_list.values() {
            for link in links {
                transpose.add_link(link.transpose());
            }
        }
        transpose
    }

    pub fn get_nodes(&self) -> Vec<Node> {
        self.adjacency_list.keys().cloned().collect()
    }

    /// get a list of links given a list of nodes
    /// this is useful for getting the links that are in a given route
    /// the links are returned in the order that they appear in the route
    pub fn get_links_in_path(&self, nodes_in_path: Vec<Node>) -> Vec<Link> {
        let mut links_in_path = Vec::new();
        for (start_node, end_node) in nodes_in_path.windows(2).map(|w| (w[0], w[1])) {
            let links = self
                .adjacency_list
                .get(&start_node)
                .expect("start node not found in graph");
            let link = links
                .iter()
                .find(|link| link.end_node == end_node)
                .expect("end node not found in graph");
            links_in_path.push(link.clone());
        }
        links_in_path
    }

    #[pyo3(name = "to_file")]
    pub fn py_to_file(&self, filename: &str) -> Result<()> {
        self.to_file(filename)
    }

    #[classmethod]
    #[pyo3(name = "from_file")]
    pub fn py_from_file(_: &PyType, filename: &str) -> Result<Self> {
        let path = PathBuf::from(filename);
        let file = std::fs::File::open(path)?;
        let graph = bincode::deserialize_from(file)?;
        Ok(graph)
    }

    pub fn add_link(&mut self, link: Link) {
        self.adjacency_list
            .entry(link.start_node)
            .or_insert_with(HashSet::new)
            .insert(link);
    }

    pub fn add_links_bulk(&mut self, links: Vec<Link>) {
        for link in links {
            self.add_link(link);
        }
    }
}
