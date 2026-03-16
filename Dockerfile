# syntax=docker/dockerfile:1

# ── Stage 1: build ───────────────────────────────────────────────────────────
FROM rust:1.77-slim AS builder

WORKDIR /app

# Cache dependencies separately from source
COPY Cargo.toml ./
# Dummy src so cargo can resolve deps without full source
RUN mkdir src && echo "fn main() {}" > src/main.rs && echo "" > src/lib.rs
RUN cargo build --release --locked || true
RUN rm -rf src

# Build the real binary
COPY src ./src
RUN touch src/main.rs src/lib.rs
RUN cargo build --release --locked

# ── Stage 2: final (distroless) ──────────────────────────────────────────────
FROM gcr.io/distroless/cc-debian12:nonroot

WORKDIR /

# Copy the compiled binary
COPY --from=builder /app/target/release/llm-operator /llm-operator

# Metrics port
EXPOSE 8080
# Webhook port
EXPOSE 8443

ENTRYPOINT ["/llm-operator"]
