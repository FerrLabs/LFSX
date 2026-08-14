#!/usr/bin/env bash
set -euo pipefail

lfsx_port=${LFSX_PORT:-8080}
forge_port=${FORGE_PORT:-8099}
token=e2e-token
namespace=FerrLabs/LFSX
lfsx="http://127.0.0.1:${lfsx_port}"

target=$(cargo metadata --format-version 1 --no-deps |
	sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p' | tr '\\' '/')
exe=""
case "$(uname -s)" in
	MINGW* | MSYS* | CYGWIN*) exe=".exe" ;;
esac

work=$(mktemp -d)
forge_pid=""
server_pid=""

cleanup() {
	[ -n "$server_pid" ] && kill "$server_pid" 2>/dev/null || true
	[ -n "$forge_pid" ] && kill "$forge_pid" 2>/dev/null || true
	rm -rf "$work"
}
trap cleanup EXIT

fail() {
	echo "e2e: $1" >&2
	exit 1
}

wait_for() {
	for _ in $(seq 50); do
		if curl -fsS "$1" >/dev/null 2>&1; then
			return 0
		fi
		sleep 0.2
	done
	fail "$1 never came up"
}

cargo build --release --bin lfsx-server --example stub-forge

"${target}/release/examples/stub-forge${exe}" "$forge_port" "$token" &
forge_pid=$!

LFSX_BIND="127.0.0.1:${lfsx_port}" \
	LFSX_PUBLIC_URL="$lfsx" \
	LFSX_STORAGE_ROOT="${work}/objects" \
	LFSX_GITHUB_API_URL="http://127.0.0.1:${forge_port}" \
	"${target}/release/lfsx-server${exe}" &
server_pid=$!

wait_for "${lfsx}/health"

export GIT_CONFIG_GLOBAL="${work}/gitconfig"
export GIT_CONFIG_NOSYSTEM=1
git config --global user.email e2e@lfsx.test
git config --global user.name "LFSX end to end"
git config --global init.defaultBranch main
git config --global credential.helper "!f() { echo username=git; echo password=${token}; }; f"
git lfs install --skip-repo

echo "--- an anonymous request is challenged"
headers="${work}/challenge.headers"
status=$(curl -s -o /dev/null -D "$headers" -w '%{http_code}' \
	-X POST "${lfsx}/${namespace}/objects/batch" \
	-H 'content-type: application/vnd.git-lfs+json' \
	-d '{"operation":"download","objects":[]}')
[ "$status" = 401 ] || fail "anonymous batch answered ${status}, expected 401"
grep -qi '^www-authenticate:' "$headers" || fail "no WWW-Authenticate on the 401"
grep -qi '^lfs-authenticate:' "$headers" || fail "no LFS-Authenticate on the 401"

echo "--- the locking endpoint answers"
status=$(curl -s -o /dev/null -w '%{http_code}' \
	-X POST "${lfsx}/${namespace}/locks/verify" \
	-H "authorization: Bearer ${token}" \
	-H 'content-type: application/vnd.git-lfs+json' -d '{}')
[ "$status" = 200 ] || fail "locks/verify answered ${status}, expected 200"

echo "--- push a repository through the real client"
git init --bare --quiet "${work}/origin.git"
git init --quiet "${work}/repo"
cd "${work}/repo"
git remote add origin "${work}/origin.git"

printf '[lfs]\n\turl = %s/%s\n' "$lfsx" "$namespace" >.lfsconfig
git lfs install --local
git lfs track '*.bin' >/dev/null

mkdir -p assets
head -c 67108864 /dev/urandom >assets/huge.bin
head -c 1024 /dev/urandom >assets/small.bin

git add -A
git commit --quiet -m "add assets"
git push --quiet origin main >"${work}/push.log" 2>&1 || {
	cat "${work}/push.log" >&2
	fail "push failed"
}

grep -Eqi '^(error|fatal)' "${work}/push.log" && {
	cat "${work}/push.log" >&2
	fail "the client reported an error while pushing"
}

echo "--- the object landed in the documented layout"
oid=$(sha256sum <assets/huge.bin | cut -d' ' -f1)
stored="${work}/objects/${namespace}/${oid:0:2}/${oid:2:2}/${oid}"
[ -f "$stored" ] || fail "expected the object at ${stored}"

echo "--- clone it back and compare the bytes"
git clone --quiet "${work}/origin.git" "${work}/clone"

for asset in huge.bin small.bin; do
	original=$(sha256sum <"${work}/repo/assets/${asset}" | cut -d' ' -f1)
	restored=$(sha256sum <"${work}/clone/assets/${asset}" | cut -d' ' -f1)
	if [ "$original" != "$restored" ]; then
		fail "${asset} came back as ${restored} ($(wc -c <"${work}/clone/assets/${asset}") bytes), expected ${original}"
	fi
done

head -c 40 "${work}/clone/assets/huge.bin" | grep -q 'git-lfs' && \
	fail "the clone kept the pointer instead of the content"

if [ "$(uname -s)" = "Linux" ]; then
	echo "--- SIGTERM shuts the server down cleanly"
	kill -TERM "$server_pid"
	if ! wait "$server_pid"; then
		fail "the server did not exit cleanly on SIGTERM"
	fi
	server_pid=""
fi

echo "--- lock a scene the way an artist would"
git lfs lock assets/small.bin >"${work}/lock.log" 2>&1 || {
	cat "${work}/lock.log" >&2
	fail "git lfs lock failed"
}

git lfs locks >"${work}/locks.log" 2>&1
grep -q 'assets/small.bin' "${work}/locks.log" || {
	cat "${work}/locks.log" >&2
	fail "the lock is not listed by the client"
}

if git lfs lock assets/small.bin >"${work}/relock.log" 2>&1; then
	fail "the client was allowed to take a lock that is already held"
fi

git lfs unlock assets/small.bin >"${work}/unlock.log" 2>&1 || {
	cat "${work}/unlock.log" >&2
	fail "git lfs unlock failed"
}

git lfs locks >"${work}/locks.log" 2>&1
grep -q 'assets/small.bin' "${work}/locks.log" && fail "the lock outlived its unlock"

echo "e2e: ok"
