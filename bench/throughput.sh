#!/usr/bin/env bash
set -euo pipefail

# Measures the server, not the client: transfers go over raw HTTP so the numbers
# are not diluted by git-lfs's own bookkeeping. Sizes are overridable because a
# laptop and a CI runner are not asked the same question.
LARGE_MIB=${LARGE_MIB:-1024}
SMALL_COUNT=${SMALL_COUNT:-500}
SMALL_KIB=${SMALL_KIB:-64}
PORT=${PORT:-8091}
NAMESPACE=Bench/Throughput


work=$(mktemp -d)
server_pid=""
cleanup() {
	[ -n "$server_pid" ] && kill "$server_pid" 2>/dev/null || true
	rm -rf "$work"
}
trap cleanup EXIT

resident_kib() {
	case "$(uname -s)" in
		Linux) awk '/VmRSS/ {print $2}' "/proc/$1/status" 2>/dev/null || echo 0 ;;
		*) echo 0 ;;
	esac
}

now_ms() {
	date +%s%3N
}

seconds_since() {
	awk -v start="$1" -v now="$(now_ms)" 'BEGIN { printf("%.2f", (now - start) / 1000) }'
}

rate_mib() {
	awk -v mib="$1" -v secs="$2" 'BEGIN { printf("%.0f", secs > 0 ? mib / secs : 0) }'
}

cargo build --release --bin lfsx-server >/dev/null 2>&1

LFSX_BIND="127.0.0.1:${PORT}" \
	LFSX_STORAGE_ROOT="${work}/objects" \
	LFSX_AUTH=disabled \
	cargo run --quiet --release --bin lfsx-server >/dev/null 2>&1 &
server_pid=$!

for _ in $(seq 50); do
	curl -fsS "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1 && break
	sleep 0.2
done

baseline_kib=$(resident_kib "$server_pid")

echo "generating ${LARGE_MIB} MiB"
head -c $((LARGE_MIB * 1024 * 1024)) /dev/urandom >"${work}/large.bin"
large_oid=$(sha256sum "${work}/large.bin" | cut -d' ' -f1)

echo "upload"
started=$(now_ms)
curl -fsS -X PUT --data-binary "@${work}/large.bin" \
	"http://127.0.0.1:${PORT}/${NAMESPACE}/objects/${large_oid}" >/dev/null
upload_secs=$(seconds_since "$started")
peak_kib=$(resident_kib "$server_pid")

echo "download"
started=$(now_ms)
curl -fsS "http://127.0.0.1:${PORT}/${NAMESPACE}/objects/${large_oid}" -o "${work}/back.bin"
download_secs=$(seconds_since "$started")
during_kib=$(resident_kib "$server_pid")
[ "$during_kib" -gt "$peak_kib" ] && peak_kib=$during_kib

cmp -s "${work}/large.bin" "${work}/back.bin" || {
	echo "the object did not come back byte for byte" >&2
	exit 1
}

echo "generating ${SMALL_COUNT} objects of ${SMALL_KIB} KiB"
mkdir -p "${work}/small"
for i in $(seq "$SMALL_COUNT"); do
	head -c $((SMALL_KIB * 1024)) /dev/urandom >"${work}/small/${i}.bin"
done

echo "small objects"
started=$(now_ms)
for i in $(seq "$SMALL_COUNT"); do
	oid=$(sha256sum "${work}/small/${i}.bin" | cut -d' ' -f1)
	curl -fsS -X PUT --data-binary "@${work}/small/${i}.bin" \
		"http://127.0.0.1:${PORT}/${NAMESPACE}/objects/${oid}" >/dev/null
done
small_secs=$(seconds_since "$started")

if [ "$peak_kib" -gt 0 ]; then
	memory="$((baseline_kib / 1024)) MiB → $((peak_kib / 1024)) MiB"
else
	memory="not measured on $(uname -s), /proc is where this is read"
fi

small_mib=$(awk -v c="$SMALL_COUNT" -v k="$SMALL_KIB" 'BEGIN { printf("%.0f", c * k / 1024) }')
per_object=$(awk -v s="$small_secs" -v c="$SMALL_COUNT" 'BEGIN { printf("%.1f", c > 0 ? s * 1000 / c : 0) }')

hardware="$(uname -sr)"
if [ -r /proc/cpuinfo ]; then
	cores=$(grep -c '^processor' /proc/cpuinfo)
	memory_gib=$(awk '/MemTotal/ { printf("%.0f", $2 / 1024 / 1024) }' /proc/meminfo)
	hardware="${hardware}, ${cores} cores, ${memory_gib} GiB"
fi

cat <<REPORT

Ran on: ${hardware}

| Measure | Result |
|---|---|
| Upload, ${LARGE_MIB} MiB single object | $(rate_mib "$LARGE_MIB" "$upload_secs") MiB/s |
| Download, ${LARGE_MIB} MiB single object | $(rate_mib "$LARGE_MIB" "$download_secs") MiB/s |
| ${SMALL_COUNT} objects of ${SMALL_KIB} KiB, sequential | ${per_object} ms per object, $(rate_mib "$small_mib" "$small_secs") MiB/s |
| Resident memory, idle → peak | ${memory} |
REPORT
