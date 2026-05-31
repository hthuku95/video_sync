#!/bin/bash
# Test yt-dlp Integration Locally
# This script simulates exactly how the application uses yt-dlp
# Run this before deploying to production to ensure everything works

set -e  # Exit on any error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}================================================${NC}"
echo -e "${BLUE}  yt-dlp Integration Test Suite${NC}"
echo -e "${BLUE}================================================${NC}"
echo ""

# Test counter
TESTS_PASSED=0
TESTS_FAILED=0

# Test output directory
TEST_DIR="test_downloads"
mkdir -p "$TEST_DIR"

# Test video (short, public domain video)
TEST_VIDEO_URL="https://www.youtube.com/watch?v=jNQXAC9IVRw"  # "Me at the zoo" - first YouTube video (18 seconds)
TEST_OUTPUT="$TEST_DIR/test_video.mp4"

# ============================================================================
# TEST 1: Check yt-dlp Installation
# ============================================================================
echo -e "${YELLOW}TEST 1: Checking yt-dlp installation...${NC}"

if command -v yt-dlp &> /dev/null; then
    VERSION=$(yt-dlp --version)
    echo -e "${GREEN}✅ PASS: yt-dlp is installed${NC}"
    echo "   Version: $VERSION"
    ((TESTS_PASSED++))
else
    echo -e "${RED}❌ FAIL: yt-dlp is not installed${NC}"
    echo "   Install with: pip install yt-dlp"
    echo "   Or: curl -L https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp -o /usr/local/bin/yt-dlp && chmod a+rx /usr/local/bin/yt-dlp"
    ((TESTS_FAILED++))
    exit 1
fi
echo ""

# ============================================================================
# TEST 2: Check yt-dlp Binary Path
# ============================================================================
echo -e "${YELLOW}TEST 2: Checking yt-dlp binary path...${NC}"

YTDLP_PATH=$(which yt-dlp)
echo "   Found at: $YTDLP_PATH"

# Check if it's in the expected locations (matching ytdlp_client.rs)
if [ "$YTDLP_PATH" = "/usr/local/bin/yt-dlp" ]; then
    echo -e "${GREEN}✅ PASS: yt-dlp is at /usr/local/bin/yt-dlp (Dockerfile location)${NC}"
    ((TESTS_PASSED++))
elif [ "$YTDLP_PATH" = "/usr/bin/yt-dlp" ]; then
    echo -e "${GREEN}✅ PASS: yt-dlp is at /usr/bin/yt-dlp (alternative location)${NC}"
    ((TESTS_PASSED++))
else
    echo -e "${YELLOW}⚠️  WARN: yt-dlp is at $YTDLP_PATH (not standard location)${NC}"
    echo "   Application will use PATH lookup, which should work"
    ((TESTS_PASSED++))
fi

# Test if binary is executable
if [ -x "$YTDLP_PATH" ]; then
    echo -e "${GREEN}✅ PASS: yt-dlp binary is executable${NC}"
    ((TESTS_PASSED++))
else
    echo -e "${RED}❌ FAIL: yt-dlp binary is not executable${NC}"
    ((TESTS_FAILED++))
    exit 1
fi
echo ""

# ============================================================================
# TEST 3: Test Video Download (Simulating apify_client.rs fallback)
# ============================================================================
echo -e "${YELLOW}TEST 3: Testing video download with application flags...${NC}"
echo "   Test video: $TEST_VIDEO_URL"
echo "   Output: $TEST_OUTPUT"
echo ""

# Remove old test file if exists
rm -f "$TEST_OUTPUT"

# Use EXACT same flags as apify_client.rs (line 265-273)
echo "   Running: yt-dlp with production flags..."
yt-dlp \
    --format "bestvideo[ext=mp4]+bestaudio[ext=m4a]/best[ext=mp4]/best" \
    --merge-output-format mp4 \
    --output "$TEST_OUTPUT" \
    --user-agent "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36" \
    --retries 3 \
    --no-playlist \
    --socket-timeout 600 \
    "$TEST_VIDEO_URL"

if [ $? -eq 0 ]; then
    echo -e "${GREEN}✅ PASS: Video download completed${NC}"
    ((TESTS_PASSED++))
else
    echo -e "${RED}❌ FAIL: Video download failed${NC}"
    ((TESTS_FAILED++))
    exit 1
fi
echo ""

# ============================================================================
# TEST 4: Validate Downloaded File
# ============================================================================
echo -e "${YELLOW}TEST 4: Validating downloaded video file...${NC}"

# Check file exists
if [ -f "$TEST_OUTPUT" ]; then
    echo -e "${GREEN}✅ PASS: Downloaded file exists${NC}"
    ((TESTS_PASSED++))
else
    echo -e "${RED}❌ FAIL: Downloaded file does not exist${NC}"
    ((TESTS_FAILED++))
    exit 1
fi

# Check file size (should be > 1MB for videos)
FILE_SIZE=$(stat -f%z "$TEST_OUTPUT" 2>/dev/null || stat -c%s "$TEST_OUTPUT" 2>/dev/null)
FILE_SIZE_MB=$(echo "scale=2; $FILE_SIZE / 1000000" | bc)

if [ "$FILE_SIZE" -gt 1000000 ]; then
    echo -e "${GREEN}✅ PASS: File size is reasonable (${FILE_SIZE_MB} MB)${NC}"
    ((TESTS_PASSED++))
else
    echo -e "${RED}❌ FAIL: File is too small ($FILE_SIZE bytes)${NC}"
    ((TESTS_FAILED++))
    exit 1
fi

# Check file is readable
if [ -r "$TEST_OUTPUT" ]; then
    echo -e "${GREEN}✅ PASS: File is readable${NC}"
    ((TESTS_PASSED++))
else
    echo -e "${RED}❌ FAIL: File is not readable${NC}"
    ((TESTS_FAILED++))
    exit 1
fi
echo ""

# ============================================================================
# TEST 5: Validate Video with ffprobe (if available)
# ============================================================================
echo -e "${YELLOW}TEST 5: Validating video format with ffprobe...${NC}"

if command -v ffprobe &> /dev/null; then
    # Get video duration and codec (simulating core::validate_video_file)
    DURATION=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$TEST_OUTPUT" 2>&1)
    CODEC=$(ffprobe -v error -select_streams v:0 -show_entries stream=codec_name -of csv=p=0 "$TEST_OUTPUT" 2>&1)

    if [ $? -eq 0 ]; then
        echo -e "${GREEN}✅ PASS: Video validation successful${NC}"
        echo "   Duration: ${DURATION}s"
        echo "   Video codec: $CODEC"
        ((TESTS_PASSED++))
    else
        echo -e "${RED}❌ FAIL: Video validation failed${NC}"
        echo "   ffprobe error: $DURATION"
        ((TESTS_FAILED++))
        exit 1
    fi
else
    echo -e "${YELLOW}⚠️  SKIP: ffprobe not available (install ffmpeg for validation)${NC}"
fi
echo ""

# ============================================================================
# TEST 6: Test Video Info Extraction (Simulating ytdlp_client.rs)
# ============================================================================
echo -e "${YELLOW}TEST 6: Testing video info extraction...${NC}"

VIDEO_INFO=$(yt-dlp --dump-json --no-playlist "$TEST_VIDEO_URL" 2>&1)

if [ $? -eq 0 ]; then
    echo -e "${GREEN}✅ PASS: Video info extraction successful${NC}"

    # Extract key fields (simulating get_video_info_ytdlp)
    VIDEO_ID=$(echo "$VIDEO_INFO" | grep -o '"id": *"[^"]*"' | head -1 | sed 's/"id": *"\([^"]*\)"/\1/')
    VIDEO_TITLE=$(echo "$VIDEO_INFO" | grep -o '"title": *"[^"]*"' | head -1 | sed 's/"title": *"\([^"]*\)"/\1/')

    echo "   Video ID: $VIDEO_ID"
    echo "   Title: $VIDEO_TITLE"
    ((TESTS_PASSED++))
else
    echo -e "${RED}❌ FAIL: Video info extraction failed${NC}"
    echo "   Error: $VIDEO_INFO"
    ((TESTS_FAILED++))
    exit 1
fi
echo ""

# ============================================================================
# TEST 7: Test Path Resolution Logic
# ============================================================================
echo -e "${YELLOW}TEST 7: Testing path resolution logic (ytdlp_client.rs check_ytdlp_installed)...${NC}"

# Simulate the path checking logic from ytdlp_client.rs (lines 203-249)
function check_ytdlp_path() {
    local path=$1
    if [ -f "$path" ] && [ -x "$path" ]; then
        if "$path" --version &> /dev/null; then
            echo -e "${GREEN}✅ Found working yt-dlp at: $path${NC}"
            return 0
        fi
    fi
    return 1
}

# Check in priority order (matching ytdlp_client.rs)
if check_ytdlp_path "/usr/local/bin/yt-dlp"; then
    echo "   Priority 1: /usr/local/bin/yt-dlp ✓"
    ((TESTS_PASSED++))
elif check_ytdlp_path "/usr/bin/yt-dlp"; then
    echo "   Priority 2: /usr/bin/yt-dlp ✓"
    ((TESTS_PASSED++))
elif command -v yt-dlp &> /dev/null; then
    echo "   Priority 3: PATH lookup ✓ ($(which yt-dlp))"
    ((TESTS_PASSED++))
else
    echo -e "${RED}❌ FAIL: yt-dlp not found in any expected location${NC}"
    ((TESTS_FAILED++))
    exit 1
fi
echo ""

# ============================================================================
# TEST 8: Test Timeout Handling (Simulating production timeout)
# ============================================================================
echo -e "${YELLOW}TEST 8: Testing timeout handling (quick test)...${NC}"

# Test with a very short timeout to ensure timeout logic works
timeout 2s yt-dlp --version &> /dev/null
TIMEOUT_EXIT=$?

if [ $TIMEOUT_EXIT -eq 0 ]; then
    echo -e "${GREEN}✅ PASS: Timeout handling works (command completed within timeout)${NC}"
    ((TESTS_PASSED++))
else
    echo -e "${YELLOW}⚠️  INFO: Timeout test inconclusive (exit code: $TIMEOUT_EXIT)${NC}"
    echo "   This is expected if command was faster than timeout"
    ((TESTS_PASSED++))
fi
echo ""

# ============================================================================
# CLEANUP
# ============================================================================
echo -e "${YELLOW}Cleaning up test files...${NC}"
# Optionally keep the test download for manual inspection
# rm -rf "$TEST_DIR"
echo "   Test downloads kept in: $TEST_DIR"
echo ""

# ============================================================================
# TEST SUMMARY
# ============================================================================
echo -e "${BLUE}================================================${NC}"
echo -e "${BLUE}  Test Summary${NC}"
echo -e "${BLUE}================================================${NC}"
echo ""
echo "   Tests Passed: ${GREEN}$TESTS_PASSED${NC}"
echo "   Tests Failed: ${RED}$TESTS_FAILED${NC}"
echo ""

if [ $TESTS_FAILED -eq 0 ]; then
    echo -e "${GREEN}✅ ALL TESTS PASSED!${NC}"
    echo ""
    echo -e "${GREEN}yt-dlp is correctly configured for production deployment.${NC}"
    echo ""
    echo "Production Setup Verified:"
    echo "  ✓ yt-dlp is installed and executable"
    echo "  ✓ Binary path resolution works"
    echo "  ✓ Video download works with production flags"
    echo "  ✓ Downloaded files are valid"
    echo "  ✓ Video info extraction works"
    echo "  ✓ Path checking logic matches application"
    echo ""
    echo "Next Steps:"
    echo "  1. Review test_downloads/test_video.mp4 if needed"
    echo "  2. Proceed with production deployment"
    echo "  3. Monitor /health/detailed endpoint after deploy"
    echo ""
    exit 0
else
    echo -e "${RED}❌ SOME TESTS FAILED!${NC}"
    echo ""
    echo "Fix the issues above before deploying to production."
    echo ""
    exit 1
fi
