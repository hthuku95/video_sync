#!/usr/bin/env python3
import asyncio
import websockets
import json
import uuid

async def test_multi_step_request():
    """Test the exact multi-step scenario that was failing"""

    # Generate a test session ID
    session_id = str(uuid.uuid4())

    # WebSocket URL
    uri = f"ws://localhost:3000/ws?session={session_id}"

    try:
        # Connect to WebSocket
        print(f"Connecting to {uri}")
        async with websockets.connect(uri) as websocket:
            print("✅ Connected to WebSocket")

            # Send the exact multi-step request that was failing
            message = {
                "type": "message",
                "content": "Please analyze the video at test_videos/sample.mp4 and then make it black and white and also add an overlay text at the center saying 'I love playing chess'",
                "session_id": session_id
            }

            print(f"📤 Sending message: {message['content']}")
            await websocket.send(json.dumps(message))

            # Listen for responses
            response_count = 0
            while True:
                try:
                    response = await asyncio.wait_for(websocket.recv(), timeout=60.0)
                    response_count += 1

                    data = json.loads(response)
                    print(f"\n📥 Response {response_count}:")
                    print(f"Type: {data.get('type', 'unknown')}")

                    if data.get('type') == 'message':
                        print(f"Content: {data.get('content', '')}")

                        # Check if this indicates completion
                        content = data.get('content', '').lower()
                        if 'completed' in content or 'finished' in content or 'done' in content:
                            print("✅ Process appears to be complete")
                            break

                    elif data.get('type') == 'error':
                        print(f"❌ Error: {data.get('content', data.get('error', 'Unknown error'))}")
                        break

                    elif data.get('type') == 'progress':
                        print(f"🔄 Progress: {data.get('content', '')}")

                    # Stop after reasonable number of responses to avoid infinite loop
                    if response_count > 20:
                        print("⚠️ Stopping after 20 responses to avoid infinite loop")
                        break

                except asyncio.TimeoutError:
                    print("⏱️ Timeout waiting for response")
                    break
                except websockets.exceptions.ConnectionClosed:
                    print("🔌 WebSocket connection closed")
                    break

    except Exception as e:
        print(f"❌ Connection error: {e}")

if __name__ == "__main__":
    print("🎬 Testing Multi-Step Video Editing Request")
    print("=" * 50)
    asyncio.run(test_multi_step_request())