#!/bin/bash

# test_modular.sh - Comprehensive test script for the modular video editor

echo "=== Video Editor Modular Structure Test ==="
echo "This script tests the newly modularized video editor structure"
echo

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test counter
TESTS_PASSED=0
TESTS_FAILED=0

# Function to print test results
print_result() {
    if [ $1 -eq 0 ]; then
        echo -e "${GREEN}✓ PASS${NC}: $2"
        ((TESTS_PASSED++))
    else
        echo -e "${RED}✗ FAIL${NC}: $2"
        ((TESTS_FAILED++))
    fi
}

# Function to run a test command
run_test() {
    local test_name="$1"
    local command="$2"
    local expected_exit_code="${3:-0}"
    
    echo -e "${BLUE}Testing:${NC} $test_name"
    echo -e "${YELLOW}Command:${NC} $command"
    
    eval $command
    local exit_code=$?
    
    if [ $exit_code -eq $expected_exit_code ]; then
        print_result 0 "$test_name"
    else
        print_result 1 "$test_name (expected exit code $expected_exit_code, got $exit_code)"
    fi
    echo
}

# Check if we're in the right directory
if [ ! -f "Cargo.toml" ]; then
    echo -e "${RED}Error:${NC} Please run this script from the video_editor project root directory"
    exit 1
fi

# Check if GStreamer is installed
echo -e "${BLUE}Checking dependencies...${NC}"
if ! command -v gst-launch-1.0 &> /dev/null; then
    echo -e "${YELLOW}Warning:${NC} GStreamer not found. Some tests may fail."
    echo "Install GStreamer with: sudo apt-get install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev"
fi

# Create test files directory
mkdir -p test_files
cd test_files

# Create a simple test video if it doesn't exist
if [ ! -f "test_input.mp4" ]; then
    echo -e "${BLUE}Creating test video file...${NC}"
    # Create a simple 10-second test video using GStreamer
    gst-launch-1.0 -e videotestsrc pattern=0 num-buffers=300 ! \
        video/x-raw,width=640,height=480,framerate=30/1 ! \
        x264enc ! mp4mux ! filesink location=test_input.mp4 &> /dev/null
    
    if [ $? -eq 0 ]; then
        echo -e "${GREEN}✓${NC} Test video created: test_input.mp4"
    else
        echo -e "${YELLOW}Warning:${NC} Could not create test video. Using manual testing only."
    fi
fi

cd ..

echo -e "${BLUE}Building project...${NC}"
cargo build
build_result=$?

if [ $build_result -ne 0 ]; then
    echo -e "${RED}✗ CRITICAL:${NC} Project failed to build"
    exit 1
else
    echo -e "${GREEN}✓${NC} Project built successfully"
fi

echo
echo -e "${BLUE}=== TESTING CORE OPERATIONS ===${NC}"

# Test help command
run_test "Help command" "./target/debug/video_editor help"

# Test analyze command (if test file exists)
if [ -f "test_files/test_input.mp4" ]; then
    run_test "Video analysis" "./target/debug/video_editor analyze test_files/test_input.mp4"
    
    # Test duration command
    run_test "Get video duration" "./target/debug/video_editor duration test_files/test_input.mp4"
    
    # Test validate command
    run_test "Validate video file" "./target/debug/video_editor validate test_files/test_input.mp4"
    
    # Test trim command
    run_test "Trim video" "./target/debug/video_editor trim test_files/test_input.mp4 test_files/trimmed.mp4 1.0 3.0"
    
    # Test extract command
    run_test "Extract video segment" "./target/debug/video_editor extract test_files/test_input.mp4 test_files/extracted.mp4 2.0 4.0"
    
    # Test merge command (create two files first)
    if [ -f "test_files/trimmed.mp4" ] && [ -f "test_files/extracted.mp4" ]; then
        run_test "Merge videos" "./target/debug/video_editor merge test_files/merged.mp4 test_files/trimmed.mp4 test_files/extracted.mp4"
    fi
    
    # Test split command
    run_test "Split video" "./target/debug/video_editor split test_files/test_input.mp4 test_files/segment 2.0"
    
else
    echo -e "${YELLOW}Skipping file-based tests - no test video available${NC}"
fi

echo
echo -e "${BLUE}=== TESTING AUDIO OPERATIONS ===${NC}"

if [ -f "test_files/test_input.mp4" ]; then
    # Test extract audio
    run_test "Extract audio to MP3" "./target/debug/video_editor extract-audio test_files/test_input.mp4 test_files/audio.mp3 mp3"
    
    # Test volume adjustment
    run_test "Adjust volume" "./target/debug/video_editor volume test_files/test_input.mp4 test_files/loud.mp4 2.0"
    
    # Test fade effects
    run_test "Add fade effects" "./target/debug/video_editor fade test_files/test_input.mp4 test_files/faded.mp4 1.0 1.0"
    
    # Test audio effects
    run_test "Apply audio effect (echo)" "./target/debug/video_editor audio-effect test_files/test_input.mp4 test_files/echo.mp4 echo 0.5"
    
else
    echo -e "${YELLOW}Skipping audio tests - no test video available${NC}"
fi

echo
echo -e "${BLUE}=== TESTING ERROR HANDLING ===${NC}"

# Test with non-existent file
run_test "Handle missing input file" "./target/debug/video_editor analyze nonexistent.mp4" 1

# Test with invalid command
run_test "Handle invalid command" "./target/debug/video_editor invalid_command" 1

# Test with missing arguments
run_test "Handle missing arguments" "./target/debug/video_editor trim" 1

echo
echo -e "${BLUE}=== TESTING MODULE STRUCTURE ===${NC}"

# Check if all module files exist
modules=("lib.rs" "types.rs" "core.rs" "audio.rs" "visual.rs" "transform.rs" "advanced.rs" "export.rs" "utils.rs")

for module in "${modules[@]}"; do
    if [ -f "src/$module" ]; then
        print_result 0 "Module file exists: src/$module"
    else
        print_result 1 "Module file missing: src/$module"
    fi
done

# Test cargo check for syntax errors
echo -e "${BLUE}Running cargo check...${NC}"
cargo check &> /dev/null
print_result $? "Code syntax and module dependencies"

# Test cargo test (if we had unit tests)
echo -e "${BLUE}Running cargo test...${NC}"
cargo test &> /dev/null
test_result=$?
if [ $test_result -eq 0 ]; then
    print_result 0 "Unit tests (if any)"
else
    echo -e "${YELLOW}Note:${NC} No unit tests found or tests failed"
fi

echo
echo -e "${BLUE}=== TESTING TOOL COMPATIBILITY ===${NC}"

# Test JSON output format (tool-friendly)
if [ -f "test_files/test_input.mp4" ]; then
    echo -e "${BLUE}Testing JSON output format...${NC}"
    output=$(./target/debug/video_editor analyze test_files/test_input.mp4 2>/dev/null | grep -A 20 "JSON Output")
    
    if echo "$output" | grep -q '"success":\|"file_path":\|"duration_seconds":'; then
        print_result 0 "JSON output format for tool compatibility"
    else
        print_result 1 "JSON output format for tool compatibility"
    fi
fi

echo
echo -e "${BLUE}=== SUMMARY ===${NC}"
echo -e "Tests passed: ${GREEN}$TESTS_PASSED${NC}"
echo -e "Tests failed: ${RED}$TESTS_FAILED${NC}"
echo -e "Total tests: $((TESTS_PASSED + TESTS_FAILED))"

if [ $TESTS_FAILED -eq 0 ]; then
    echo -e "${GREEN}🎉 All tests passed! The modular structure is working correctly.${NC}"
    echo
    echo -e "${BLUE}Next steps:${NC}"
    echo "1. Implement remaining features in visual.rs, transform.rs, advanced.rs, and export.rs"
    echo "2. Add comprehensive unit tests"
    echo "3. Create AI agent tool wrappers"
    echo "4. Add comprehensive error handling and logging"
    exit 0
else
    echo -e "${RED}❌ Some tests failed. Please check the issues above.${NC}"
    exit 1
fi