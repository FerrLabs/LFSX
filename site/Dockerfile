# syntax=docker/dockerfile:1
FROM docker.io/library/node:24-alpine@sha256:ebfe2f90462722a7a4de65e91990e97fe0d401c70e0e762c5b53302f905ec1c1 AS builder

WORKDIR /app

ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0
RUN corepack enable

COPY package.json pnpm-workspace.yaml pnpm-lock.yaml .npmrc ./
COPY site/package.json ./site/

# NODE_AUTH_TOKEN authenticates pnpm against GitHub Packages for the private
# @ferrlabs/* packages. Mounted as a BuildKit secret, never a build-arg, so it
# never lands in an image layer or the build log. pnpm will not expand it from
# the committed .npmrc, so it is written to a trusted config for the install and
# removed in the same layer.
RUN --mount=type=secret,id=packages-read \
    pnpm config set "//npm.pkg.github.com/:_authToken" "$(cat /run/secrets/packages-read)" \
    && pnpm install --frozen-lockfile --filter @lfsx/site... \
    && pnpm config delete "//npm.pkg.github.com/:_authToken"

COPY site ./site
COPY docs ./docs

ENV NODE_OPTIONS=--max-old-space-size=4096
RUN pnpm --filter @lfsx/site build

# Analog static output: prerendered HTML plus the client bundles.
FROM docker.io/nginxinc/nginx-unprivileged:alpine@sha256:6a23acdfca2b9cfbcec61419e3f1426bcbedb91362f2f19306a8567423bb4612

USER root
RUN apk upgrade --no-cache
USER nginx

COPY --from=builder /app/site/dist/analog/public /usr/share/nginx/html
COPY site/nginx.conf /etc/nginx/conf.d/default.conf

EXPOSE 8080
CMD ["nginx", "-g", "daemon off;"]
