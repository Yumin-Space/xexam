# Nginx 镜像：构建时用 Node 编译前端，只把静态产物和 nginx.conf 放进最终镜像。
# 最终镜像里没有 Node、没有源码、没有数据库。
FROM node:24-alpine AS frontend
WORKDIR /build
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

FROM nginxinc/nginx-unprivileged:1.27-alpine
COPY nginx/nginx.conf /etc/nginx/nginx.conf
COPY --from=frontend /build/dist /usr/share/nginx/html
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD wget -q -O /dev/null http://127.0.0.1:8080/healthz || exit 1
