# Chain Registry Explorer - Web UI
FROM node:20-slim AS builder

WORKDIR /app

# Vite build-time env vars
ARG VITE_API_BASE=
ARG VITE_RPC_URL=
ARG VITE_CREG_TOKEN=
ARG VITE_STAKING_ADDR=
ARG VITE_NETWORK=testnet
ENV VITE_API_BASE=$VITE_API_BASE
ENV VITE_RPC_URL=$VITE_RPC_URL
ENV VITE_CREG_TOKEN=$VITE_CREG_TOKEN
ENV VITE_STAKING_ADDR=$VITE_STAKING_ADDR
ENV VITE_NETWORK=$VITE_NETWORK

# Copy package files
COPY package*.json ./
RUN if [ -f package-lock.json ]; then npm ci; else npm install --no-fund --no-audit; fi

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
