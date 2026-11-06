# One image: the upstream frontend's Vite build, embedded in the Rust binary, on distroless.

FROM node:22-bookworm-slim AS frontend
WORKDIR /src/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY frontend/ ./
RUN npx vite build

FROM rust:1.97-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs ./
COPY crates ./crates
COPY config ./config
COPY src ./src
COPY --from=frontend /src/frontend/dist ./frontend/dist
RUN cargo build --release --locked

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/home-hq /usr/local/bin/home-hq
ENV HQ_LISTEN=0.0.0.0:8080
EXPOSE 8080 8443
# distroless nonroot, numeric so Kubernetes runAsNonRoot can verify it.
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/home-hq"]
