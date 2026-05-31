#!/usr/bin/env python3
"""
ElevenLabs Integration Test Script
Tests all 4 ElevenLabs audio generation tools through the video editor API
"""

import asyncio
import websockets
import json
import time
import os

# Configuration
WS_URL = "ws://localhost:3000/ws"
OUTPUT_DIR = "outputs"

# Ensure output directory exists
os.makedirs(OUTPUT_DIR, exist_ok=True)

# Test cases
TEST_CASES = [
    {
        "name": "Text-to-Speech with Rachel (Default Voice)",
        "tool": "generate_text_to_speech",
        "message": "Generate speech: 'Welcome to my video editing platform. This is a test of the Eleven Labs text-to-speech integration.'",
        "expected_file": f"{OUTPUT_DIR}/test_tts_rachel.mp3"
    },
    {
        "name": "Text-to-Speech with Male Voice (Drew)",
        "tool": "generate_text_to_speech",
        "message": "Generate speech with Drew's voice: 'This is Drew speaking. Testing male voice generation with Eleven Labs.'",
        "expected_file": f"{OUTPUT_DIR}/test_tts_drew.mp3"
    },
    {
        "name": "Sound Effect Generation",
        "tool": "generate_sound_effect",
        "message": "Generate a sound effect: 'cinematic explosion with deep rumble and debris falling'",
        "expected_file": f"{OUTPUT_DIR}/test_explosion_sfx.mp3"
    },
    {
        "name": "Sound Effect - Door Creak",
        "tool": "generate_sound_effect",
        "message": "Create a sound effect of an old wooden door creaking open slowly",
        "expected_file": f"{OUTPUT_DIR}/test_door_creak.mp3"
    },
    {
        "name": "Music Generation - Upbeat Electronic",
        "tool": "generate_music",
        "message": "Generate 20 seconds of upbeat electronic dance music at 120 BPM with synth and drums",
        "expected_file": f"{OUTPUT_DIR}/test_edm_music.mp3"
    },
    {
        "name": "Music Generation - Calm Piano",
        "tool": "generate_music",
        "message": "Generate 15 seconds of peaceful piano meditation music",
        "expected_file": f"{OUTPUT_DIR}/test_piano_music.mp3"
    }
]


async def test_elevenlabs_tool(test_case):
    """Test a single ElevenLabs tool through websocket"""
    print(f"\n{'='*80}")
    print(f"🧪 TEST: {test_case['name']}")
    print(f"{'='*80}")
    print(f"📝 Message: {test_case['message']}")
    print(f"📁 Expected output: {test_case['expected_file']}")
    print(f"⏳ Connecting to {WS_URL}...")

    try:
        async with websockets.connect(WS_URL) as websocket:
            print("✅ Connected!")

            # Send the message
            print(f"📤 Sending message...")
            await websocket.send(test_case['message'])

            # Receive responses
            print("📥 Waiting for AI response...")
            start_time = time.time()

            while True:
                try:
                    response = await asyncio.wait_for(websocket.recv(), timeout=180.0)
                    elapsed = time.time() - start_time

                    # Pretty print the response
                    print(f"\n[{elapsed:.1f}s] 🤖 Agent Response:")
                    print("-" * 80)

                    # Try to parse as JSON for pretty printing
                    try:
                        response_json = json.loads(response)
                        print(json.dumps(response_json, indent=2))
                    except:
                        print(response)

                    print("-" * 80)

                    # Check if the expected file was created
                    if os.path.exists(test_case['expected_file']):
                        file_size = os.path.getsize(test_case['expected_file'])
                        print(f"\n✅ SUCCESS! File created: {test_case['expected_file']}")
                        print(f"📊 File size: {file_size:,} bytes ({file_size/1024:.2f} KB)")
                        break

                    # Check for completion indicators in response
                    if "✅" in response or "saved to" in response.lower():
                        print("\n✅ Task appears completed!")

                        # Wait a moment for file system
                        await asyncio.sleep(1)

                        if os.path.exists(test_case['expected_file']):
                            file_size = os.path.getsize(test_case['expected_file'])
                            print(f"✅ File verified: {test_case['expected_file']}")
                            print(f"📊 File size: {file_size:,} bytes ({file_size/1024:.2f} KB)")
                        else:
                            print(f"⚠️  File not found at expected location: {test_case['expected_file']}")
                        break

                    # Check for errors
                    if "❌" in response or "error" in response.lower():
                        print(f"\n❌ ERROR detected in response")
                        break

                    # Music generation takes longer
                    if elapsed > 120:
                        print(f"\n⏱️  Test timeout after {elapsed:.1f}s")
                        break

                except asyncio.TimeoutError:
                    print(f"\n⏱️  Timeout after {time.time() - start_time:.1f}s")
                    break

            print(f"\n⏱️  Total time: {time.time() - start_time:.1f}s")

    except Exception as e:
        print(f"\n❌ ERROR: {e}")
        import traceback
        traceback.print_exc()


async def test_all():
    """Run all test cases"""
    print("=" * 80)
    print("🚀 ELEVENLABS INTEGRATION TEST SUITE")
    print("=" * 80)
    print(f"Testing {len(TEST_CASES)} scenarios...")
    print(f"Output directory: {OUTPUT_DIR}/")

    results = []

    for i, test_case in enumerate(TEST_CASES, 1):
        print(f"\n\n{'#'*80}")
        print(f"# TEST {i}/{len(TEST_CASES)}")
        print(f"{'#'*80}")

        try:
            await test_elevenlabs_tool(test_case)

            # Check if file was created
            if os.path.exists(test_case['expected_file']):
                results.append((test_case['name'], "✅ PASS", test_case['expected_file']))
            else:
                results.append((test_case['name'], "❌ FAIL", "File not created"))

        except Exception as e:
            results.append((test_case['name'], "❌ ERROR", str(e)))

        # Wait between tests
        if i < len(TEST_CASES):
            print("\n⏳ Waiting 3 seconds before next test...")
            await asyncio.sleep(3)

    # Print summary
    print("\n\n" + "=" * 80)
    print("📊 TEST RESULTS SUMMARY")
    print("=" * 80)

    for name, status, detail in results:
        print(f"{status} {name}")
        if "FAIL" in status or "ERROR" in status:
            print(f"    ↳ {detail}")

    passed = sum(1 for _, status, _ in results if "PASS" in status)
    total = len(results)

    print("\n" + "=" * 80)
    print(f"📈 FINAL SCORE: {passed}/{total} tests passed ({100*passed//total}%)")
    print("=" * 80)

    if passed == total:
        print("🎉 ALL TESTS PASSED! ElevenLabs integration is working perfectly!")
    else:
        print(f"⚠️  {total - passed} test(s) failed. Review the output above for details.")

    print(f"\n📁 Check the {OUTPUT_DIR}/ directory for generated audio files")


# Quick manual test functions
async def quick_test_tts():
    """Quick test for TTS only"""
    test_case = {
        "name": "Quick TTS Test",
        "tool": "generate_text_to_speech",
        "message": "Generate speech: 'This is a quick test of text to speech.'",
        "expected_file": f"{OUTPUT_DIR}/quick_test.mp3"
    }
    await test_elevenlabs_tool(test_case)


async def quick_test_sfx():
    """Quick test for sound effects"""
    test_case = {
        "name": "Quick SFX Test",
        "tool": "generate_sound_effect",
        "message": "Generate a sound effect: 'whoosh transition sound'",
        "expected_file": f"{OUTPUT_DIR}/quick_sfx.mp3"
    }
    await test_elevenlabs_tool(test_case)


if __name__ == "__main__":
    import sys

    print("""
╔══════════════════════════════════════════════════════════════════════════════╗
║                  ELEVENLABS INTEGRATION TEST SUITE                           ║
║                                                                              ║
║  This script tests the ElevenLabs audio generation integration through      ║
║  the video editor's WebSocket API.                                          ║
║                                                                              ║
║  PREREQUISITES:                                                              ║
║  1. Start the video editor server: cargo run                                ║
║  2. Ensure ELEVEN_LABS_API_KEY is set in .env                              ║
║  3. Server should be running on http://localhost:3000                       ║
║                                                                              ║
╚══════════════════════════════════════════════════════════════════════════════╝
    """)

    if len(sys.argv) > 1:
        mode = sys.argv[1]
        if mode == "tts":
            print("🎤 Running quick TTS test only...")
            asyncio.run(quick_test_tts())
        elif mode == "sfx":
            print("🔊 Running quick SFX test only...")
            asyncio.run(quick_test_sfx())
        else:
            print(f"❌ Unknown mode: {mode}")
            print("Usage: python test_elevenlabs_integration.py [tts|sfx]")
            print("       python test_elevenlabs_integration.py  (run all tests)")
    else:
        print("🧪 Running complete test suite...")
        asyncio.run(test_all())
