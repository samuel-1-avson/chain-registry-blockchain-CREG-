# Chain Registry Explorer - Web UI
FROM node:20-slim AS builder

WORKDIR /app

# Vite build-time env vars
ARG VITE_API_BASE=
ARG VITE_RPC_URL=
ARG VITE_CREG_TOKEN=
ARG VITE_STAKING_ADDR=
ARG VITE_SEPOLIA_CREG_TOKEN=
ARG VITE_SEPOLIA_STAKING_ADDR=
ARG VITE_SEPOLIA_REGISTRY_ADDR=
ARG VITE_DEFAULT_NETWORK_PROFILE=sepolia
ARG VITE_NETWORK=testnet
ARG VITE_NODE_ROUTE_MODE=auto
ARG VITE_OPERATOR_API_KEY=

ENV VITE_API_BASE=$VITE_API_BASE
ENV VITE_RPC_URL=$VITE_RPC_URL
ENV VITE_CREG_TOKEN=$VITE_CREG_TOKEN
ENV VITE_STAKING_ADDR=$VITE_STAKING_ADDR
ENV VITE_SEPOLIA_CREG_TOKEN=$VITE_SEPOLIA_CREG_TOKEN
ENV VITE_SEPOLIA_STAKING_ADDR=$VITE_SEPOLIA_STAKING_ADDR
ENV VITE_SEPOLIA_REGISTRY_ADDR=$VITE_SEPOLIA_REGISTRY_ADDR
ENV VITE_DEFAULT_NETWORK_PROFILE=$VITE_DEFAULT_NETWORK_PROFILE
ENV VITE_NETWORK=$VITE_NETWORK
ENV VITE_NODE_ROUTE_MODE=$VITE_NODE_ROUTE_MODE
ENV VITE_OPERATOR_API_KEY=$VITE_OPERATOR_API_KEY

# Copy package files + .npmrc (legacy-peer-deps config)
COPY package*.json .npmrc* ./
RUN if [ -f package-lock.json ]; then npm ci --no-fund --no-audit; else npm install --no-fund --no-audit; fi

# Copy source
COPY . .

# Build the app
RUN npm run build

# Production stage
FROM nginx:alpine

RUN apk add --no-cache wget curl

# Copy built assets
COPY --from=builder /app/dist /usr/share/nginx/html

# Copy nginx config with API proxy, security headers, and SPA fallback
COPY nginx.conf /etc/nginx/conf.d/default.conf

EXPOSE 80

CMD ["nginx", "-g", "daemon off;"]
