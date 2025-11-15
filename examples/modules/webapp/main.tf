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

output "success" {
  value = true
}

variable lb {
  type = string
}

variable "subnet" {
  type = string
}

variable app_name {
  type = string
}
