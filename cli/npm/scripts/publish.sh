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

# Two ways in, and the workflow decides which by whether it granted `id-token`.
#
# Trusted publishing is the one to prefer: GitHub mints a short-lived OIDC token
# for this exact workflow, npm checks it against the publisher configured on each
# package, and nothing long-lived exists to expire or to leak. It also sidesteps
# the account's 2FA, which is all an automation token ever bought.
#
# A token stays as the fallback, for running this outside Actions.
if [ -n "${ACTIONS_ID_TOKEN_REQUEST_URL:-}" ]; then
	NEED="11.5.1"
	HAVE="$(npm --version)"
	if [ "$(printf '%s
%s
' "$NEED" "$HAVE" | sort -V | head -1)" != "$NEED" ]; then
		echo "::error::npm ${HAVE} cannot do trusted publishing; ${NEED} or newer is required. Pin it with actions/setup-node." >&2
		exit 1
	fi

	# Any token here would take precedence over the OIDC exchange and quietly put
	# publishing back on a credential that expires.
	unset NODE_AUTH_TOKEN NPM_TOKEN
	echo "publishing with trusted publishing, npm ${HAVE}"
elif [ -n "${NODE_AUTH_TOKEN:-}" ]; then
	printf '//registry.npmjs.org/:_authToken=%s
' "$NODE_AUTH_TOKEN" >"${WORK_DIR}/.npmrc"
	export NPM_CONFIG_USERCONFIG="${WORK_DIR}/.npmrc"

	# Asked before five archives are downloaded and five packages assembled, so a
	# token that expired costs a second rather than the whole job.
	if ! WHO="$(npm whoami 2>&1)"; then
		echo "::error::npm will not accept this token (${WHO}). It has most likely expired: npm tokens do. Prefer trusted publishing; failing that, generate an automation token, the only kind that bypasses the account's 2FA, and update the NPM_TOKEN secret." >&2
		exit 1
	fi
	echo "authenticated to npm as ${WHO}"
else
	echo "::error::no id-token permission and no NODE_AUTH_TOKEN. Publishing anonymously is answered with a 404 that looks like a missing package." >&2
	exit 1
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
  echo "${archive} -> @ferrlabs/lfsx-${platform}"

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
    publish_if_new "@ferrlabs/lfsx-${platform}"
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

publish_if_new @ferrlabs/lfsx

echo "published @ferrlabs/lfsx@${VERSION} and its platform packages"
