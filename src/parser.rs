use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer};
use serde_yaml::Value;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::Path;
use thiserror::Error;

/// Root structure for the whole infrastructure file.
#[derive(Debug, Clone)]
pub struct InfraFile {
    pub nodes: HashMap<String, InfraNode>,
    pub source_defaults: HashMap<String, ModuleDefaults>,
}

#[derive(Debug, Clone, Deserialize)]
pub enum InfraNode {
    Scope(ScopeNode),
    Module(ModuleNode),
}

/// Input value enum
#[derive(Debug, Clone)]
pub enum InputValue {
    /// Reference to module output or scope variable
    Ref { path: String }, // "vpc.main_lb" or "tenant.id"
    /// Reference with fallback default
    RefWithDefault {
        path: String,
        default: serde_yaml::Value,
    },
    /// Literal value
    Default(serde_yaml::Value),
}
/// Represents module definitions (concrete Terraform stacks).
#[derive(Debug, Clone, Deserialize)]
pub struct ModuleNode {
    pub source: String,
    #[serde(default)]
    pub id: String,
    #[serde(default, deserialize_with = "deserialize_dependencies")]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub variables: HashMap<String, Value>,
    #[serde(default)]
    pub mocked_outputs: Option<HashMap<String, Value>>,
    #[serde(default)]
    pub inputs: HashMap<String, InputValue>,
    #[serde(default)]
    pub scope_ids: HashSet<String>,
}

fn deserialize_dependencies<'de, D>(deserializer: D) -> Result<Vec<Dependency>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = <Vec<String>>::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .map(|s| Dependency {
            id: "".to_string(),
            name: s,
        })
        .collect())
}

/// Defines a nested scope (e.g., account, tenant)
#[derive(Debug, Clone, Deserialize)]
pub struct ScopeNode {
    pub scope: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub variables: HashMap<String, Value>,
    #[serde(default)]
    pub children: HashMap<String, InfraNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Dependency {
    pub id: String,
    pub name: String,
}
/// Defines reusable default settings for a given source
#[derive(Debug, Clone, Deserialize)]
pub struct ModuleDefaults {
    #[serde(default, deserialize_with = "deserialize_dependencies")]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub variables: HashMap<String, Value>,
    #[serde(default)]
    pub mocked_outputs: Option<HashMap<String, Value>>,
    #[serde(default)]
    pub inputs: HashMap<String, InputValue>,
}

#[derive(Error, Debug)]
pub enum InfraError {
    #[error("Failed to parse YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("Invalid YAML structure: {0}")]
    InvalidStructure(String),

    #[error("Scope '{0}' contains a 'source' key — scopes cannot define sources.")]
    InvalidScopeSource(String),
}

impl<'de> Deserialize<'de> for InputValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let val = serde_yaml::Value::deserialize(deserializer)?;

        match val {
            serde_yaml::Value::Mapping(map) => {
                if let Some(serde_yaml::Value::String(path)) =
                    map.get(&serde_yaml::Value::from("from"))
                {
                    let default_val = map.get(&serde_yaml::Value::from("default")).cloned();
                    if let Some(default_val) = default_val {
                        Ok(InputValue::RefWithDefault {
                            path: path.clone(),
                            default: default_val,
                        })
                    } else {
                        Ok(InputValue::Ref { path: path.clone() })
                    }
                } else {
                    Ok(InputValue::Default(serde_yaml::Value::Mapping(map)))
                }
            }
            other => Ok(InputValue::Default(other)),
        }
    }
}

/// Validate a ModuleNode according to schema rules
fn validate_module_node(module: &ModuleNode, modules_dir: &Path) -> Result<()> {
    // 1. id should not be set by user
    if !module.id.is_empty() {
        anyhow::bail!("Module 'id' must not be set by user; it is auto-generated.");
    }
    // 2. source must be set and correspond to a terraform project dirname
    if module.source.is_empty() {
        anyhow::bail!("Module 'source' must be set and non-empty.");
    }
    let tf_dir = modules_dir.join(&module.source);
    if !tf_dir.is_dir() {
        anyhow::bail!(
            "Module 'module' must correspond to a directory in modules_dir: {:?}",
            tf_dir
        );
    }
    // 3. variables must be empty
    if !module.variables.is_empty() {
        anyhow::bail!("Module 'variables' must be empty; only orchestrator sets variables.");
    }
    // 5. scope_ids must not be set
    if !module.scope_ids.is_empty() {
        anyhow::bail!("Module 'scope_ids' must not be set by user; it is auto-populated.");
    }

    Ok(())
}

impl<'de> Deserialize<'de> for InfraFile {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw: serde_yaml::Mapping = Deserialize::deserialize(deserializer)?;
        let mut nodes = HashMap::new();
        let mut source_defaults = HashMap::new();
        let modules_dir = Path::new("modules"); // Default modules dir
        for (key, value) in raw {
            let key_str = key.as_str().unwrap_or("<invalid>").to_string();

            if key_str == "source_default" {
                let defaults_map = value
                    .as_mapping()
                    .ok_or_else(|| serde::de::Error::custom("source_default must be a mapping"))?;

                for (src_key, src_val) in defaults_map {
                    let src_str = src_key.as_str().unwrap_or("<invalid>").to_string();
                    let defaults: ModuleDefaults = serde_yaml::from_value(src_val.clone())
                        .map_err(serde::de::Error::custom)?;
                    source_defaults.insert(src_str, defaults);
                }
            } else {
                let node = parse_infra_node(&value, &key_str, modules_dir)
                    .map_err(|e| serde::de::Error::custom(e.to_string()))?;
                nodes.insert(key_str, node);
            }
        }

        Ok(InfraFile {
            nodes,
            source_defaults,
        })
    }
}

/// Parse a node (module or scope)
fn parse_infra_node(
    value: &Value,
    path: &str,
    modules_dir: &Path,
) -> Result<InfraNode, InfraError> {
    let map = value
        .as_mapping()
        .ok_or_else(|| InfraError::InvalidStructure(format!("expected mapping at {path}")))?;

    if map.contains_key(&Value::from("source")) {
        // Module
        let mut module: ModuleNode = serde_yaml::from_value(value.clone())?;
        validate_module_node(&module, modules_dir)
            .map_err(|e| InfraError::InvalidStructure(e.to_string()))?;
        module.id = path.to_string();

        Ok(InfraNode::Module(module))
    } else if map.contains_key(&Value::from("scope")) {
        // Scope
        let scope_val = map
            .get(&Value::from("scope"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                InfraError::InvalidStructure(format!(
                    "Scope at path '{}' must have a non-empty 'scope'",
                    path
                ))
            })?;

        let mut variables = HashMap::new();
        if let Some(vars) = map
            .get(&Value::from("variables"))
            .and_then(|v| v.as_mapping())
        {
            variables = vars
                .iter()
                .map(|(k, v)| (k.as_str().unwrap_or("<invalid>").to_string(), v.clone()))
                .collect();
        }

        let mut children = HashMap::new();
        for (k, v) in map {
            let key_str = k.as_str().unwrap_or("<invalid>").to_string();
            if key_str != "scope" && key_str != "variables" && v.is_mapping() {
                let child = parse_infra_node(v, &format!("{path}.{key_str}"), modules_dir)?;
                children.insert(key_str, child);
            }
        }
        Ok(InfraNode::Scope(ScopeNode {
            id: path.to_string(),
            scope: scope_val.to_string(),
            variables,
            children,
        }))
    } else {
        Err(InfraError::InvalidStructure(format!(
            "Failed to parse node at {} it must be a scope (have scope key) or a module (have a source key)",
            &path
        )))
    }
}

impl InfraFile {
    /// Load and parse an InfraFile from disk, then apply defaults.
    pub fn from_path(path: &Path) -> Result<Self> {
        let file = File::open(path)
            .with_context(|| format!("Failed to open infra YAML file {:?}", path))?;
        let mut infra: InfraFile = serde_yaml::from_reader(file)
            .with_context(|| format!("Failed to parse YAML file {:?}", path))?;

        // Apply defaults like `source_default`, inheritance, etc.
        infra.apply_defaults();
        infra.add_scope_id_to_childrens();

        Ok(infra)
    }

    /// Merge defaults into all modules recursively
    pub fn apply_defaults(&mut self) {
        fn apply_recursive(node: &mut InfraNode, defaults: &HashMap<String, ModuleDefaults>) {
            match node {
                InfraNode::Module(m) => {
                    if let Some(def) = defaults.get(&m.source) {
                        merge_module_defaults(m, def);
                    }
                }
                InfraNode::Scope(scope) => {
                    for child in scope.children.values_mut() {
                        apply_recursive(child, defaults);
                    }
                }
            }
        }

        for node in self.nodes.values_mut() {
            apply_recursive(node, &self.source_defaults);
        }
    }
    fn add_scope_id_to_childrens(&mut self) {
        fn add_scope_ids_to_childrens_recursive(
            childrens: &mut HashMap<String, InfraNode>,
            scope_ids: &HashSet<String>,
        ) {
            // now recurse / update modules
            for child in childrens.values_mut() {
                match child {
                    InfraNode::Scope(scope) => {
                        let mut scopes_ids = scope_ids.clone();
                        scopes_ids.insert(scope.id.clone());
                        add_scope_ids_to_childrens_recursive(&mut scope.children, &scopes_ids);
                    }
                    InfraNode::Module(m) => {
                        for id in scope_ids {
                            m.scope_ids.insert(id.clone());
                        }
                    }
                }
            }
        }

        for node in self.nodes.values_mut() {
            match node {
                InfraNode::Scope(scope) => {
                    let mut scope_ids = HashSet::new();
                    scope_ids.insert(scope.id.clone());
                    add_scope_ids_to_childrens_recursive(&mut scope.children, &scope_ids);
                }
                InfraNode::Module(_) => continue,
            }
        }
    }
}

/// Merge defaults → module (module overrides defaults)
fn merge_module_defaults(module: &mut ModuleNode, defaults: &ModuleDefaults) {
    // dependencies
    if module.dependencies.is_empty() && !defaults.dependencies.is_empty() {
        module.dependencies = defaults.dependencies.clone();
    }

    // variables
    for (k, v) in &defaults.variables {
        module.variables.entry(k.clone()).or_insert(v.clone());
    }

    // inputs
    for (k, v) in &defaults.inputs {
        module.inputs.entry(k.clone()).or_insert(v.clone());
    }

    // mocked outputs
    if module.mocked_outputs.is_none() && defaults.mocked_outputs.is_some() {
        module.mocked_outputs = defaults.mocked_outputs.clone();
    }
}

//fn resolve_dependencies_ids(infra: InfraFile, module_id: &str, dep_name: &str) -> Option<String> {}
