#!/usr/bin/env bash
set -euo pipefail

repo=FerrLabs/LFSX
image=nixos/nix
store_volume=lfsx-nix-store
maintainers='with lib.maintainers; [ bryanfrd ]'
fake=sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
host_here="$(cd "$here" && { pwd -W 2>/dev/null || pwd; })"
build="$here/build"

version="${1:-}"
if [ -z "$version" ]; then
  if command -v gh >/dev/null 2>&1; then
    version="$(gh release view --repo "$repo" --json tagName --jq .tagName)"
  else
    version="$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" |
      sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
  fi
fi
version="${version#v}"
[ -n "$version" ] || { echo "could not determine a version" >&2; exit 1; }
echo "==> lfsx $version"

mkdir -p "$build"

render() { # render <maintainers-expression> <src-hash> <cargo-hash> <destination>
  cat > "$4" <<EOF
{
  lib,
  rustPlatform,
  fetchFromGitHub,
  versionCheckHook,
  nix-update-script,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "lfsx";
  version = "$version";

  src = fetchFromGitHub {
    owner = "FerrLabs";
    repo = "LFSX";
    tag = "v\${finalAttrs.version}";
    hash = "$2";
  };

  cargoHash = "$3";

  doInstallCheck = true;
  nativeInstallCheckInputs = [ versionCheckHook ];
  versionCheckProgram = "\${placeholder "out"}/bin/lfsx";
  versionCheckProgramArg = "--version";

  passthru.updateScript = nix-update-script { };

  meta = {
    description = "Fast, lightweight, secure Git LFS server";
    homepage = "https://lfsx.dev";
    changelog = "https://github.com/FerrLabs/LFSX/releases/tag/v\${finalAttrs.version}";
    license = lib.licenses.mpl20;
    maintainers = $1;
    mainProgram = "lfsx-server";
    platforms = lib.platforms.unix;
  };
})
EOF
}

cat > "$build/default.nix" <<'EOF'
let
  pkgs = import (fetchTarball "https://github.com/NixOS/nixpkgs/archive/nixpkgs-unstable.tar.gz") { };
in
pkgs.callPackage ./package.nix { }
EOF

nix_in_docker() {
  MSYS_NO_PATHCONV=1 docker run --rm \
    -v "$store_volume:/nix" \
    -v "$host_here/build:/w" \
    -w /w "$image" "$@"
}

src_hash="$fake"
cargo_hash="$fake"
log="$build/nix-build.log"

for attempt in 1 2 3; do
  render "[ ]" "$src_hash" "$cargo_hash" "$build/package.nix"
  echo "==> nix-build (attempt $attempt)"
  if nix_in_docker nix-build > "$log" 2>&1; then
    break
  fi
  got="$(sed -n 's/.*got: *\(sha256-[A-Za-z0-9+/=]*\).*/\1/p' "$log" | tail -1)"
  if [ -z "$got" ]; then
    echo "build failed for a reason other than a hash mismatch, tail of $log:" >&2
    tail -30 "$log" >&2
    exit 1
  fi
  if [ "$src_hash" = "$fake" ]; then
    src_hash="$got"
    echo "    src hash:   $src_hash"
  else
    cargo_hash="$got"
    echo "    cargo hash: $cargo_hash"
  fi
done

if [ "$src_hash" = "$fake" ] || [ "$cargo_hash" = "$fake" ]; then
  echo "did not resolve both hashes, see $log" >&2
  exit 1
fi

echo "==> ./result/bin/lfsx --version"
reported="$(nix_in_docker ./result/bin/lfsx --version 2>&1 | tr -d '')"
case "$reported" in
  *"$version"*) echo "    $reported" ;;
  *) echo "the built binary reports \"$reported\", expected $version" >&2; exit 1 ;;
esac

echo "==> nixfmt --check"
render "$maintainers" "$src_hash" "$cargo_hash" "$here/package.nix"
mkdir -p "$build/final"
cp "$here/package.nix" "$build/final/package.nix"
if ! nix_in_docker nix-shell -p nixfmt --run "nixfmt --check final/package.nix" > "$build/nixfmt.log" 2>&1; then
  echo "package.nix is not nixfmt-clean:" >&2
  cat "$build/nixfmt.log" >&2
  exit 1
fi

render "[ ]" "$src_hash" "$cargo_hash" "$build/package.nix"

cat <<EOF

==> done
  version:    $version
  src hash:   $src_hash
  cargo hash: $cargo_hash

  package.nix written to $here/package.nix
  copy it to pkgs/by-name/lf/lfsx/package.nix in a nixpkgs checkout

  binaries built and version-checked inside the nix sandbox; the build log is
  at $log
EOF
