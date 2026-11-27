output "reserved_ip" {
  description = "The public address of the service. DNS points here."
  value       = digitalocean_reserved_ip.this.ip_address
}

output "droplet_ip" {
  description = "The droplet's own public address (changes when the droplet is rebuilt; use reserved_ip instead)."
  value       = digitalocean_droplet.this.ipv4_address
}

output "droplet_id" {
  value = digitalocean_droplet.this.id
}

output "fqdn" {
  value = local.fqdn
}

output "ssh_command" {
  description = "How to log in (needs your address in ssh_allowed_cidrs)."
  value       = "ssh ${var.deploy_user}@${digitalocean_reserved_ip.this.ip_address}"
}

output "deploy_dir" {
  value = var.deploy_dir
}

output "packages_bucket" {
  description = "S3_BUCKET in the droplet's .env."
  value       = digitalocean_spaces_bucket.packages.name
}

output "db_bucket" {
  description = "LITESTREAM_BUCKET in the droplet's .env."
  value       = digitalocean_spaces_bucket.db.name
}

output "spaces_endpoint" {
  description = "S3_ENDPOINT and LITESTREAM_ENDPOINT in the droplet's .env."
  value       = "https://${local.spaces_region}.digitaloceanspaces.com"
}

output "spaces_region" {
  description = "S3_REGION and LITESTREAM_REGION in the droplet's .env."
  value       = local.spaces_region
}

output "dns_records" {
  description = "The DNS record the service needs."
  value = var.manage_dns ? "Created by Terraform: ${local.fqdn}. A ${digitalocean_reserved_ip.this.ip_address} (TTL ${var.dns_ttl})" : join("\n", [
    "Create this record at the DNS host of ${var.domain}:",
    "  type A   name ${var.subdomain}   value ${digitalocean_reserved_ip.this.ip_address}   TTL 300",
    "Do not create an AAAA record: the droplet has no IPv6 address.",
    "If the zone has CAA records, one of them must allow letsencrypt.org.",
  ])
}

output "env_hints" {
  description = "Lines to copy into the droplet's .env."
  value = join("\n", [
    "DOMAIN=${local.fqdn}",
    "PUBLIC_ORIGIN=https://${local.fqdn}",
    "S3_ENDPOINT=https://${local.spaces_region}.digitaloceanspaces.com",
    "S3_REGION=${local.spaces_region}",
    "S3_BUCKET=${digitalocean_spaces_bucket.packages.name}",
    "LITESTREAM_ENDPOINT=https://${local.spaces_region}.digitaloceanspaces.com",
    "LITESTREAM_REGION=${local.spaces_region}",
    "LITESTREAM_BUCKET=${digitalocean_spaces_bucket.db.name}",
    "APP_UID=${var.app_uid}",
    "APP_GID=${var.app_uid}",
  ])
}
