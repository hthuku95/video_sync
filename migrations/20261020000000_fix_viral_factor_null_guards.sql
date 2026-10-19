-- Fix: refresh_viral_factor_performance() violated the NOT NULL constraints on
-- viral_factor_performance whenever a published clip had no matching row in
-- clip_analytics_history (or had NULL 24h metrics).
--
-- The aggregates were computed as `AVG(cah.like_rate)` etc. over a
-- LEFT JOIN LATERAL, which yields NULL when the lateral finds no row — and AVG
-- over all-NULL inputs is NULL regardless. Supplying that NULL explicitly on
-- INSERT raised:
--
--   null value in column "avg_like_rate" of relation "viral_factor_performance"
--   violates not-null constraint
--
-- so the entire statistics refresh failed, every cycle, logging
-- "Failed to refresh viral factor stats". The columns' `DEFAULT 0` does not
-- help: it applies only when the column is omitted from the INSERT, not when
-- NULL is passed explicitly. Each aggregate is therefore coalesced here.
--
-- CREATE OR REPLACE is used because the original definition in
-- 20260119000000_add_performance_tracking.sql has already been applied and
-- cannot be edited in place.

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
        COALESCE(AVG(COALESCE(cah.views, ec.views_24h)), 0) as avg_views,
        COALESCE(AVG(COALESCE(cah.likes, ec.likes_24h)), 0) as avg_likes,
        COALESCE(AVG(COALESCE(cah.comments, ec.comments_24h)), 0) as avg_comments,
        COALESCE(AVG(cah.like_rate), 0) as avg_like_rate,
        COALESCE(AVG(cah.comment_rate), 0) as avg_comment_rate,
        COALESCE(AVG(cah.avg_watch_percentage), 0) as avg_watch_percentage,
        COALESCE(
            calculate_viral_factor_performance_score(
                COALESCE(AVG(COALESCE(cah.views, ec.views_24h)), 0),
                COALESCE(AVG(cah.like_rate), 0),
                COALESCE(AVG(cah.comment_rate), 0),
                COALESCE(AVG(cah.avg_watch_percentage), 0)
            ),
            0
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
