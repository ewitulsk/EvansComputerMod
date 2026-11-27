variable "name" {
  description = "Name of the droplet and prefix of the firewall and SSH key names."
  type        = string
  default     = "ecm-repo"

  validation {
    condition     = can(regex("^[a-z0-9][a-z0-9-]{0,40}$", var.name))
    error_message = "Use lower-case letters, digits and hyphens."
  }
}

variable "region" {
  description = "DigitalOcean region of the droplet and the reserved IP."
  type        = string
  default     = "nyc3"
}

variable "spaces_region" {
  description = "Region of the Spaces buckets. Empty means the droplet's region. Not every region has Spaces."
  type        = string
  default     = ""
}

variable "droplet_size" {
  description = "Droplet size slug. s-1vcpu-1gb is the smallest that runs this stack comfortably."
  type        = string
  default     = "s-1vcpu-1gb"
}

variable "droplet_image" {
  description = "Droplet image slug (an Ubuntu LTS release)."
  type        = string
  default     = "ubuntu-24-04-x64"
}

variable "droplet_backups" {
  description = "Weekly DigitalOcean droplet backups (costs extra). The databases are already replicated by Litestream and the packages live in Spaces, so this is off by default."
  type        = bool
  default     = false
}

variable "admin_ssh_public_key" {
  description = "Your SSH public key (the whole line, for example 'ssh-ed25519 AAAA... you@laptop'). It is installed for the deploy user."
  type        = string

  validation {
    condition     = can(regex("^(ssh-ed25519|ssh-rsa|ecdsa-sha2-nistp(256|384|521)|sk-ssh-ed25519@openssh.com|sk-ecdsa-sha2-nistp256@openssh.com) [A-Za-z0-9+/=]+", trimspace(var.admin_ssh_public_key)))
    error_message = "Give one OpenSSH public key line (the contents of a .pub file), not a private key or a path."
  }
}

variable "ci_ssh_public_key" {
  description = "Optional second SSH public key for the deploy user: the key GitHub Actions deploys with. Empty means none."
  type        = string
  default     = ""
}

variable "deploy_user" {
  description = "Login name of the unprivileged user that owns the compose directory and runs deploys."
  type        = string
  default     = "deploy"

  validation {
    condition     = can(regex("^[a-z][a-z0-9_-]{0,30}$", var.deploy_user)) && var.deploy_user != "root"
    error_message = "Use a normal Unix user name other than root."
  }
}

variable "deploy_dir" {
  description = "Directory on the droplet that holds docker-compose.yml, .env and the data directory."
  type        = string
  default     = "/opt/ecm-repo"
}

variable "app_uid" {
  description = "Numeric user and group id the containers run as. Must equal APP_UID and APP_GID in the droplet's .env."
  type        = number
  default     = 10001
}

variable "ssh_allowed_cidrs" {
  description = <<-EOT
    CIDR blocks that may reach SSH (port 22). The default is the empty list:
    port 22 is closed to everyone and you cannot log in until you set this.
    Use your own address, for example ["203.0.113.7/32"]. GitHub-hosted
    runners have no stable address, so deploying from GitHub Actions needs
    ["0.0.0.0/0", "::/0"]; logins are key-only and fail2ban is installed.
  EOT
  type        = list(string)
  default     = []

  validation {
    condition     = alltrue([for c in var.ssh_allowed_cidrs : can(cidrhost(c, 0))])
    error_message = "Every entry must be a CIDR block such as 203.0.113.7/32."
  }
}

variable "bucket_prefix" {
  description = "Prefix of the two Spaces buckets, <prefix>-packages and <prefix>-db. Bucket names are global across all DigitalOcean customers, so pick something that is yours."
  type        = string

  validation {
    condition     = can(regex("^[a-z0-9][a-z0-9-]{1,50}[a-z0-9]$", var.bucket_prefix))
    error_message = "Use 3 to 52 lower-case letters, digits and hyphens."
  }
}

variable "packages_noncurrent_days" {
  description = "Days an overwritten or deleted package file is kept as a noncurrent version before it is removed for good."
  type        = number
  default     = 30
}

variable "db_noncurrent_days" {
  description = "Days an overwritten or deleted Litestream file is kept as a noncurrent version. Litestream prunes its own files often, so keep this short."
  type        = number
  default     = 7
}

variable "bucket_force_destroy" {
  description = "Let 'terraform destroy' delete the buckets even when they still hold objects. Leave false except for teardown."
  type        = bool
  default     = false
}

variable "project_name" {
  description = "Create a DigitalOcean project with this name and put the resources in it. Empty means use the account's default project."
  type        = string
  default     = ""
}

variable "domain" {
  description = "The DNS zone, for example evanwitulski.com."
  type        = string
  default     = "evanwitulski.com"
}

variable "subdomain" {
  description = "Host name inside the zone. The service is served at <subdomain>.<domain>."
  type        = string
  default     = "repo"
}

variable "manage_dns" {
  description = "Create the A record. Set true only when the zone 'domain' is already hosted at DigitalOcean (Networking > Domains). When false, create the record at your DNS host by hand from the outputs."
  type        = bool
  default     = false
}

variable "dns_ttl" {
  description = "TTL of the A record in seconds (only with manage_dns)."
  type        = number
  default     = 300
}
