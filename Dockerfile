# Build stage
FROM rust:1.92 as builder

WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Copy source code
COPY src ./src

# Build for release
RUN cargo build --release --bin theme-sender --bin theme-override

# Runtime stage
FROM debian:trixie-slim

# Install runtime dependencies
RUN apt-get update && \
    apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy binaries from builder
COPY --from=builder /app/target/release/theme-sender /usr/local/bin/
COPY --from=builder /app/target/release/theme-override /usr/local/bin/

# Create non-root user
RUN useradd -m -u 1000 theme && \
    chown -R theme:theme /usr/local/bin/theme-sender /usr/local/bin/theme-override

USER theme

# Default environment variables
ENV MQTT_HOST=localhost \
    MQTT_TOPIC=neiam/sync/theme \
    MQTT_OVERRIDE_TOPIC=neiam/sync/theme/override \
    MQTT_REVERT_TOPIC=neiam/sync/theme/revert \
    PUBLISH_INTERVAL_SECS=300 \
    RUST_LOG=info

# Run theme-sender by default
CMD ["theme-sender"]
