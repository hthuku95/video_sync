FROM rust:1.82-slim
RUN apt-get update && apt-get install -y pkg-config libssl-dev python3 python3-pip ffmpeg && rm -rf /var/lib/apt/lists/*
RUN pip3 install --no-cache-dir --break-system-packages yt-dlp
WORKDIR /app
COPY . .
RUN cargo build --release --bin video_editor
EXPOSE 10000
ENV PORT=10000
ENV RUST_LOG=info
CMD ["/app/target/release/video_editor"]
