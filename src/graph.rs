use crate::parser::Dependency;
use crate::parser::{InfraFile, InfraNode, ModuleNode, ScopeNode};
use anyhow::{Result, anyhow};
use petgraph::Direction;
use petgraph::graph::{DiGraph, NodeIndex};
use serde_yaml::Value;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct Scope {
    pub name: String,
    pub variables: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct ModuleGraph {
    mod_dependency_graph: DiGraph<String, ()>,
    modules: HashMap<String, ModuleNode>,
    scopes: HashMap<String, Scope>,
}

impl ModuleGraph {
    pub fn new(infra: &InfraFile) -> Result<Self> {
        let mut mod_dependency_graph: DiGraph<String, ()> = DiGraph::new();
        let mut modules: HashMap<String, ModuleNode> = HashMap::new();
        let mut scopes: HashMap<String, ScopeNode> = HashMap::new();

        // Collect all modules and scopes
        for node in infra.nodes.values() {
            collect_modules(node, &mut modules, &mut scopes);
        }

        // Add nodes
        let mut node_indices = HashMap::new();
        for id in modules.keys() {
            let idx = mod_dependency_graph.add_node(id.clone());
            node_indices.insert(id.clone(), idx);
        }

        let mut final_modules = HashMap::new();
        for (id, module) in &modules {
            let mut enriched_deps = Vec::new();
            for dependency in &module.dependencies {
                let dep_id = resolve_dependency_id(module, &dependency.name, &modules)?;
                enriched_deps.push(Dependency {
                    id: dep_id.clone(),
                    name: dependency.name.clone(),
                });
                if let (Some(&from), Some(&to)) = (node_indices.get(&dep_id), node_indices.get(id))
                {
                    mod_dependency_graph.add_edge(from, to, ());
                }
            }
            let mut module = module.clone();
            module.dependencies = enriched_deps;
            final_modules.insert(id.clone(), module);
        }

        Ok(Self {
            mod_dependency_graph,
            modules: final_modules,
            scopes: scopes
                .into_iter()
                .map(|(id, s)| {
                    (
                        id,
                        Scope {
                            name: s.scope,
                            variables: s.variables,
                        },
                    )
                })
                .collect(),
        })
    }

    pub fn execution_layers(&self, target_module_id: &str) -> Result<(Vec<Vec<String>>, String)> {
        dbg!(&self.mod_dependency_graph);
        // Find NodeIndex for target module
        let target_idx = self
            .mod_dependency_graph
            .node_indices()
            .find(|&i| self.mod_dependency_graph[i] == target_module_id)
            .ok_or_else(|| anyhow!("Target module not found: {}", target_module_id))?;

        // Collect all dependencies (ancestors) of the target module
        let mut relevant = HashSet::new();
        let mut stack = vec![target_idx];
        while let Some(idx) = stack.pop() {
            if relevant.insert(idx) {
                for dep in self
                    .mod_dependency_graph
                    .neighbors_directed(idx, Direction::Incoming)
                {
                    stack.push(dep);
                }
            }
        }

        // Topologically sort the relevant subgraph
        let mut sorted: Vec<NodeIndex> = Vec::new();
        let mut temp_mark = HashSet::new();
        let mut perm_mark = HashSet::new();

        fn visit(
            idx: NodeIndex,
            graph: &DiGraph<String, ()>,
            relevant: &HashSet<NodeIndex>,
            temp_mark: &mut HashSet<NodeIndex>,
            perm_mark: &mut HashSet<NodeIndex>,
            sorted: &mut Vec<NodeIndex>,
        ) -> Result<()> {
            if perm_mark.contains(&idx) {
                return Ok(());
            }
            if temp_mark.contains(&idx) {
                return Err(anyhow!("Cycle detected in dependency graph"));
            }
            temp_mark.insert(idx);
            for dep in graph.neighbors_directed(idx, Direction::Incoming) {
                if relevant.contains(&dep) {
                    visit(dep, graph, relevant, temp_mark, perm_mark, sorted)?;
                }
            }
            temp_mark.remove(&idx);
            perm_mark.insert(idx);
            sorted.push(idx);
            Ok(())
        }

        visit(
            target_idx,
            &self.mod_dependency_graph,
            &relevant,
            &mut temp_mark,
            &mut perm_mark,
            &mut sorted,
        )?;

        // Build layers (excluding the target from layers, return it separately)
        let mut layers: Vec<Vec<String>> = Vec::new();
        let mut assigned: HashSet<NodeIndex> = HashSet::new();
        let mut remaining: HashSet<NodeIndex> = sorted.iter().cloned().collect();
        remaining.remove(&target_idx); // Exclude target from layers

        while !remaining.is_empty() {
            let mut layer = Vec::new();
            let mut next_remaining = HashSet::new();

            for &idx in &remaining {
                let all_deps_assigned = self
                    .mod_dependency_graph
                    .neighbors_directed(idx, Direction::Incoming)
                    .filter(|dep_idx| relevant.contains(dep_idx))
                    .all(|dep_idx| assigned.contains(&dep_idx));

                if all_deps_assigned {
                    layer.push(self.mod_dependency_graph[idx].clone());
                } else {
                    next_remaining.insert(idx);
                }
            }

            if layer.is_empty() {
                return Err(anyhow!(
                    "Cannot resolve layer dependencies: possible cycle or missing outputs"
                ));
            }

            for id in &layer {
                let idx = self
                    .mod_dependency_graph
                    .node_indices()
                    .find(|&i| &self.mod_dependency_graph[i] == id)
                    .unwrap();
                assigned.insert(idx);
            }

            layers.push(layer);
            remaining = next_remaining;
        }

        Ok((layers, target_module_id.to_string()))
    }

    pub fn modules(self) -> HashMap<String, ModuleNode> {
        self.modules
    }
    pub fn scopes(self) -> HashMap<String, Scope> {
        self.scopes
    }
    pub fn get_module_by_id(&self, id: &str) -> Option<ModuleNode> {
        self.modules.get(id).cloned()
    }
    pub fn get_scope_by_id(&self, id: &str) -> Option<Scope> {
        self.scopes.get(id).cloned()
    }
}
fn resolve_dependency_id(
    module: &ModuleNode,
    dep_name: &str,
    modules: &HashMap<String, ModuleNode>,
) -> Result<String> {
    // Search in current scope_ids from most specific to least
    let mut scope_ids: Vec<_> = module.scope_ids.iter().collect();
    scope_ids.reverse();
    for scope_id in scope_ids {
        // Find a module in this scope with matching source
        for m in modules.values() {
            if m.source == dep_name && m.scope_ids.contains(scope_id) {
                return Ok(m.id.clone());
            }
        }
    }
    Err(anyhow!(
        "dependency '{}' of module '{}' not found in the infrastructure",
        dep_name,
        module.id
    ))
}
fn collect_modules(
    node: &InfraNode,
    modules: &mut HashMap<String, ModuleNode>,
    scopes: &mut HashMap<String, ScopeNode>,
) {
    match node {
        InfraNode::Module(m) => {
            modules.insert(m.id.clone(), m.clone());
        }
        InfraNode::Scope(s) => {
            scopes.insert(s.id.clone(), s.clone());
            for child in s.children.values() {
                collect_modules(child, modules, scopes);
            }
        }
    }
}
