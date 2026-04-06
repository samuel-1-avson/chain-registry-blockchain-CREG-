# Chain Registry Explorer - Web UI
FROM node:20-slim AS builder

WORKDIR /app

# Vite build-time env vars
ARG VITE_API_BASE=http://127.0.0.1:8080
ARG VITE_FAUCET_BASE=http://127.0.0.1:8082
ARG VITE_RPC_URL=http://127.0.0.1:8545
ARG VITE_CREG_TOKEN=
ARG VITE_STAKING_ADDR=
ENV VITE_API_BASE=$VITE_API_BASE
ENV VITE_FAUCET_BASE=$VITE_FAUCET_BASE
ENV VITE_RPC_URL=$VITE_RPC_URL
ENV VITE_CREG_TOKEN=$VITE_CREG_TOKEN
ENV VITE_STAKING_ADDR=$VITE_STAKING_ADDR

# Copy package files
COPY package*.json ./
RUN npm ci

# Copy source
COPY . .

# Build the app
RUN npm run build

# Production stage
FROM nginx:alpine

RUN apk add --no-cache wget

# Copy built assets
COPY --from=builder /app/dist /usr/share/nginx/html

# Copy nginx config
RUN echo 'server { \
    listen 80; \
    location / { \
        root /usr/share/nginx/html; \
        index index.html; \
        try_files $uri $uri/ /index.html; \
    } \
}' > /etc/nginx/conf.d/default.conf

EXPOSE 80

CMD ["nginx", "-g", "daemon off;"]
