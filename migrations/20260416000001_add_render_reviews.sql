-- Audit log of every LLM QA review on a rendered output.
-- Populated by src/render_review.rs after each successful render, so the
-- admin dashboard can see pass/fail rate per tool over time + spot the
-- tools that keep producing garbage.

CREATE TABLE IF NOT EXISTS blender_render_reviews (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tool_name    TEXT NOT NULL,                -- e.g. 'blender_generate_scene', 'ffmpeg_trim', 'auto_generate_video'
    delivery_id  UUID REFERENCES deliveries(id) ON DELETE SET NULL,
    output_url   TEXT NOT NULL,                -- R2 URL of the rendered file
    pass         BOOLEAN NOT NULL,
    score        INT NOT NULL,                 -- 1-10
    feedback     TEXT,
    retry_hint   TEXT,                         -- set when pass=false and we can retry with a hint
    reviewed_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_render_reviews_tool_time
    ON blender_render_reviews(tool_name, reviewed_at DESC);
CREATE INDEX IF NOT EXISTS idx_render_reviews_delivery
    ON blender_render_reviews(delivery_id)
    WHERE delivery_id IS NOT NULL;
