#!/bin/bash

# manual_test.sh - Complete manual testing guide for all 37 features
# Run this script to test every single feature of the video editor

echo "🎬 COMPLETE VIDEO EDITOR TESTING SUITE"
echo "Testing all 37 functions with real examples"
echo

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
PURPLE='\033[0;35m'
NC='\033[0m'

# Create test directories
mkdir -p test_videos test_outputs test_assets frames_output

# Function to create test video if it doesn't exist
create_test_video() {
    if [ ! -f "test_videos/sample.mp4" ]; then
        echo -e "${BLUE}Creating test video...${NC}"
        # Create a 10-second test video with audio
        gst-launch-1.0 -e \
            videotestsrc pattern=0 num-buffers=300 ! \
            video/x-raw,width=640,height=480,framerate=30/1 ! \
            x264enc ! mux.video_0 \
            audiotestsrc freq=440 num-buffers=441 ! \
            audio/x-raw,rate=44100 ! \
            avenc_aac ! mux.audio_0 \
            mp4mux name=mux ! \
            filesink location=test_videos/sample.mp4 &> /dev/null
        
        if [ $? -eq 0 ]; then
            echo -e "${GREEN}✓${NC} Test video created: test_videos/sample.mp4"
        else
            echo -e "${RED}✗${NC} Failed to create test video. Install GStreamer tools."
            return 1
        fi
    fi

    # Create second test video for advanced features
    if [ ! -f "test_videos/overlay.mp4" ]; then
        echo -e "${BLUE}Creating overlay test video...${NC}"
        gst-launch-1.0 -e \
            videotestsrc pattern=1 num-buffers=150 ! \
            video/x-raw,width=320,height=240,framerate=30/1 ! \
            x264enc ! \
            mp4mux ! \
            filesink location=test_videos/overlay.mp4 &> /dev/null
    fi

    # Create test audio file
    if [ ! -f "test_videos/background.mp3" ]; then
        echo -e "${BLUE}Creating test audio file...${NC}"
        gst-launch-1.0 -e \
            audiotestsrc freq=220 num-buffers=441 ! \
            audio/x-raw,rate=44100 ! \
            lamemp3enc ! \
            filesink location=test_videos/background.mp3 &> /dev/null
    fi

    # Create test image for overlay
    if [ ! -f "test_assets/logo.png" ]; then
        echo -e "${BLUE}Creating test overlay image...${NC}"
        gst-launch-1.0 -e \
            videotestsrc pattern=2 num-buffers=1 ! \
            video/x-raw,width=100,height=100 ! \
            pngenc ! \
            filesink location=test_assets/logo.png &> /dev/null
    fi
}

# Function to run a test and show result
run_test() {
    local test_name="$1"
    local command="$2"
    
    echo -e "${PURPLE}Testing:${NC} $test_name"
    echo -e "${YELLOW}Command:${NC} $command"
    
    eval $command
    local exit_code=$?
    
    if [ $exit_code -eq 0 ]; then
        echo -e "${GREEN}✓ SUCCESS${NC}: $test_name"
    else
        echo -e "${RED}✗ FAILED${NC}: $test_name"
    fi
    echo
}

# Create test files
create_test_video

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}TESTING ALL 37 VIDEO EDITOR FEATURES${NC}"
echo -e "${BLUE}========================================${NC}"
echo

# ===============================
# CORE OPERATIONS (7 functions)
# ===============================
echo -e "${BLUE}=== CORE OPERATIONS (7 functions) ===${NC}"

run_test "1. Video Analysis" \
    "./target/debug/video_editor analyze test_videos/sample.mp4"

run_test "2. Video Trimming" \
    "./target/debug/video_editor trim test_videos/sample.mp4 test_outputs/trimmed.mp4 2.0 6.0"

run_test "3. Segment Extraction" \
    "./target/debug/video_editor extract test_videos/sample.mp4 test_outputs/extracted.mp4 1.0 4.0"

run_test "4. Video Merging" \
    "./target/debug/video_editor merge test_outputs/merged.mp4 test_outputs/trimmed.mp4 test_outputs/extracted.mp4"

run_test "5. Video Splitting" \
    "./target/debug/video_editor split test_videos/sample.mp4 test_outputs/segment 3.0"

run_test "6. Duration Check" \
    "./target/debug/video_editor duration test_videos/sample.mp4"

run_test "7. File Validation" \
    "./target/debug/video_editor validate test_videos/sample.mp4"

# ===============================
# AUDIO OPERATIONS (5 functions)
# ===============================
echo -e "${BLUE}=== AUDIO OPERATIONS (5 functions) ===${NC}"

run_test "8. Extract Audio to MP3" \
    "./target/debug/video_editor extract-audio test_videos/sample.mp4 test_outputs/audio.mp3 mp3"

run_test "9. Add Background Audio" \
    "./target/debug/video_editor add-audio test_outputs/trimmed.mp4 test_videos/background.mp3 test_outputs/with_audio.mp4"

run_test "10. Volume Adjustment" \
    "./target/debug/video_editor volume test_videos/sample.mp4 test_outputs/loud.mp4 2.0"

run_test "11. Audio Fade Effects" \
    "./target/debug/video_editor fade test_videos/sample.mp4 test_outputs/faded.mp4 1.0 1.0"

run_test "12. Audio Echo Effect" \
    "./target/debug/video_editor audio-effect test_videos/sample.mp4 test_outputs/echo.mp4 echo 0.5"

# ===============================
# VISUAL OPERATIONS (8 functions)
# ===============================
echo -e "${BLUE}=== VISUAL OPERATIONS (8 functions) ===${NC}"

run_test "13. Sepia Filter" \
    "./target/debug/video_editor filter test_videos/sample.mp4 test_outputs/sepia.mp4 sepia 0.8"

run_test "14. Color Adjustment" \
    "./target/debug/video_editor color test_videos/sample.mp4 test_outputs/bright.mp4 0.3 0.2 -0.1"

run_test "15. Image Overlay" \
    "./target/debug/video_editor overlay test_videos/sample.mp4 test_assets/logo.png test_outputs/overlaid.mp4 50 50 0.8"

run_test "16. Text Overlay" \
    "./target/debug/video_editor text test_videos/sample.mp4 test_outputs/texted.mp4 \"Hello World!\" 100 50 32 \"#FF0000\" 2.0 8.0"

run_test "17. Animated Text" \
    "./target/debug/video_editor animated-text test_videos/sample.mp4 test_outputs/animated.mp4 \"Welcome!\" fade_in 1.0 2.0"

run_test "18. Filter Chain" \
    "./target/debug/video_editor filter-chain test_videos/sample.mp4 test_outputs/chain.mp4 blur:0.3 sepia:0.6"

run_test "19. Video Transition" \
    "./target/debug/video_editor transition test_outputs/trimmed.mp4 test_outputs/extracted.mp4 test_outputs/transition.mp4 fade 1.0"

echo -e "${YELLOW}Note: Subtitle test requires SRT file - creating one...${NC}"
echo "1
00:00:01,000 --> 00:00:05,000
Hello, this is a test subtitle!" > test_assets/subtitles.srt

run_test "20. Subtitle Addition" \
    "./target/debug/video_editor subtitles test_videos/sample.mp4 test_assets/subtitles.srt test_outputs/subtitled.mp4 24 \"#FFFFFF\""

# ===============================
# TRANSFORMATION OPERATIONS (9 functions)
# ===============================
echo -e "${BLUE}=== TRANSFORMATION OPERATIONS (9 functions) ===${NC}"

run_test "21. Video Resize" \
    "./target/debug/video_editor resize test_videos/sample.mp4 test_outputs/resized.mp4 1280 720 true"

run_test "22. Video Crop" \
    "./target/debug/video_editor crop test_videos/sample.mp4 test_outputs/cropped.mp4 100 100 400 300"

run_test "23. Video Rotation" \
    "./target/debug/video_editor rotate test_videos/sample.mp4 test_outputs/rotated.mp4 90"

run_test "24. Speed Adjustment" \
    "./target/debug/video_editor speed test_videos/sample.mp4 test_outputs/fast.mp4 2.0"

run_test "25. Video Flip" \
    "./target/debug/video_editor flip test_videos/sample.mp4 test_outputs/flipped.mp4 horizontal"

run_test "26. Video Scaling" \
    "./target/debug/video_editor scale test_videos/sample.mp4 test_outputs/scaled.mp4 0.5 bicubic"

run_test "27. Video Stabilization" \
    "./target/debug/video_editor stabilize test_videos/sample.mp4 test_outputs/stabilized.mp4 0.7"

run_test "28. Thumbnail Creation" \
    "./target/debug/video_editor thumbnail test_videos/sample.mp4 test_outputs/thumb.jpg 3.0 320 240"

run_test "29. Deinterlacing" \
    "./target/debug/video_editor deinterlace test_videos/sample.mp4 test_outputs/deinterlaced.mp4 linear"

# ===============================
# ADVANCED FEATURES (3 functions)
# ===============================
echo -e "${BLUE}=== ADVANCED FEATURES (3 functions) ===${NC}"

run_test "30. Picture-in-Picture" \
    "./target/debug/video_editor picture-in-picture test_videos/sample.mp4 test_videos/overlay.mp4 test_outputs/pip.mp4 top-right 0.3"

run_test "31. Split Screen" \
    "./target/debug/video_editor split-screen test_videos/sample.mp4 test_videos/overlay.mp4 test_outputs/splitscreen.mp4 horizontal"

# Create green screen test
echo -e "${YELLOW}Creating green background for chroma key test...${NC}"
gst-launch-1.0 -e \
    videotestsrc pattern=4 num-buffers=150 ! \
    video/x-raw,width=640,height=480,framerate=30/1 ! \
    x264enc ! mp4mux ! \
    filesink location=test_videos/green_bg.mp4 &> /dev/null

run_test "32. Chroma Key (Green Screen)" \
    "./target/debug/video_editor chroma-key test_videos/sample.mp4 test_videos/green_bg.mp4 test_outputs/chromakey.mp4 \"#00FF00\" 0.3"

# ===============================
# EXPORT OPTIONS (5 functions)
# ===============================
echo -e "${BLUE}=== EXPORT OPTIONS (5 functions) ===${NC}"

run_test "33. Format Conversion" \
    "./target/debug/video_editor convert test_videos/sample.mp4 test_outputs/converted.avi avi"

run_test "34. Custom Quality Export" \
    "./target/debug/video_editor export-quality test_videos/sample.mp4 test_outputs/hq.mp4 high"

run_test "35. Platform Export (YouTube)" \
    "./target/debug/video_editor export-platform test_videos/sample.mp4 test_outputs/youtube.mp4 youtube"

run_test "36. Video Compression" \
    "./target/debug/video_editor compress test_videos/sample.mp4 test_outputs/compressed.mp4 medium"

run_test "37. Frame Extraction" \
    "./target/debug/video_editor extract-frames test_videos/sample.mp4 frames_output 2.0 png"

# ===============================
# TESTING SUMMARY
# ===============================
echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}TESTING COMPLETE!${NC}"
echo -e "${BLUE}========================================${NC}"
echo

echo -e "${GREEN}✅ All 37 features have been tested!${NC}"
echo
echo -e "${BLUE}Generated Files:${NC}"
echo "📁 test_outputs/ - Contains all processed videos"
echo "📁 frames_output/ - Contains extracted frames"
echo "📁 test_assets/ - Contains test assets (images, subtitles)"
echo
echo -e "${BLUE}Check the output files to verify quality:${NC}"
echo "🎬 ls -la test_outputs/"
echo "🖼️  ls -la frames_output/"
echo
echo -e "${YELLOW}Tips for verification:${NC}"
echo "• Play videos with: vlc test_outputs/filename.mp4"
echo "• Check file sizes: du -h test_outputs/*"
echo "• Verify JSON output was displayed for each operation"
echo
echo -e "${GREEN}🎉 Ready for AI Agent integration!${NC}"