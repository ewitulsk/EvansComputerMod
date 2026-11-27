#!/usr/bin/env bash
# Prove that the Litestream replicas in the bucket can be restored.
#
#   ./backup-check.sh
#
# Run it in the compose directory (/opt/ecm-repo) on the droplet. For each
# database it restores the newest replica from the bucket into a temporary
# directory (never over the live file), runs SQLite's integrity check on the
# copy, and prints its size and tables. The live stack keeps running.
# Exit status 0 means both restores passed.

set -euo pipefail

cd "$(dirname "$0")"

[ -f .env ] || {
	echo "backup-check.sh: .env not found in $PWD" >&2
	exit 1
}

uid="$(grep -E '^APP_UID=' .env | tail -n 1 | cut -d= -f2- || true)"
gid="$(grep -E '^APP_GID=' .env | tail -n 1 | cut -d= -f2- || true)"
uid="${uid:-10001}"
gid="${gid:-10001}"

tmp="$(mktemp -d "${TMPDIR:-/tmp}/ecm-restore-check.XXXXXX")"
# shellcheck disable=SC2329  # called by the EXIT trap
cleanup() {
	# Files in there belong to the container user: remove them as root in a
	# container when plain rm is not allowed.
	rm -rf "$tmp" 2>/dev/null ||
		docker run --rm -v "$tmp:/restore" --entrypoint /bin/sh litestream/litestream:0.5.18 -c 'rm -rf /restore/*' || true
	rm -rf "$tmp" 2>/dev/null || true
}
trap cleanup EXIT

# World-writable so the container user can write into it and this user can
# read the result.
chmod 0777 "$tmp"

status=0
for db in auth.db repo.db; do
	printf '\n==> %s\n' "$db"
	if ! docker compose run --rm --no-deps -T \
		--user "$uid:$gid" \
		-v "$tmp:/restore" \
		litestream restore -integrity-check full -o "/restore/$db" "/data/$db"; then
		echo "FAIL: could not restore $db from the bucket" >&2
		status=1
		continue
	fi

	if [ ! -s "$tmp/$db" ]; then
		echo "FAIL: restore of $db produced no file" >&2
		status=1
		continue
	fi

	echo "restored $(wc -c <"$tmp/$db") bytes, Litestream integrity check passed"

	# A second, independent check with the sqlite3 command when the host has
	# it (cloud-init installs it).
	if command -v sqlite3 >/dev/null 2>&1; then
		result="$(sqlite3 "file:$tmp/$db?mode=ro" 'PRAGMA integrity_check;' 2>&1 || true)"
		if [ "$result" = "ok" ]; then
			echo "PRAGMA integrity_check: ok"
			echo "tables: $(sqlite3 "file:$tmp/$db?mode=ro" "SELECT group_concat(name, ' ') FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%';" 2>/dev/null || true)"
		else
			echo "FAIL: PRAGMA integrity_check on $db said: $result" >&2
			status=1
		fi
	else
		echo "(sqlite3 is not installed on this host: skipped the second check)"
	fi
done

echo
if [ "$status" -eq 0 ]; then
	echo "OK: both databases restore from the bucket and pass the integrity check."
else
	echo "FAILED: see above." >&2
fi
exit "$status"
