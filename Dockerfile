# syntax=docker/dockerfile:1

FROM rust:trixie AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src/main.rs src/lib.rs ./src/
RUN cargo fetch --locked

COPY src ./src
RUN --mount=type=cache,id=learn-target,target=/app/target,sharing=locked \
    cargo build --locked --release && \
    cp target/release/learn /tmp/learn

# Precompressed variants are served by ServeDir::precompressed_gzip at no runtime cost.
COPY pages ./pages
RUN find pages -type f \( -name '*.js' -o -name '*.css' -o -name '*.svg' \) -exec gzip -kf9 {} +

FROM gcr.io/distroless/cc-debian13:nonroot
WORKDIR /home/nonroot

COPY --from=builder /tmp/learn ./learn
COPY migrations ./migrations
COPY --from=builder /app/pages ./pages

EXPOSE 8788
ENTRYPOINT ["./learn"]
