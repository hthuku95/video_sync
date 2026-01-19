-- Add Thumbnail Support to Clipping System
-- Enables AI-generated custom thumbnails for every clip

-- Add thumbnail tracking to extracted_clips table
ALTER TABLE extracted_clips
ADD COLUMN custom_thumbnail_path VARCHAR(512),
ADD COLUMN thumbnail_uploaded_at TIMESTAMPTZ,
ADD COLUMN thumbnail_generation_method VARCHAR(50);  -- 'ai_generated', 'frame_extraction', 'hybrid'

-- Add indexes for thumbnail queries
CREATE INDEX idx_extracted_clips_thumbnail_uploaded ON extracted_clips(thumbnail_uploaded_at)
WHERE custom_thumbnail_path IS NOT NULL;

-- Add thumbnail performance tracking to analytics history
ALTER TABLE clip_analytics_history
ADD COLUMN thumbnail_click_through_rate DECIMAL(5,4);  -- CTR = clicks / impressions

-- Table: Track thumbnail generation performance for learning
CREATE TABLE thumbnail_performance_analysis (
    id SERIAL PRIMARY KEY,
    generation_method VARCHAR(50) NOT NULL,  -- 'ai_generated', 'frame_extraction', 'hybrid'
    text_overlay_style VARCHAR(100),  -- Style of text overlay used
    frame_selection_strategy VARCHAR(100),  -- How frame was selected

    -- Performance metrics
    total_clips INTEGER NOT NULL DEFAULT 0,
    avg_ctr DECIMAL(5,4) NOT NULL DEFAULT 0,  -- Average click-through rate
    avg_views INTEGER NOT NULL DEFAULT 0,

    -- Performance score
    performance_score DECIMAL(8,4) NOT NULL DEFAULT 0,

    last_calculated_at TIMESTAMPTZ DEFAULT NOW(),

    UNIQUE(generation_method, text_overlay_style, frame_selection_strategy)
);

CREATE INDEX idx_thumbnail_performance_score ON thumbnail_performance_analysis(performance_score DESC);

-- View: Top-performing thumbnail strategies
CREATE VIEW top_thumbnail_strategies AS
SELECT
    generation_method,
    text_overlay_style,
    frame_selection_strategy,
    total_clips,
    avg_ctr,
    avg_views,
    performance_score
FROM thumbnail_performance_analysis
WHERE total_clips >= 5  -- Minimum sample size
ORDER BY performance_score DESC
LIMIT 10;

-- Function: Calculate thumbnail performance score
CREATE OR REPLACE FUNCTION calculate_thumbnail_performance_score(
    p_avg_ctr DECIMAL,
    p_avg_views INTEGER
) RETURNS DECIMAL AS $$
BEGIN
    RETURN (
        (p_avg_ctr * 100.0) * 0.6 +  -- 60% weight on CTR (most important for thumbnails)
        (p_avg_views / 10000.0) * 0.4  -- 40% weight on views (normalized)
    );
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- Add learning recommendation type for thumbnails
INSERT INTO learning_recommendations (recommendation_type, recommendation, confidence, supporting_data, is_active)
VALUES (
    'thumbnail_strategy',
    'Initial setup - will be populated after first thumbnail performance data is collected',
    0.5,
    '{"note": "Collecting baseline data"}',
    true
)
ON CONFLICT (recommendation_type) DO NOTHING;

-- Comments for documentation
COMMENT ON COLUMN extracted_clips.custom_thumbnail_path IS 'Path to custom AI-generated thumbnail image';
COMMENT ON COLUMN extracted_clips.thumbnail_generation_method IS 'Method used to generate thumbnail: ai_generated, frame_extraction, or hybrid';
COMMENT ON TABLE thumbnail_performance_analysis IS 'Tracks which thumbnail generation strategies perform best for learning';
