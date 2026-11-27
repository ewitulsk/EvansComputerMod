terraform {
  required_version = ">= 1.6.0"

  required_providers {
    digitalocean = {
      source  = "digitalocean/digitalocean"
      version = "~> 2.50"
    }
  }
}

# Credentials come from the environment, never from files in this directory:
#   DIGITALOCEAN_TOKEN        the API token (droplet, firewall, reserved IP, DNS)
#   SPACES_ACCESS_KEY_ID      a Spaces key that may create buckets
#   SPACES_SECRET_ACCESS_KEY
# See docs/DEPLOY.md, step 3.
provider "digitalocean" {}
