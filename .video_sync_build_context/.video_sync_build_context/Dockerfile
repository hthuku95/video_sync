# Stage 1: Build Rust application
# rust:1.82 is Debian Bookworm (OpenSSL 3) — matches runtime image
FROM rust:1.94.1 as builder
WORKDIR /app

# Allow overriding parallelism at build time (e.g., --build-arg CARGO_JOBS=2).
# Cloud Build E2_HIGHCPU_8 can handle 2 concurrent jobs safely.
ARG CARGO_JOBS=1
ENV CARGO_BUILD_JOBS=${CARGO_JOBS}

# Install OpenSSL dev libraries (needed by openssl-sys, qdrant-client, reqwest)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy dependency manifests
COPY Cargo.toml Cargo.lock ./

# Copy source code
COPY src ./src
COPY migrations ./migrations

# Build release binary (links against Bookworm's libssl.so.3)
RUN cargo build --release -j ${CARGO_JOBS}

# Stage 2: Runtime image — Debian Trixie to match the builder's glibc/OpenSSL ABI
FROM debian:trixie-slim

# Install system dependencies including Python and yt-dlp
# yt-dlp is most reliable for bypassing YouTube bot detection
RUN apt-get update && apt-get install -y --no-install-recommends \
    ffmpeg \
    ca-certificates \
    libpq5 \
    libssl3t64 \
    curl \
    python3 \
    python3-pip \
    && pip3 install --no-cache-dir --break-system-packages yt-dlp \
    && rm -rf /var/lib/apt/lists/*

# Verify yt-dlp installation
RUN yt-dlp --version

# Copy compiled binary from builder
COPY --from=builder /app/target/release/video_editor /usr/local/bin/video_editor

# Create necessary directories for video processing
RUN mkdir -p /app/outputs /app/uploads /app/downloads
WORKDIR /app

# Expose port (Render uses PORT env var)
EXPOSE 3000

# Run the application
CMD ["video_editor"]
