# KiteTerm 管理平台 —— 多阶段构建,产出静态 musl 二进制,运行镜像为 scratch。
# 依赖全部走 rustls(自带 webpki 根证书),运行时不需要系统 CA 与 glibc。
#   docker build -t kite-server:latest server/
#   docker run -d --name kite-server --env-file kite-server.env -p 8787:8787 -v kite-data:/data kite-server:latest
FROM rust:1-alpine AS build
RUN apk add --no-cache build-base musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked \
 && strip target/release/kite-server

FROM scratch
COPY --from=build /src/target/release/kite-server /kite-server
# 数据目录:SQLite 库放这里(KITE_DB 指向 /data/kite-server.db)
VOLUME ["/data"]
ENV KITE_LISTEN=0.0.0.0:8787 KITE_DB=/data/kite-server.db
EXPOSE 8787
ENTRYPOINT ["/kite-server"]
