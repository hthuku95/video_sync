-- Performance Tracking & Learning System Migration
-- Implements Recommendation 5: Feedback Loop for Continuous Learning
-- Tracks clip performance and learns which viral factors drive engagement

-- Table 1: Detailed Analytics History
-- Stores historical analytics snapshots for trend analysis
CREATE TABLE clip_analytics_history (
    id SERIAL PRIMARY KEY,
    clip_id INTEGER NOT NULL REFERENCES extracted_clips(id) ON DELETE CASCADE,
    youtube_video_id VARCHAR(255) NOT NULL,

    -- Performance metrics
    views INTEGER NOT NULL DEFAULT 0,
    likes INTEGER NOT NULL DEFAULT 0,
    dislikes INTEGER NOT NULL DEFAULT 0,
    comments INTEGER NOT NULL DEFAULT 0,
    shares INTEGER NOT NULL DEFAULT 0,

    -- Engagement rates (calculated)
    like_rate DECIMAL(5,4),  -- likes / views
    comment_rate DECIMAL(5,4),  -- comments / views

    -- Demographics (from YouTube Analytics)
    avg_watch_percentage DECIMAL(5,2),  -- % of video watched on average
    audience_retention_first_30s DECIMAL(5,2),  -- Critical for Shorts

    -- Traffic sources
    traffic_source_browse_features DECIMAL(5,2),  -- % from YouTube homepage
    traffic_source_suggested_videos DECIMAL(5,2),  -- % from recommendations
    traffic_source_shorts_feed DECIMAL(5,2),  -- % from Shorts feed
    traffic_source_external DECIMAL(5,2),  -- % from external sites

    -- Snapshot metadata
    data_fetched_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    hours_since_published INTEGER,  -- Age of clip when data fetched

    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_analytics_history_clip ON clip_analytics_history(clip_id);
CREATE INDEX idx_analytics_history_video ON clip_analytics_history(youtube_video_id);
CREATE INDEX idx_analytics_history_fetched ON clip_analytics_history(data_fetched_at);

-- Table 2: Viral Factor Performance Statistics
-- Learns which viral_factors correlate with high engagement
CREATE TABLE viral_factor_performance (
    id SERIAL PRIMARY KEY,
    viral_factor VARCHAR(255) NOT NULL,  -- e.g., "dramatic_hook", "plot_twist"

    -- Aggregate statistics
    times_used INTEGER NOT NULL DEFAULT 0,
    total_clips INTEGER NOT NULL DEFAULT 0,

    -- Average performance metrics
    avg_views DECIMAL(12,2) NOT NULL DEFAULT 0,
    avg_likes DECIMAL(12,2) NOT NULL DEFAULT 0,
    avg_comments DECIMAL(12,2) NOT NULL DEFAULT 0,
    avg_like_rate DECIMAL(5,4) NOT NULL DEFAULT 0,
    avg_comment_rate DECIMAL(5,4) NOT NULL DEFAULT 0,
    avg_watch_percentage DECIMAL(5,2) NOT NULL DEFAULT 0,

    -- Performance score (calculated: weighted combination of metrics)
    performance_score DECIMAL(8,4) NOT NULL DEFAULT 0,

    -- Ranking among all factors
    rank INTEGER,

    -- Last updated timestamp
    last_calculated_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),

    UNIQUE(viral_factor)
);

CREATE INDEX idx_viral_factor_score ON viral_factor_performance(performance_score DESC);
CREATE INDEX idx_viral_factor_rank ON viral_factor_performance(rank);

-- Table 3: Clip Duration Performance Analysis
-- Learns optimal clip lengths for different content types
CREATE TABLE duration_performance_analysis (
    id SERIAL PRIMARY KEY,
    duration_bucket VARCHAR(50) NOT NULL,  -- e.g., "30-40s", "40-50s", "50-60s"
    duration_min INTEGER NOT NULL,
    duration_max INTEGER NOT NULL,

    -- Statistics
    total_clips INTEGER NOT NULL DEFAULT 0,
    avg_views DECIMAL(12,2) NOT NULL DEFAULT 0,
    avg_engagement_rate DECIMAL(5,4) NOT NULL DEFAULT 0,
    avg_watch_percentage DECIMAL(5,2) NOT NULL DEFAULT 0,

    -- Performance score
    performance_score DECIMAL(8,4) NOT NULL DEFAULT 0,

    last_calculated_at TIMESTAMPTZ DEFAULT NOW(),

    UNIQUE(duration_bucket)
);

CREATE INDEX idx_duration_performance_score ON duration_performance_analysis(performance_score DESC);

-- Table 4: Tag Performance Statistics
-- Tracks which tags drive discovery and views
CREATE TABLE tag_performance (
    id SERIAL PRIMARY KEY,
    tag VARCHAR(255) NOT NULL,

    -- Usage statistics
    times_used INTEGER NOT NULL DEFAULT 0,

    -- Performance metrics
    avg_views DECIMAL(12,2) NOT NULL DEFAULT 0,
    avg_discovery_rate DECIMAL(5,4) NOT NULL DEFAULT 0,  -- % from search/browse

    -- Performance score
    performance_score DECIMAL(8,4) NOT NULL DEFAULT 0,

    last_calculated_at TIMESTAMPTZ DEFAULT NOW(),

    UNIQUE(tag)
);

CREATE INDEX idx_tag_performance_score ON tag_performance(performance_score DESC);

-- Table 5: Learning Recommendations
-- Stores AI-generated insights and recommendations based on performance data
CREATE TABLE learning_recommendations (
    id SERIAL PRIMARY KEY,
    recommendation_type VARCHAR(100) NOT NULL,  -- 'viral_factor', 'duration', 'tag', 'general'

    -- Recommendation details
    recommendation TEXT NOT NULL,
    confidence DECIMAL(3,2) NOT NULL,  -- 0.00 to 1.00
    supporting_data JSONB,  -- Detailed statistics supporting this recommendation

    -- Implementation tracking
    is_active BOOLEAN DEFAULT true,
    implemented_at TIMESTAMPTZ,
    effectiveness_score DECIMAL(5,4),  -- Track if recommendation actually helped

    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_learning_recs_type ON learning_recommendations(recommendation_type);
CREATE INDEX idx_learning_recs_active ON learning_recommendations(is_active);
CREATE INDEX idx_learning_recs_confidence ON learning_recommendations(confidence DESC);

-- Table 6: Clip Comparison Analysis
-- Compares similar clips to understand what makes one outperform another
CREATE TABLE clip_comparison_insights (
    id SERIAL PRIMARY KEY,

    -- Clips being compared
    high_performer_clip_id INTEGER NOT NULL REFERENCES extracted_clips(id) ON DELETE CASCADE,
    low_performer_clip_id INTEGER NOT NULL REFERENCES extracted_clips(id) ON DELETE CASCADE,

    -- Performance differential
    views_differential INTEGER NOT NULL,
    engagement_differential DECIMAL(5,4) NOT NULL,

    -- Identified differences
    key_differences JSONB NOT NULL,  -- What made the difference
    insight TEXT,  -- AI-generated insight

    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_clip_comparison_high ON clip_comparison_insights(high_performer_clip_id);
CREATE INDEX idx_clip_comparison_low ON clip_comparison_insights(low_performer_clip_id);

-- View: Top Performing Viral Factors (for quick queries)
CREATE VIEW top_viral_factors AS
SELECT
    viral_factor,
    avg_views,
    avg_like_rate,
    avg_comment_rate,
    performance_score,
    rank
FROM viral_factor_performance
WHERE total_clips >= 3  -- Minimum sample size for statistical significance
ORDER BY performance_score DESC
LIMIT 20;

-- View: Optimal Clip Durations
CREATE VIEW optimal_durations AS
SELECT
    duration_bucket,
    duration_min,
    duration_max,
    total_clips,
    avg_views,
    avg_engagement_rate,
    performance_score
FROM duration_performance_analysis
WHERE total_clips >= 5  -- Minimum sample size
ORDER BY performance_score DESC;

-- View: High-Performing Tags
CREATE VIEW top_performing_tags AS
SELECT
    tag,
    times_used,
    avg_views,
    avg_discovery_rate,
    performance_score
FROM tag_performance
WHERE times_used >= 3  -- Minimum usage count
ORDER BY performance_score DESC
LIMIT 30;

-- Function: Calculate Performance Score for a Viral Factor
-- Weighted combination of views, engagement, and watch time
CREATE OR REPLACE FUNCTION calculate_viral_factor_performance_score(
    p_avg_views DECIMAL,
    p_avg_like_rate DECIMAL,
    p_avg_comment_rate DECIMAL,
    p_avg_watch_percentage DECIMAL
) RETURNS DECIMAL AS $$
BEGIN
    RETURN (
        (p_avg_views / 10000.0) * 0.4 +  -- 40% weight on views (normalized)
        p_avg_like_rate * 0.25 +          -- 25% weight on like rate
        p_avg_comment_rate * 0.20 +       -- 20% weight on comment rate
        (p_avg_watch_percentage / 100.0) * 0.15  -- 15% weight on watch time
    );
END;
$$ LANGUAGE plpgsql IMMUTABLE;

-- Function: Update Viral Factor Statistics
-- Recalculates performance statistics for all viral factors
CREATE OR REPLACE FUNCTION refresh_viral_factor_performance()
RETURNS void AS $$
BEGIN
    -- Clear existing data
    TRUNCATE viral_factor_performance;

    -- Calculate new statistics
    INSERT INTO viral_factor_performance (
        viral_factor,
        times_used,
        total_clips,
        avg_views,
        avg_likes,
        avg_comments,
        avg_like_rate,
        avg_comment_rate,
        avg_watch_percentage,
        performance_score,
        last_calculated_at
    )
    SELECT
        unnest(ec.viral_factors) as viral_factor,
        COUNT(*) as times_used,
        COUNT(DISTINCT ec.id) as total_clips,
        AVG(COALESCE(cah.views, ec.views_24h)) as avg_views,
        AVG(COALESCE(cah.likes, ec.likes_24h)) as avg_likes,
        AVG(COALESCE(cah.comments, ec.comments_24h)) as avg_comments,
        AVG(cah.like_rate) as avg_like_rate,
        AVG(cah.comment_rate) as avg_comment_rate,
        AVG(cah.avg_watch_percentage) as avg_watch_percentage,
        calculate_viral_factor_performance_score(
            AVG(COALESCE(cah.views, ec.views_24h)),
            AVG(cah.like_rate),
            AVG(cah.comment_rate),
            AVG(cah.avg_watch_percentage)
        ) as performance_score,
        NOW() as last_calculated_at
    FROM extracted_clips ec
    LEFT JOIN LATERAL (
        SELECT * FROM clip_analytics_history
        WHERE clip_id = ec.id
        ORDER BY data_fetched_at DESC
        LIMIT 1
    ) cah ON true
    WHERE ec.upload_status = 'published'
      AND ec.viral_factors IS NOT NULL
      AND array_length(ec.viral_factors, 1) > 0
    GROUP BY unnest(ec.viral_factors);

    -- Update rankings
    WITH ranked_factors AS (
        SELECT
            id,
            ROW_NUMBER() OVER (ORDER BY performance_score DESC) as new_rank
        FROM viral_factor_performance
    )
    UPDATE viral_factor_performance vfp
    SET rank = rf.new_rank
    FROM ranked_factors rf
    WHERE vfp.id = rf.id;
END;
$$ LANGUAGE plpgsql;

-- Trigger: Auto-update analytics when clip_analytics_history is updated
CREATE OR REPLACE FUNCTION trigger_refresh_performance_stats()
RETURNS TRIGGER AS $$
BEGIN
    -- Schedule async refresh (to avoid blocking the insert)
    -- In practice, this will be called by a background job
    -- For now, just log that refresh is needed
    RAISE NOTICE 'Performance statistics refresh needed';
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER analytics_history_updated
AFTER INSERT ON clip_analytics_history
FOR EACH STATEMENT
EXECUTE FUNCTION trigger_refresh_performance_stats();

-- Initial data: Seed with empty statistics (will be populated by analytics sync job)
-- This ensures queries don't fail before first sync
INSERT INTO viral_factor_performance (viral_factor, times_used, total_clips, performance_score)
VALUES ('initialization', 0, 0, 0.0)
ON CONFLICT (viral_factor) DO NOTHING;
