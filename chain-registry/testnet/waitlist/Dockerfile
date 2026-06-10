# Static waitlist SPA (Vite dist/). Build on workstation before deploy:
#   .\testnet\gcp\deploy-waitlist.ps1 -BuildOnly
FROM nginx:1.27-alpine
COPY dist/ /usr/share/nginx/html/
COPY nginx.conf /etc/nginx/conf.d/default.conf
EXPOSE 80
