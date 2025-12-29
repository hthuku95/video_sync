-- Insert the missing migration record to fix the version mismatch
INSERT INTO _sqlx_migrations (version, checksum, installed_on) 
VALUES (20250909000000, '', NOW()) 
ON CONFLICT (version) DO NOTHING;