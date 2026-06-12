FROM node:20-slim AS builder

WORKDIR /app

COPY package.json package-lock.json ./
RUN npm ci --no-fund --no-audit

COPY tsconfig.json ./
COPY src ./src
COPY migrations ./migrations
RUN npm run build

FROM node:20-slim

WORKDIR /app

ENV NODE_ENV=production
ENV HUB_API_PORT=8095
ENV HUB_DB_PATH=/data/hub.db

COPY package.json package-lock.json ./
RUN npm ci --omit=dev --no-fund --no-audit

COPY --from=builder /app/dist ./dist
COPY --from=builder /app/migrations ./migrations

RUN apt-get update \
  && apt-get install -y --no-install-recommends wget \
  && rm -rf /var/lib/apt/lists/*

EXPOSE 8095

HEALTHCHECK --interval=15s --timeout=5s --retries=5 --start-period=10s \
  CMD wget -qO- http://127.0.0.1:8095/api/health || exit 1

CMD ["node", "dist/index.js"]
