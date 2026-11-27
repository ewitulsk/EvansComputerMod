locals {
  fqdn            = "${var.subdomain}.${var.domain}"
  spaces_region   = var.spaces_region != "" ? var.spaces_region : var.region
  packages_bucket = "${var.bucket_prefix}-packages"
  db_bucket       = "${var.bucket_prefix}-db"

  authorized_keys = compact([
    trimspace(var.admin_ssh_public_key),
    trimspace(var.ci_ssh_public_key),
  ])

  # The non-secret files of infra/docker, put on the droplet at first boot so
  # that a first deploy needs only a .env. Later deploys overwrite them.
  docker_dir = "${path.module}/../docker"
  compose_files = {
    "docker-compose.yml" = "0644"
    "Caddyfile"          = "0644"
    "litestream.yml"     = "0644"
    ".env.example"       = "0644"
    "deploy.sh"          = "0755"
    "backup-check.sh"    = "0755"
  }
}

# --- SSH key -----------------------------------------------------------------
# Registered with the account and given to the droplet so that DigitalOcean
# does not generate and e-mail a root password. Root login is then disabled by
# cloud-init; the same key is installed for the deploy user.
resource "digitalocean_ssh_key" "admin" {
  name       = "${var.name}-admin"
  public_key = trimspace(var.admin_ssh_public_key)
}

# --- Droplet -----------------------------------------------------------------
# No vpc_uuid: the droplet joins the region's default VPC. With one droplet
# there is no private traffic to isolate.
resource "digitalocean_droplet" "this" {
  name       = var.name
  region     = var.region
  size       = var.droplet_size
  image      = var.droplet_image
  ssh_keys   = [digitalocean_ssh_key.admin.fingerprint]
  monitoring = true
  backups    = var.droplet_backups
  ipv6       = false
  tags       = [var.name]

  # A change of droplet_size resizes CPU and memory only, so it can be undone
  # (a grown disk cannot shrink). The databases and packages are in Spaces;
  # the 25 GB disk of the smallest size is enough.
  resize_disk = false

  user_data = templatefile("${path.module}/cloud-init.yaml.tftpl", {
    deploy_user     = var.deploy_user
    deploy_dir      = var.deploy_dir
    app_uid         = var.app_uid
    authorized_keys = local.authorized_keys
    files = {
      for name, mode in local.compose_files :
      name => { mode = mode, content = base64gzip(file("${local.docker_dir}/${name}")) }
    }
  })

  lifecycle {
    # user_data and image only matter at first boot, and a change to either
    # would otherwise destroy and recreate the droplet. To rebuild on purpose:
    #   terraform apply -replace=digitalocean_droplet.this
    # (docs/DEPLOY.md, "Restore onto a new droplet").
    ignore_changes = [user_data, image, ssh_keys]
  }
}

# --- Reserved IP -------------------------------------------------------------
# The address DNS points at. It survives a droplet rebuild.
resource "digitalocean_reserved_ip" "this" {
  region = var.region
}

resource "digitalocean_reserved_ip_assignment" "this" {
  ip_address = digitalocean_reserved_ip.this.ip_address
  droplet_id = digitalocean_droplet.this.id
}

# --- Firewall ----------------------------------------------------------------
resource "digitalocean_firewall" "this" {
  name        = "${var.name}-fw"
  droplet_ids = [digitalocean_droplet.this.id]

  # SSH: only from ssh_allowed_cidrs. With the default (empty list) there is
  # no rule at all and port 22 is closed.
  dynamic "inbound_rule" {
    for_each = length(var.ssh_allowed_cidrs) > 0 ? [1] : []
    content {
      protocol         = "tcp"
      port_range       = "22"
      source_addresses = var.ssh_allowed_cidrs
    }
  }

  inbound_rule {
    protocol         = "tcp"
    port_range       = "80"
    source_addresses = ["0.0.0.0/0", "::/0"]
  }

  inbound_rule {
    protocol         = "tcp"
    port_range       = "443"
    source_addresses = ["0.0.0.0/0", "::/0"]
  }

  inbound_rule {
    protocol         = "icmp"
    source_addresses = ["0.0.0.0/0", "::/0"]
  }

  outbound_rule {
    protocol              = "tcp"
    port_range            = "1-65535"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }

  outbound_rule {
    protocol              = "udp"
    port_range            = "1-65535"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }

  outbound_rule {
    protocol              = "icmp"
    destination_addresses = ["0.0.0.0/0", "::/0"]
  }
}

# --- Spaces buckets ----------------------------------------------------------
# Both private. repo-api reads and writes the package files itself; nothing is
# served from the bucket directly.
resource "digitalocean_spaces_bucket" "packages" {
  name          = local.packages_bucket
  region        = local.spaces_region
  acl           = "private"
  force_destroy = var.bucket_force_destroy

  versioning {
    enabled = true
  }

  lifecycle_rule {
    id                                     = "expire-noncurrent"
    enabled                                = true
    abort_incomplete_multipart_upload_days = 2

    noncurrent_version_expiration {
      days = var.packages_noncurrent_days
    }
  }
}

resource "digitalocean_spaces_bucket" "db" {
  name          = local.db_bucket
  region        = local.spaces_region
  acl           = "private"
  force_destroy = var.bucket_force_destroy

  versioning {
    enabled = true
  }

  lifecycle_rule {
    id                                     = "expire-noncurrent"
    enabled                                = true
    abort_incomplete_multipart_upload_days = 2

    noncurrent_version_expiration {
      days = var.db_noncurrent_days
    }
  }
}

# --- Project (optional) ------------------------------------------------------
resource "digitalocean_project" "this" {
  count       = var.project_name != "" ? 1 : 0
  name        = var.project_name
  description = "ECM package repository backend"
  purpose     = "Web Application"
  environment = "Production"
}

resource "digitalocean_project_resources" "this" {
  count   = var.project_name != "" ? 1 : 0
  project = digitalocean_project.this[0].id
  resources = [
    digitalocean_droplet.this.urn,
    digitalocean_reserved_ip.this.urn,
    digitalocean_spaces_bucket.packages.urn,
    digitalocean_spaces_bucket.db.urn,
  ]

  # The reserved IP can only be moved between projects once it is assigned.
  depends_on = [digitalocean_reserved_ip_assignment.this]
}

# --- DNS (optional) ----------------------------------------------------------
# Only when the zone is hosted at DigitalOcean. The zone itself is not managed
# here: it must already exist under Networking > Domains.
resource "digitalocean_record" "a" {
  count  = var.manage_dns ? 1 : 0
  domain = var.domain
  type   = "A"
  name   = var.subdomain
  value  = digitalocean_reserved_ip.this.ip_address
  ttl    = var.dns_ttl
}
