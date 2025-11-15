# tfstacks
tfstacks is a lightweight CLI tool to run Terraform modules with dependency management.
It allows you to define modules, their dependencies, inputs, variables, and mocked outputs declaratively in YAML.
## Installation
```bash
cargo install tfstacks
```
## Project Structure Example
```
deployments/
├─ infra.yaml
modules/
├─ webapp/
│ └─ main.tf
├─ compute/
│ └─ main.tf
└─ vpc/
  └─ main.tf
```
## How to use 
```
Commands:
  plan     Plan the module
  apply    Apply the module
  destroy  Destroy the module
  help     Print this message or the help of the given subcommand(s)

Options:
      --infra-file <INFRA_FILE>    Path to the infrastructure YAML file [env: TFSTACKS_INFRA_FILE=] [default: deployments/infra_example.yaml]
      --module-id <MODULE_ID>      Target module ID (e.g., "account-1.tenant-a.webapp")
      --cache-dir <CACHE_DIR>      [env: TFSTACKS_CACHE_DIR=] [default: /tmp/.tfstacks_cache]
      --modules-dir <MODULES_DIR>  [env: TFSTACKS_MODULES_DIR=] [default: modules]
      --bin-path <BIN_PATH>        [env: TFSTACKS_TF_BIN=] [default: terraform]
  -h, --help                       Print help
```
## YAML Infrastructure Schema
The infrastructure YAML file defines the hierarchy of scopes and modules.
Scopes can be nested (e.g., account → tenant) and contain modules or other scopes.
Modules represent Terraform stacks.
### Scope Node
```
<scope_name>:
  scope: <string> # e.g., account, tenant
  variables: # optional variables available to child modules
    var1: value1
    var2: value2
  <child_modules_or_scope>:
...
```

### Module Node
```
<module_name>:
  source: <path_to_module> # required
  dependencies: # optional, list of sources of modules this module depends on
    - compute
    - vpc
  inputs: # optional, maps dependency outputs or constants to Terraform variables
    <target_variable_name>: <value>
    <target_variable_name>: 
      from: <module_source>.<output_name>.<optional_output_attribut_path> or <scope_name>.<variable_name>.<optional_variable_attribut_path>
      default: <default_value_if_output_not_found>
  mocked_outputs: # optional, for testing without applying Terraform
```
### Source Defaults
```
source_default:
<module_source_name>:
  dependencies: [...] # default dependencies applied to all modules of this source
  inputs: {...} # default inputs merged into modules
  mocked_outputs: {...} # default mocked outputs
```
## How Dependencies Work
1. Within Scope and Parent Scope
- A module can only depend on other modules that exist in the same scope (folder/section in YAML) or in a parent scope above it.
- Example: account-1.tenant-a.webapp can depend on account-1.compute (parent scope) but not on account-2.compute (another account).
2. Cross-Scope Restrictions
- Modules cannot depend on sibling or unrelated scopes outside their hierarchy.
- This prevents mistakes like accidentally using resources from another account or tenant.
3. Merging Defaults
- Every module inherits settings from source_default based on its source.
- Example: if all webapp modules need vpc and compute as default dependencies, you define it once in source_default.
- Module-specific definitions override defaults if there is a conflict (e.g., custom variables or inputs).
Think of scopes as folders and modules as files inside the folder. Dependencies can see “upwards” to parent folders but not sideways into other folders.
## Example Infrastructure YAML
```yaml
account-1:
  scope_type: account
  scope_variables:
    name: account-1
    type: aws
    environment: prod
  compute:
    source: "compute"
    variables:
      instance_type: "t3.medium"
  vpc:
    source: "vpc"
  tenant-a: 
    scope_type: tenant
    scope_variables:
      id: "tenant-a"
      users: 
        - "user1"
        - "user2"
    webapp:
      source: "webapp"
      dependencies:
        - compute
        - vpc
      variables:
        app_name: "webapp-a"

    webapp2:
      source: "webapp"
      variables:
        app_name: "webapp-b"
  tenant:
    scope_variables:
      id: "tenant-b"
      users: 
        - "user1"
        - "user2"
    webapp3:
      source: "webapp"
      variables:
        app_name: "webapp-c"
    
account-2:
  vpc:
    source: "vpc"
  compute:
    source: "compute"
    variables:
      instance_type: "t3.medium"
  tenant-a.webapp:
    source: "webapp"
    variables:
      app_name: "webapp-c"
      inputs:
        subnet: compute.subnets[1]

source_default:
  #first key must match source name
  webapp:
    dependencies:
      - compute
      - vpc
    inputs:
      lb: vpc.main_lb
      subnet: compute.subnets[0]
      region:
        path: vpc.region
        default: "us-east-1"
  compute:
    dependencies:
      - "vpc"
    inputs:
      subnets: vpc.public_subnets
    variables:
      tags:
        managed_by_terraform: "true"
    mocked_outputs:
      instance_id: "alb-111"
      security_group_id: "sg-111"
      subnets:
        - "subnet-111"
    vpc:
      mocked_outputs:
        main_lb: "alb-111"
        public_subnets:
          - "subnet-111"
          - "subnet-222"

```
## Usage
```bash
tfstacks run --module-id account-2.tenant-c.webapp --infra-file deployements/infra1.yaml apply
```
## Terraform Actions
- plan → Preview changes
- apply → Apply changes
- destroy → Destroy resources
## Best Practices
- Keep module name unique to simplify dependency resolution.
- Apply dependencies before running dependent modules.
- Use scope_variables to define reusable variables for child modules.
- Use source_default to avoid repeating common settings across multiple modules.
- Inputs can reference dependency outputs or provide default values.