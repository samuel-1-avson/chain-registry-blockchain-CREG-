FROM node:20-slim AS builder

WORKDIR /app/hub-web

COPY hub-web/package.json hub-web/package-lock.json ./
RUN npm ci --legacy-peer-deps --install-strategy=nested --no-fund --no-audit

COPY hub-web/ ./
COPY hub/content/ ../hub/content/

ARG VITE_WALLETCONNECT_PROJECT_ID=
ARG VITE_EXPLORER_URL=
ARG VITE_FAUCET_URL=
ARG VITE_WAITLIST_URL=
ARG VITE_SPEC_URL=
ARG VITE_DOCS_URL=
ARG VITE_API_URL=

ENV VITE_WALLETCONNECT_PROJECT_ID=$VITE_WALLETCONNECT_PROJECT_ID
ENV VITE_EXPLORER_URL=$VITE_EXPLORER_URL
ENV VITE_FAUCET_URL=$VITE_FAUCET_URL
ENV VITE_WAITLIST_URL=$VITE_WAITLIST_URL
ENV VITE_SPEC_URL=$VITE_SPEC_URL
ENV VITE_DOCS_URL=$VITE_DOCS_URL
ENV VITE_API_URL=$VITE_API_URL

RUN npm run build

FROM nginx:1.27-alpine

COPY hub-web/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=builder /app/hub-web/dist /usr/share/nginx/html

EXPOSE 80
