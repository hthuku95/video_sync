-- Add output videos table for storing generated video metadata
CREATE TABLE IF NOT EXISTS output_videos (
    id SERIAL PRIMARY KEY,
    session_id INTEGER REFERENCES chat_sessions(id) ON DELETE CASCADE,
    user_id INTEGER REFERENCES users(id) ON DELETE CASCADE,
    original_input_file_id VARCHAR(255), -- Reference to uploaded_files.id
    file_name VARCHAR(255) NOT NULL,
    file_path VARCHAR(500) NOT NULL UNIQUE,
    file_size BIGINT NOT NULL,
    mime_type VARCHAR(100) NOT NULL,
    
    -- Video metadata
    duration_seconds DOUBLE PRECISION,
    width INTEGER,
    height INTEGER,
    frame_rate DOUBLE PRECISION,
    
    -- Processing information
    operation_type VARCHAR(100) NOT NULL, -- e.g., "trim", "merge", "compress"
    operation_params TEXT, -- JSON string of operation parameters
    processing_status VARCHAR(50) NOT NULL DEFAULT 'processing', -- processing, completed, failed
    tool_used VARCHAR(100) NOT NULL, -- Which AI tool created this video
    ai_response_message TEXT, -- AI's response when creating this video
    
    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for performance
CREATE INDEX IF NOT EXISTS idx_output_videos_session_id ON output_videos(session_id);
CREATE INDEX IF NOT EXISTS idx_output_videos_user_id ON output_videos(user_id);
CREATE INDEX IF NOT EXISTS idx_output_videos_created_at ON output_videos(created_at);
CREATE INDEX IF NOT EXISTS idx_output_videos_processing_status ON output_videos(processing_status);