FROM rust:1.98-bookworm@sha256:93ce27a88655056a51dbdd8f5f2d7ddc071c7b0070fb288a37b5a285fc83971e AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY parsers ./parsers
COPY src ./src
COPY assets ./assets
COPY bench/tls ./bench/tls
RUN cargo build --release --locked

FROM debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3
RUN useradd --system --uid 65532 --no-create-home --shell /usr/sbin/nologin glasir
COPY --from=build /src/target/release/glasir /usr/local/bin/glasir
COPY --chown=65532:65532 . /repo
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/glasir"]
