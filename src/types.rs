// types.rs - Common data structures for all modules
use serde::{Deserialize, Serialize};

// Base result type for all operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationResult {
    pub success: bool,
    pub output_file: String,
    pub operation: String,
    pub duration_seconds: f64,
    pub message: String,
    pub error: Option<String>,
}

// Video metadata structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoMetadata {
    pub file_path: String,
    pub duration_seconds: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_audio: bool,
    pub has_video: bool,
    pub format: String,
    pub file_size_mb: f64,
}

// Core operation parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrimParameters {
    pub input_file: String,
    pub output_file: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractParameters {
    pub input_file: String,
    pub output_file: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeParameters {
    pub input_files: Vec<String>,
    pub output_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitParameters {
    pub input_file: String,
    pub output_prefix: String,
    pub segment_duration: f64,
}

// Audio operation parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractAudioParameters {
    pub input_file: String,
    pub output_file: String,
    pub format: String, // mp3, wav, aac, etc.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddAudioParameters {
    pub video_file: String,
    pub audio_file: String,
    pub output_file: String,
    pub replace_existing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeParameters {
    pub input_file: String,
    pub output_file: String,
    pub volume_level: f64, // 1.0 = normal, 0.5 = half, 2.0 = double
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FadeParameters {
    pub input_file: String,
    pub output_file: String,
    pub fade_in_duration: f64,
    pub fade_out_duration: f64,
}

// Visual enhancement parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterParameters {
    pub input_file: String,
    pub output_file: String,
    pub filter_type: String, // "sepia", "grayscale", "blur", etc.
    pub intensity: f64, // 0.0 to 1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorAdjustParameters {
    pub input_file: String,
    pub output_file: String,
    pub brightness: f64, // -1.0 to 1.0
    pub contrast: f64,   // -1.0 to 1.0  
    pub saturation: f64, // -1.0 to 1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayParameters {
    pub input_file: String,
    pub overlay_file: String,
    pub output_file: String,
    pub x_position: u32,
    pub y_position: u32,
    pub opacity: f64, // 0.0 to 1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtitleParameters {
    pub input_file: String,
    pub subtitle_file: String,
    pub output_file: String,
    pub font_size: u32,
    pub font_color: String, // hex color like "#FFFFFF"
}

// Transformation parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResizeParameters {
    pub input_file: String,
    pub output_file: String,
    pub width: u32,
    pub height: u32,
    pub maintain_aspect_ratio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CropParameters {
    pub input_file: String,
    pub output_file: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotateParameters {
    pub input_file: String,
    pub output_file: String,
    pub angle: f64, // degrees: 90, 180, 270, or any custom angle
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeedParameters {
    pub input_file: String,
    pub output_file: String,
    pub speed_factor: f64, // 0.5 = half speed, 2.0 = double speed
}

// Additional transformation parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleParameters {
    pub input_file: String,
    pub output_file: String,
    pub scale_factor: f64,
    pub algorithm: String, // "bilinear", "bicubic", "lanczos", "nearest"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlipParameters {
    pub input_file: String,
    pub output_file: String,
    pub flip_type: String, // "horizontal", "vertical", "both"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StabilizeParameters {
    pub input_file: String,
    pub output_file: String,
    pub strength: f64, // 0.0 to 1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThumbnailParameters {
    pub input_file: String,
    pub output_file: String,
    pub timestamp: f64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeinterlaceParameters {
    pub input_file: String,
    pub output_file: String,
    pub method: String, // "linear", "yadif", "greedy", "blend"
}

// Export parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportParameters {
    pub input_file: String,
    pub output_file: String,
    pub format: String, // mp4, avi, mov, webm, etc.
    pub quality: String, // low, medium, high, custom
    pub bitrate: Option<u32>, // kbps
    pub resolution: Option<(u32, u32)>, // (width, height)
}

// Advanced feature parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PictureInPictureParameters {
    pub main_video: String,
    pub overlay_video: String,
    pub output_file: String,
    pub position: String, // "top-left", "top-right", "bottom-left", "bottom-right"
    pub size_ratio: f64, // 0.1 to 0.5 (10% to 50% of main video size)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChromaKeyParameters {
    pub input_file: String,
    pub background_file: String,
    pub output_file: String,
    pub key_color: String, // hex color like "#00FF00" for green
    pub threshold: f64, // 0.0 to 1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitScreenParameters {
    pub video1: String,
    pub video2: String,
    pub output_file: String,
    pub orientation: String, // "horizontal" or "vertical"
}

// Additional visual enhancement parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextOverlayParameters {
    pub input_file: String,
    pub output_file: String,
    pub text: String,
    pub x_position: u32,
    pub y_position: u32,
    pub font_size: u32,
    pub font_color: String, // hex color like "#FFFFFF"
    pub start_time: f64,
    pub end_time: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnimatedTextParameters {
    pub input_file: String,
    pub output_file: String,
    pub text: String,
    pub animation_type: String, // "fade_in", "slide_in", "typewriter"
    pub start_time: f64,
    pub duration: f64,
    pub font_size: u32,
    pub font_color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterChainParameters {
    pub input_file: String,
    pub output_file: String,
    pub filters: Vec<FilterStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterStep {
    pub filter_type: String,
    pub intensity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionParameters {
    pub input1: String,
    pub input2: String,
    pub output_file: String,
    pub transition_type: String, // "fade", "dissolve", "wipe_left", "slide"
    pub duration: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressParameters {
    pub input_file: String,
    pub output_file: String,
    pub compression_level: String, // "light", "medium", "heavy", "extreme"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractFramesParameters {
    pub input_file: String,
    pub output_dir: String,
    pub fps: f64,
    pub format: String, // "png", "jpg", "bmp"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformExportParameters {
    pub input_file: String,
    pub output_file: String,
    pub platform: String, // "youtube", "instagram", "tiktok", etc.
}

// Utility functions for OperationResult
impl OperationResult {
    pub fn success(operation: &str, output_file: &str, duration: f64, message: &str) -> Self {
        Self {
            success: true,
            output_file: output_file.to_string(),
            operation: operation.to_string(),
            duration_seconds: duration,
            message: message.to_string(),
            error: None,
        }
    }

    pub fn failure(operation: &str, output_file: &str, message: &str, error: &str) -> Self {
        Self {
            success: false,
            output_file: output_file.to_string(),
            operation: operation.to_string(),
            duration_seconds: 0.0,
            message: message.to_string(),
            error: Some(error.to_string()),
        }
    }
}