# Image is built exclusively by GitHub-hosted Actions, never personal servers.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p kordra-daemon -p kordra-cli

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --create-home kordra && mkdir -p /config /downloads && chown 10001:10001 /config /downloads
COPY --from=build /src/target/release/kordra-daemon /usr/local/bin/kordra-daemon
COPY --from=build /src/target/release/kordra-cli /usr/local/bin/kordra-cli
USER 10001
ENV KORDRA_LISTEN=0.0.0.0:8079 KORDRA_DATA_DIR=/config KORDRA_DOWNLOAD_DIR=/downloads
VOLUME ["/config", "/downloads"]
EXPOSE 8079
ENTRYPOINT ["kordra-daemon"]
