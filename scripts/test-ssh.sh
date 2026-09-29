#!/bin/bash
# SSH scenario: sshd on one computer, ssh from another. Wraps
# scripts/run-scenarios.sh.
exec "$(dirname "$0")/run-scenarios.sh" ssh "$@"
