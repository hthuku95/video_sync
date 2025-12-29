// StateGraph - Node and edge management (LangGraph-inspired)
use super::state::{WorkflowState, StateUpdate};
use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Node function type - async function that processes state
#[async_trait]
pub trait NodeFunction: Send + Sync {
    async fn execute(&self, state: &WorkflowState) -> Result<StateUpdate, String>;
}

/// Conditional routing function - decides next node
pub type RouterFunction = Arc<dyn Fn(&WorkflowState) -> Option<String> + Send + Sync>;

/// Node types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeType {
    /// Start node (entry point)
    Start,
    /// Agent node (LLM reasoning + tool calling)
    Agent,
    /// Tool node (executes specific tool)
    Tool,
    /// Decision node (conditional routing)
    Decision,
    /// Human node (requires human input)
    Human,
    /// Parallel node (spawns concurrent executions)
    Parallel,
    /// End node (terminal state)
    End,
}

/// Edge types
#[derive(Clone)]
pub enum EdgeType {
    /// Fixed edge (always follows this path)
    Fixed(String),  // target node
    /// Conditional edge (router function decides)
    Conditional(RouterFunction),
    /// Parallel edges (fork to multiple nodes)
    Parallel(Vec<String>),
}

impl std::fmt::Debug for EdgeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EdgeType::Fixed(target) => write!(f, "Fixed({})", target),
            EdgeType::Conditional(_) => write!(f, "Conditional(<router>)"),
            EdgeType::Parallel(targets) => write!(f, "Parallel({:?})", targets),
        }
    }
}

/// Graph node
pub struct Node {
    pub id: String,
    pub node_type: NodeType,
    pub function: Arc<dyn NodeFunction>,
    pub description: String,
    pub max_retries: usize,
    pub timeout_seconds: u64,
}

/// StateGraph - The workflow graph
pub struct StateGraph {
    /// All nodes in the graph
    nodes: HashMap<String, Node>,

    /// Edges: node_id -> EdgeType
    edges: HashMap<String, EdgeType>,

    /// Entry point node
    entry_point: Option<String>,

    /// Compiled flag
    compiled: bool,
}

impl StateGraph {
    /// Create new state graph
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            edges: HashMap::new(),
            entry_point: None,
            compiled: false,
        }
    }

    /// Add node to graph
    pub fn add_node(
        &mut self,
        id: String,
        node_type: NodeType,
        function: Arc<dyn NodeFunction>,
        description: String,
    ) -> &mut Self {
        if self.compiled {
            panic!("Cannot modify compiled graph");
        }

        let node = Node {
            id: id.clone(),
            node_type,
            function,
            description,
            max_retries: 3,
            timeout_seconds: 300,
        };

        self.nodes.insert(id, node);
        self
    }

    /// Add fixed edge
    pub fn add_edge(&mut self, from: String, to: String) -> &mut Self {
        if self.compiled {
            panic!("Cannot modify compiled graph");
        }
        self.edges.insert(from, EdgeType::Fixed(to));
        self
    }

    /// Add conditional edge with router
    pub fn add_conditional_edge(
        &mut self,
        from: String,
        router: RouterFunction,
    ) -> &mut Self {
        if self.compiled {
            panic!("Cannot modify compiled graph");
        }
        self.edges.insert(from, EdgeType::Conditional(router));
        self
    }

    /// Add parallel edges (fork execution)
    pub fn add_parallel_edges(&mut self, from: String, targets: Vec<String>) -> &mut Self {
        if self.compiled {
            panic!("Cannot modify compiled graph");
        }
        self.edges.insert(from, EdgeType::Parallel(targets));
        self
    }

    /// Set entry point
    pub fn set_entry_point(&mut self, node_id: String) -> &mut Self {
        if self.compiled {
            panic!("Cannot modify compiled graph");
        }
        self.entry_point = Some(node_id);
        self
    }

    /// Compile graph (validate and optimize)
    pub fn compile(&mut self) -> Result<(), String> {
        // Validate entry point exists
        if self.entry_point.is_none() {
            return Err("No entry point set".to_string());
        }

        let entry = self.entry_point.as_ref().unwrap();
        if !self.nodes.contains_key(entry) {
            return Err(format!("Entry point node '{}' does not exist", entry));
        }

        // Validate all edges point to existing nodes
        for (from, edge) in &self.edges {
            if !self.nodes.contains_key(from) {
                return Err(format!("Edge from non-existent node: {}", from));
            }

            match edge {
                EdgeType::Fixed(to) => {
                    if !self.nodes.contains_key(to) {
                        return Err(format!("Edge to non-existent node: {}", to));
                    }
                }
                EdgeType::Parallel(targets) => {
                    for target in targets {
                        if !self.nodes.contains_key(target) {
                            return Err(format!("Parallel edge to non-existent node: {}", target));
                        }
                    }
                }
                EdgeType::Conditional(_) => {
                    // Router validated at runtime
                }
            }
        }

        // Detect cycles (simple DFS check)
        if self.has_cycles() {
            tracing::warn!("Graph contains cycles - this is allowed but may loop indefinitely");
        }

        self.compiled = true;
        tracing::info!("✅ StateGraph compiled successfully: {} nodes, {} edges",
            self.nodes.len(), self.edges.len());

        Ok(())
    }

    /// Check for cycles using DFS
    fn has_cycles(&self) -> bool {
        let mut visited = HashMap::new();
        let mut rec_stack = HashMap::new();

        for node_id in self.nodes.keys() {
            if self.dfs_cycle_check(node_id, &mut visited, &mut rec_stack) {
                return true;
            }
        }

        false
    }

    fn dfs_cycle_check(
        &self,
        node: &str,
        visited: &mut HashMap<String, bool>,
        rec_stack: &mut HashMap<String, bool>,
    ) -> bool {
        visited.insert(node.to_string(), true);
        rec_stack.insert(node.to_string(), true);

        if let Some(edge) = self.edges.get(node) {
            let targets = match edge {
                EdgeType::Fixed(target) => vec![target.clone()],
                EdgeType::Parallel(targets) => targets.clone(),
                EdgeType::Conditional(_) => vec![], // Skip conditional
            };

            for target in targets {
                if !visited.get(&target).unwrap_or(&false) {
                    if self.dfs_cycle_check(&target, visited, rec_stack) {
                        return true;
                    }
                } else if *rec_stack.get(&target).unwrap_or(&false) {
                    return true;
                }
            }
        }

        rec_stack.insert(node.to_string(), false);
        false
    }

    /// Get next node(s) based on current state
    pub fn get_next_nodes(&self, current_node: &str, state: &WorkflowState) -> Vec<String> {
        match self.edges.get(current_node) {
            Some(EdgeType::Fixed(target)) => vec![target.clone()],
            Some(EdgeType::Conditional(router)) => {
                if let Some(next) = router(state) {
                    vec![next]
                } else {
                    vec![]
                }
            }
            Some(EdgeType::Parallel(targets)) => targets.clone(),
            None => vec![], // End node
        }
    }

    /// Get next node (singular) based on current state
    pub fn get_next_node(&self, current_node: &str, state: &WorkflowState) -> Option<String> {
        self.get_next_nodes(current_node, state).into_iter().next()
    }

    /// Get node by ID
    pub fn get_node(&self, node_id: &str) -> Option<&Node> {
        self.nodes.get(node_id)
    }

    /// Get entry point
    pub fn get_entry_point(&self) -> Option<&String> {
        self.entry_point.as_ref()
    }

    /// Check if compiled
    pub fn is_compiled(&self) -> bool {
        self.compiled
    }

    /// Get all node IDs
    pub fn get_node_ids(&self) -> Vec<String> {
        self.nodes.keys().cloned().collect()
    }
}

impl Default for StateGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder pattern for StateGraph
pub struct StateGraphBuilder {
    graph: StateGraph,
}

impl StateGraphBuilder {
    pub fn new() -> Self {
        Self {
            graph: StateGraph::new(),
        }
    }

    pub fn add_node(
        mut self,
        id: &str,
        node_type: NodeType,
        function: Arc<dyn NodeFunction>,
        description: &str,
    ) -> Self {
        self.graph.add_node(
            id.to_string(),
            node_type,
            function,
            description.to_string(),
        );
        self
    }

    pub fn add_edge(mut self, from: &str, to: &str) -> Self {
        self.graph.add_edge(from.to_string(), to.to_string());
        self
    }

    pub fn add_conditional_edge(mut self, from: &str, router: RouterFunction) -> Self {
        self.graph.add_conditional_edge(from.to_string(), router);
        self
    }

    pub fn set_entry_point(mut self, node_id: &str) -> Self {
        self.graph.set_entry_point(node_id.to_string());
        self
    }

    pub fn build(mut self) -> Result<StateGraph, String> {
        self.graph.compile()?;
        Ok(self.graph)
    }
}

impl Default for StateGraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}
