#!/bin/bash
# Shell/process scenarios: child output, pipes, redirects, stdin, jobs,
# exit codes, Ctrl+T. Wraps scripts/run-scenarios.sh.
exec "$(dirname "$0")/run-scenarios.sh" boot_echo processes ctrl_t "$@"
