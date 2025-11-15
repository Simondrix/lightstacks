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

output "main_lb" {
  value = "alb-111"
}

output "public_subnets" {
  value = [
    "subnet-111",
    "subnet-222",
  ]
}

output "vpc" {
  value = {
    name = "vpc31"
    id   = 31
    ipam = {
      cidr = "172.30.0.0/16"
      subnet_addr = [
      "172.30.0.0/24",
      "172.30.1.0/24",
     ]
    }
  }
}