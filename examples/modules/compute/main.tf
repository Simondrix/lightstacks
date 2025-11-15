terraform {
  required_providers {
    null = {
      source  = "hashicorp/null"
      version = "~> 3.0"
    }
  }
}

# This resource does nothing
resource "null_resource" "dummy" {}

output "instance_id" {
  value = "linux-1"
}

output "subnets" {
  value = var.subnets_ids
}
output "security_group_id" {
  value = "sg-111"
}

variable subnets_ids {
  type = list(string)
}

variable account_name {
  type = string
}

variable subnet_addr {
  type = string
}

variable vpc_cidr {
  type = string
}