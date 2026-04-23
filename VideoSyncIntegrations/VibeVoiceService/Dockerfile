FROM python:3.11-slim

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    VIBEVOICE_OUTPUT_DIR=/tmp/videosync-vibevoice

WORKDIR /app

COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY app.py videosync_vibevoice_client.py ./

EXPOSE 8015

CMD ["uvicorn", "app:app", "--host", "0.0.0.0", "--port", "8015"]
