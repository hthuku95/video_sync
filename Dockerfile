# Stage 1: Build Rust application
FROM rust:1.75 as builder
WORKDIR /app

# Copy dependency manifests
COPY Cargo.toml Cargo.lock ./

# Copy source code
COPY src ./src
COPY migrations ./migrations

# Build release binary
RUN cargo build --release

# Stage 2: Runtime image with system dependencies
FROM debian:bookworm-slim

# Install system dependencies including Python and yt-dlp
# yt-dlp is most reliable for bypassing YouTube bot detection
RUN apt-get update && apt-get install -y \
    ffmpeg \
    ca-certificates \
    libpq5 \
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
