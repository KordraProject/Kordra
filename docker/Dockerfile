# All builds happen on GitHub-hosted Actions runners. No personal servers.
FROM node:22-bookworm-slim AS web
WORKDIR /web
COPY apps/web/package.json ./
RUN npm install --no-audit --no-fund
COPY apps/web/ ./
RUN npm run build

FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p kordra-daemon -p kordra-cli

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --create-home kordra && mkdir -p /config /downloads /usr/share/kordra/web && chown 10001:10001 /config /downloads
COPY --from=build /src/target/release/kordra-daemon /usr/local/bin/kordra-daemon
COPY --from=build /src/target/release/kordra-cli /usr/local/bin/kordra-cli
COPY --from=web /web/dist/ /usr/share/kordra/web/
USER 10001
ENV KORDRA_LISTEN=0.0.0.0:8079 KORDRA_DATA_DIR=/config KORDRA_DOWNLOAD_DIR=/downloads KORDRA_WEB_ENABLED=true KORDRA_WEB_DIR=/usr/share/kordra/web
VOLUME ["/config", "/downloads"]
EXPOSE 8079
ENTRYPOINT ["kordra-daemon"]
