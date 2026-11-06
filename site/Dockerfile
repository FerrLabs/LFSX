# syntax=docker/dockerfile:1
FROM docker.io/library/node:26-alpine@sha256:8996bb5821d62a8fd9bb27c1cb07462f3fdce6e47d640c6a881ead0b064ce216 AS builder

WORKDIR /app

ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0
RUN corepack enable

COPY package.json pnpm-workspace.yaml pnpm-lock.yaml .npmrc ./
COPY site/package.json ./site/

RUN pnpm install --frozen-lockfile --filter @lfsx/site...

COPY site ./site
COPY docs ./docs

ENV NODE_OPTIONS=--max-old-space-size=4096
RUN pnpm --filter @lfsx/site build

# Analog static output: prerendered HTML plus the client bundles.
FROM docker.io/nginxinc/nginx-unprivileged:alpine@sha256:b5047b9b76845eac124358373c6852377c52e4b05834a47647e955dd91cb8f1e

USER root
RUN apk upgrade --no-cache
USER nginx

COPY --from=builder /app/site/dist/analog/public /usr/share/nginx/html
COPY site/nginx.conf /etc/nginx/conf.d/default.conf

EXPOSE 8080
CMD ["nginx", "-g", "daemon off;"]
