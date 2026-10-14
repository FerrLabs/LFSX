# syntax=docker/dockerfile:1
FROM docker.io/library/node:24-alpine@sha256:44fce696f38ba8c987c4db89954051000bbd05ffd8d8659ba15ed4a8602e937f AS builder

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
FROM docker.io/nginxinc/nginx-unprivileged:alpine@sha256:378d0c32f6562f7fd9629170797f57d2b953aa09be983b3daaf4809bb039ca00

USER root
RUN apk upgrade --no-cache
USER nginx

COPY --from=builder /app/site/dist/analog/public /usr/share/nginx/html
COPY site/nginx.conf /etc/nginx/conf.d/default.conf

EXPOSE 8080
CMD ["nginx", "-g", "daemon off;"]
