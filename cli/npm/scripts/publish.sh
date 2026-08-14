#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:?usage: publish.sh <version>}"

declare -A ARCHIVES=(
  ["lfsx-x86_64-unknown-linux-musl.tar.gz"]="linux-x64"
  ["lfsx-aarch64-unknown-linux-musl.tar.gz"]="linux-arm64"
  ["lfsx-x86_64-apple-darwin.tar.gz"]="darwin-x64"
  ["lfsx-aarch64-apple-darwin.tar.gz"]="darwin-arm64"
  ["lfsx-x86_64-pc-windows-msvc.zip"]="win32-x64"
)

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
NPM_DIR="$(dirname "$SCRIPT_DIR")"
REPO_ROOT="$(cd "${NPM_DIR}/../.." && pwd)"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

if [ -n "${NODE_AUTH_TOKEN:-}" ]; then
	printf '//registry.npmjs.org/:_authToken=%s\n' "$NODE_AUTH_TOKEN" >"${WORK_DIR}/.npmrc"
	export NPM_CONFIG_USERCONFIG="${WORK_DIR}/.npmrc"
fi

publish_if_new() {
  if npm view "$1@${VERSION}" version >/dev/null 2>&1; then
    echo "  $1@${VERSION} is already on the registry, skipping"
  else
    npm publish --access public
  fi
}

for archive in "${!ARCHIVES[@]}"; do
  platform="${ARCHIVES[$archive]}"
  echo "${archive} -> lfsx-${platform}"

  gh release download "v${VERSION}" -p "$archive" -D "$WORK_DIR"

  package="${WORK_DIR}/packages/${platform}"
  mkdir -p "${package}/bin"

  if [[ "$archive" == *.zip ]]; then
    unzip -q "${WORK_DIR}/${archive}" -d "${package}/bin/"
  else
    tar xzf "${WORK_DIR}/${archive}" -C "${package}/bin/"
    chmod +x "${package}/bin/lfsx"
  fi

  cp "${NPM_DIR}/platforms/${platform}/package.json" "${package}/package.json"
  cp "${REPO_ROOT}/cli/README.md" "${package}/README.md"
  cp "${REPO_ROOT}/LICENSE" "${package}/LICENSE"

  (
    cd "$package"
    npm version "$VERSION" --no-git-tag-version --allow-same-version >/dev/null
    publish_if_new "lfsx-${platform}"
  )
done

echo "publishing the wrapper"
cp "${REPO_ROOT}/cli/README.md" "${NPM_DIR}/README.md"
cp "${REPO_ROOT}/LICENSE" "${NPM_DIR}/LICENSE"

cd "$NPM_DIR"
npm version "$VERSION" --no-git-tag-version --allow-same-version >/dev/null
node -e "
  const fs = require('fs');
  const pkg = JSON.parse(fs.readFileSync('package.json', 'utf8'));
  for (const dependency of Object.keys(pkg.optionalDependencies ?? {})) {
    pkg.optionalDependencies[dependency] = process.argv[1];
  }
  fs.writeFileSync('package.json', JSON.stringify(pkg, null, 2) + '\n');
" "$VERSION"

publish_if_new lfsx

echo "published lfsx@${VERSION} and its platform packages"
