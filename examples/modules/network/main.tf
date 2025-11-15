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