FROM rust:1.99-bookworm@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY parsers ./parsers
COPY src ./src
COPY assets ./assets
COPY bench/tls ./bench/tls
RUN cargo build --release --locked

FROM debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc
RUN useradd --system --uid 65532 --no-create-home --shell /usr/sbin/nologin glasir
COPY --from=build /src/target/release/glasir /usr/local/bin/glasir
COPY --chown=65532:65532 . /repo
USER 65532:65532
LABEL io.modelcontextprotocol.server.name="io.github.Attackwave/glasir"
ENTRYPOINT ["/usr/local/bin/glasir"]
