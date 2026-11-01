FROM ghcr.io/htostudiosbusiness-beep/video_editor:latest
EXPOSE 10000
ENV PORT=10000
ENV RUST_LOG=info
CMD ["/app/target/release/video_editor"]
