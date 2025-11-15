use crate::graph::ModuleGraph;
use crate::parser::{InfraFile, InputValue, ModuleNode};
use crate::terraform::{RunTerraformCommand, TerraformAction};
use anyhow::{Context, Result, anyhow};
use futures::future::join_all;
use serde_yaml::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
enum PathSegment {
    Key(String),
    Index(usize),
}
#[derive(Debug)]
pub struct Runtime {
    pub runner: Arc<dyn RunTerraformCommand + Send + Sync>,
    pub graph: ModuleGraph,
}

impl Runtime {
    pub fn new(
        runner: Arc<dyn RunTerraformCommand + Send + Sync>,
        infra: &InfraFile,
    ) -> Result<Self> {
        let graph = ModuleGraph::new(infra).context("While building dependency graph")?;
        Ok(Self { runner, graph })
    }

    /// Execute a target module and all its dependencies in correct graph order
    pub async fn run_module(&self, module_id: &str, action: TerraformAction) -> Result<()> {
        let (layers, target) = self.graph.execution_layers(module_id)?;
        let mut outputs_map: HashMap<String, HashMap<String, Value>> = HashMap::new();

        for layer in layers {
            // Run all modules in this layer in parallel
            let futures = layer.into_iter().map(|id| {
                let runner = Arc::clone(&self.runner);
                let graph = self.graph.clone();
                let mut module = graph.get_module_by_id(&id).unwrap();
                let outputs_map = outputs_map.clone();
                async move {
                    inject_inputs(&mut module, &outputs_map, &graph)?;
                    runner.init(&module).await?;
                    let outputs = runner.output(&module).await?;
                    Ok::<(String, HashMap<String, Value>), anyhow::Error>((id, outputs))
                }
            });

            let results = join_all(futures).await;
            for res in results {
                let (id, outputs) = res?;
                outputs_map.insert(id, outputs);
            }
        }

        // Finally, run the target module
        let mut target_module = self
            .graph
            .get_module_by_id(&module_id)
            .ok_or_else(|| anyhow!("Target module not found: {}", target))?;
        inject_inputs(&mut target_module, &outputs_map, &self.graph)?;
        self.runner.init(&target_module).await?;
        let _outputs = self.runner.output(&target_module).await?;
        self.runner.apply(&target_module).await?;
        Ok(())
    }
}

/// Inject resolved inputs into a Terraform module before execution
fn inject_inputs(
    module: &mut ModuleNode,
    outputs_map: &HashMap<String, HashMap<String, Value>>,
    graph: &ModuleGraph,
) -> Result<()> {
    for (key, val) in &module.inputs {
        let resolved = match val {
            InputValue::Default(v) => v.clone(),
            InputValue::Ref { path } => resolve_ref(path, module, outputs_map, graph)?
                .ok_or_else(|| anyhow!("Reference '{}' not found", path))?,
            InputValue::RefWithDefault { path, default } => {
                resolve_ref(path, module, outputs_map, graph)?.unwrap_or(default.clone())
            }
        };
        module.variables.insert(key.clone(), resolved);
    }
    Ok(())
}

/// Resolve a Terraform-style reference like "vpc.subnets[0]" or "region.id"
fn resolve_ref(
    path: &str,
    module: &ModuleNode,
    outputs_map: &HashMap<String, HashMap<String, Value>>,
    graph: &ModuleGraph,
) -> Result<Option<Value>> {
    let mut parts = path.splitn(2, '.');
    let first = parts.next().unwrap();
    let rest = parts.next().unwrap_or("");
    // 1️⃣ Dependency reference (vpc.subnets[0])
    if let Some(dep) = module.dependencies.iter().find(|dep| dep.name == first) {
        let dep_outputs = outputs_map
            .get(&dep.id)
            .ok_or_else(|| anyhow!("Outputs missing for '{}'", dep.id))?;

        let yaml = Value::Mapping(
            dep_outputs
                .iter()
                .map(|(k, v)| (Value::String(k.clone()), v.clone()))
                .collect(),
        );

        let segments = parse_path(rest);
        let resolved = get_value_from_path(&yaml, &segments)
            .ok_or_else(|| anyhow!("Cannot resolve '{}'", path))?;
        return Ok(Some(resolved));
    }

    // 2️⃣ Scope variable (from ancestor scopes)
    if let Some(val) = find_scope_variable(module, first, rest, graph) {
        return Ok(Some(val));
    }

    Ok(None)
}

/// Lookup a scope variable by traversing parent scopes upward
fn find_scope_variable(
    module: &ModuleNode,
    scope_type: &str,
    rest: &str,
    graph: &ModuleGraph,
) -> Option<Value> {
    if let Some(scope) = module.scope_ids.iter().find_map(|id| {
        graph.get_scope_by_id(id).and_then(|scope| {
            if scope.name == scope_type {
                Some(scope)
            } else {
                None
            }
        })
    }) {
        let yaml = Value::Mapping(
            scope
                .variables
                .iter()
                .map(|(k, v)| (Value::String(k.clone()), v.clone()))
                .collect(),
        );
        let segments = parse_path(rest);
        return get_value_from_path(&yaml, &segments);
    }
    None
}

/// Parses a path like "subnets[0].id" into Key/Index segments
fn parse_path(path: &str) -> Vec<PathSegment> {
    let mut segs = Vec::new();
    for part in path.split('.') {
        let mut rem = part;
        while let Some(i) = rem.find('[') {
            if i > 0 {
                segs.push(PathSegment::Key(rem[..i].to_string()));
            }
            let j = rem[i + 1..].find(']').expect("Unmatched bracket") + i + 1;
            let idx = rem[i + 1..j].parse::<usize>().expect("Invalid index");
            segs.push(PathSegment::Index(idx));
            rem = &rem[j + 1..];
        }
        if !rem.is_empty() {
            segs.push(PathSegment::Key(rem.to_string()));
        }
    }

    segs
}

fn get_value_from_path(root: &Value, path: &[PathSegment]) -> Option<Value> {
    let mut cur = root;
    for seg in path {
        match seg {
            PathSegment::Key(k) => {
                if let Value::Mapping(m) = cur {
                    cur = m.get(Value::String(k.clone()))?;
                } else {
                    return None;
                }
            }
            PathSegment::Index(i) => match cur {
                Value::Sequence(seq) => cur = seq.get(*i)?,
                _ => return None,
            },
        }

        // Final recursive unwrap for "value" keys after all segments
        loop {
            if let Value::Mapping(m) = cur
                && let Some(v) = m.get(Value::String("value".into()))
            {
                cur = v;
                continue;
            }
            break;
        }
    }
    Some(cur.clone())
}
