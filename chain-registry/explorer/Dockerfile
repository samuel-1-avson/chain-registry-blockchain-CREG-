# Chain Registry Explorer - Web UI
FROM node:20-slim AS builder

WORKDIR /app

# Vite build-time env vars
ARG VITE_API_BASE=
ARG VITE_FAUCET_BASE=
ARG VITE_RPC_URL=
ARG VITE_CREG_TOKEN=
ARG VITE_STAKING_ADDR=
ARG VITE_NETWORK=testnet
ENV VITE_API_BASE=$VITE_API_BASE
ENV VITE_FAUCET_BASE=$VITE_FAUCET_BASE
ENV VITE_RPC_URL=$VITE_RPC_URL
ENV VITE_CREG_TOKEN=$VITE_CREG_TOKEN
ENV VITE_STAKING_ADDR=$VITE_STAKING_ADDR
ENV VITE_NETWORK=$VITE_NETWORK

# Copy package files
COPY package*.json ./
RUN npm ci

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
RUN cat > /etc/nginx/conf.d/default.conf << 'NGINX'
server {
    listen 80;
    server_name _;
    root /usr/share/nginx/html;
    index index.html;

    # Security headers
    add_header X-Frame-Options "SAMEORIGIN" always;
    add_header X-Content-Type-Options "nosniff" always;
    add_header X-XSS-Protection "1; mode=block" always;
    add_header Referrer-Policy "strict-origin-when-cross-origin" always;

    # Gzip
    gzip on;
    gzip_types text/plain text/css application/json application/javascript text/xml;

    # Static assets — long cache
    location ~* \.(js|css|png|jpg|jpeg|gif|ico|svg|woff2?)$ {
        expires 1y;
        add_header Cache-Control "public, immutable";
        try_files $uri =404;
    }

    # API proxy to node service
    location /v1/ {
        proxy_pass http://node-1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }

    # SSE events proxy
    location /v1/events {
        proxy_pass http://node-1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        proxy_buffering off;
        proxy_cache off;
        proxy_read_timeout 86400s;
    }

    # SPA fallback
    location / {
        try_files $uri $uri/ /index.html;
    }

    # Health check
    location = /health {
        access_log off;
        return 200 "ok";
    }
}
NGINX

EXPOSE 80

CMD ["nginx", "-g", "daemon off;"]
