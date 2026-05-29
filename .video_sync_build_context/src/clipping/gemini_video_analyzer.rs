// Single-call video analysis via YouTube URL
//
// Uses Gemini's native YouTube video understanding (fileData with fileUri) to analyze
// a full video in ONE API call instead of 100+ frame-by-frame calls.
//
// media_resolution: "low" processes video at ~100 tokens/second.
// A 10-min video costs ~60,000 tokens; a 30-min video ~180,000 tokens.
// Returns structured JSON with viral moments, each including timestamps and quality scores.

use serde::{Deserialize, Serialize};

/// A single viral moment identified by Gemini during video analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViralMoment {
    /// Start timestamp in seconds
    pub start_sec: f64,
    /// End timestamp in seconds
    pub end_sec: f64,
    /// Duration in seconds (derived from start/end)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_sec: Option<f64>,
    /// Short, engaging title for YouTube Shorts (≤60 chars)
    pub title: String,
    /// First line / hook sentence to grab attention in description
    pub hook: String,
    /// Quality/virality score from 0.0 (low) to 1.0 (high)
    pub quality_score: f64,
    /// Contributing viral factors (e.g. ["surprise", "humor", "action"])
    pub viral_factors: Vec<String>,
    /// Best timestamp within the clip for thumbnail extraction
    pub thumbnail_sec: f64,
    /// Explanation of why this moment is engaging
    pub reason: String,
}

impl ViralMoment {
    /// Duration of the clip in seconds
    pub fn duration(&self) -> f64 {
        self.end_sec - self.start_sec
    }
}

/// Full video analysis result returned by `analyze_video_from_url`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoAnalysis {
    /// Plain-text summary of the entire video (used for Qdrant embedding)
    pub video_summary: String,
    /// High-level content category
    pub content_type: String,
    /// Overall video quality score 0.0–1.0
    pub overall_quality: f64,
    /// Ordered list of identified viral moments (highest quality first)
    pub viral_moments: Vec<ViralMoment>,
}

impl VideoAnalysis {
    /// Return only moments that meet the quality threshold (default 0.6)
    pub fn qualified_moments(&self, min_quality: f64) -> Vec<&ViralMoment> {
        self.viral_moments
            .iter()
            .filter(|m| m.quality_score >= min_quality)
            .collect()
    }

    /// Return top-N moments by quality score
    pub fn top_moments(&self, n: usize) -> Vec<&ViralMoment> {
        let mut sorted: Vec<&ViralMoment> = self.viral_moments.iter().collect();
        sorted.sort_by(|a, b| {
            b.quality_score
                .partial_cmp(&a.quality_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted.truncate(n);
        sorted
    }
}
