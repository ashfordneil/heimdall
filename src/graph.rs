use std::{cmp, iter};

use either::Either;
use slab::Slab;

#[derive(Debug)]
pub struct Edge<'a, W> {
    pub weight: &'a W,
    pub connects_to: usize,
    // ensure that nobody creates arbitrary edges
    _seal: (),
}

#[derive(Debug)]
struct InnerEdge {
    weight: usize,
    connects_to: usize,
}

#[derive(Debug)]
struct Node {
    incoming: Vec<InnerEdge>,
    outgoing: Vec<InnerEdge>,
}

impl Node {
    /// Create a new node.
    pub fn new() -> Self {
        let incoming = Vec::new();
        let outgoing = Vec::new();

        Node { incoming, outgoing }
    }
}

/// A directed graph with weighted edges, backed by two-way adjacency lists. Nodes are usizes, and
/// store no data. As such, there is no method to create a node. Instead, nodes will be initialised
/// (with no edges) where necessary.
#[derive(Debug)]
pub struct Graph<W> {
    weights: Slab<W>,
    nodes: Vec<Node>,
}

impl<W> Graph<W> {
    /// Create a new, empty graph.
    pub fn new() -> Self {
        let weights = Slab::new();
        let nodes = Vec::new();

        Graph { weights, nodes }
    }

    /// Ensure there is space in the graph for an nth node, by adding new empty nodes up to n if
    /// they do not exist.
    fn extend_to(&mut self, n: usize) {
        self.nodes
            .extend(iter::repeat_with(Node::new).take((1 + n).saturating_sub(self.nodes.len())));
    }

    /// Create an edge. Takes the indices of the nodes to connect, as well as the weight to connect
    /// them with.
    pub fn add_edge(&mut self, from: usize, to: usize, weight: W) {
        self.extend_to(cmp::max(from, to));
        let weight = self.weights.insert(weight);

        self.nodes[from].outgoing.push(InnerEdge {
            weight,
            connects_to: to,
        });
        self.nodes[to].incoming.push(InnerEdge {
            weight,
            connects_to: from,
        });
    }

    /// Iterate over the edges leaving a node.
    pub fn outgoing(&self, node: usize) -> impl Iterator<Item = Edge<W>> {
        match self.nodes.get(node) {
            Some(node) => Either::Left(node.outgoing.iter().map(move |edge| Edge {
                weight: &self.weights[edge.weight],
                connects_to: edge.connects_to,
                _seal: (),
            })),
            None => Either::Right(iter::empty()),
        }
    }

    /// Iterate over the edges entering a node.
    pub fn incoming(&self, node: usize) -> impl Iterator<Item = Edge<W>> {
        match self.nodes.get(node) {
            Some(node) => Either::Left(node.incoming.iter().map(move |edge| Edge {
                weight: &self.weights[edge.weight],
                connects_to: edge.connects_to,
                _seal: (),
            })),
            None => Either::Right(iter::empty()),
        }
    }
}
