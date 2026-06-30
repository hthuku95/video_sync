# Google Colab Ollama GPU Server
# ===============================
# 
# Steps:
# 1. Open https://colab.research.google.com/
# 2. Runtime → Change runtime type → T4 GPU
# 3. Paste this entire cell, click Run (▶)
# 4. Wait ~2-3 min for the tunnel URL
# 5. Run the commands it prints to update your EC2

import os, subprocess, time, urllib.request, json, re, sys

print("=" * 60)
print("Step 1/4 — Installing Ollama (GPU-enabled)")
print("=" * 60)

!curl -fsSL https://ollama.com/install.sh | sh
!nvidia-smi

print("=" * 60)
print("Step 2/4 — Pulling model")
print("=" * 60)

MODEL = "gemma4:12b"  # multimodal, 7.6 GB, fits T4 16GB VRAM
!ollama pull {MODEL}

print("=" * 60)
print("Step 3/4 — Starting Ollama server")
print("=" * 60)

!pkill ollama 2>/dev/null
time.sleep(1)
!ollama serve > /tmp/ollama_server.log 2>&1 &
time.sleep(3)
!cat /tmp/ollama_server.log | tail -3

# Test
time.sleep(2)
try:
    req = urllib.request.Request(
        "http://127.0.0.1:11434/api/chat",
        data=json.dumps({"model": MODEL, "messages": [{"role": "user", "content": "hi"}],
                         "stream": False, "options": {"num_predict": 10}}).encode(),
        headers={"Content-Type": "application/json"}
    )
    resp = urllib.request.urlopen(req, timeout=30)
    print("✅ Ollama GPU working")
except Exception as e:
    print(f"⚠️  Ollama test: {e}")

print("=" * 60)
print("Step 4/4 — Creating Cloudflare tunnel")
print("=" * 60)

!wget -q https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64 \
  -O /usr/local/bin/cloudflared && chmod +x /usr/local/bin/cloudflared

# Start tunnel
proc = subprocess.Popen(
    ["cloudflared", "tunnel", "--url", "http://127.0.0.1:11434",
     "--log-file", "/tmp/cf.log", "--no-autoupdate"],
    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
)
time.sleep(10)

# Extract URL
url = None
try:
    with open("/tmp/cf.log") as f:
        m = re.search(r'https://[a-zA-Z0-9-]+\.trycloudflare\.com', f.read())
        if m: url = m.group(0)
except: pass

if url:
    print(f"\n✅ Your GPU Ollama URL:\n\n   {url}\n")
    print("Run these commands on your EC2 instance:\n")
    print(f'  ssh -i .ssh-key-v6.pem ubuntu@3.224.144.201 \\')
    print(f'    "sudo sed -i \'s|OLLAMA_BASE_URL=.*|OLLAMA_BASE_URL={url}|\' /etc/blender-mcp/env &&')
    print(f'     sudo systemctl restart blender-mcp"')
    print(f'\n⚠️  Keep this Colab tab OPEN — closing it kills the GPU.')
else:
    print("Could not detect URL. Check /tmp/cf.log")

# Keep alive
print("\nKeeping session alive...")
while True:
    time.sleep(60)
    try:
        urllib.request.urlopen(urllib.request.Request(
            "http://127.0.0.1:11434/api/chat",
            data=json.dumps({"model": MODEL, "messages": [{"role":"user","content":"ok"}],
                             "stream": False, "options": {"num_predict": 5}}).encode(),
            headers={"Content-Type": "application/json"}
        ), timeout=30)
    except: pass
    print(".", end="", flush=True)
