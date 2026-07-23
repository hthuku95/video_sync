# Runtime image — Debian Trixie (same glibc/OpenSSL as the build host)
FROM debian:trixie-slim

# Install system dependencies including Python, yt-dlp, and AWS CLI
RUN apt-get update && apt-get install -y --no-install-recommends \
    ffmpeg \
    ca-certificates \
    libpq5 \
    libssl3t64 \
    curl \
    python3 \
    python3-pip \
    && pip3 install --no-cache-dir --break-system-packages yt-dlp awscli \
    && rm -rf /var/lib/apt/lists/*

# Verify yt-dlp installation
RUN yt-dlp --version

# Copy pre-compiled binary from host
COPY target/release/video_editor /usr/local/bin/video_editor

# Copy entrypoint script
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

# Create necessary directories for video processing
RUN mkdir -p /app/outputs /app/uploads /app/downloads
WORKDIR /app

# Expose port (Render uses PORT env var)
EXPOSE 3000

# Use entrypoint to load env from S3 for Batch, then run the binary
ENTRYPOINT ["/entrypoint.sh"]
CMD ["video_editor"]
