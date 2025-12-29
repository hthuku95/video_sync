-- Add admin fields to users table for Django-like admin functionality
ALTER TABLE users 
ADD COLUMN is_superuser BOOLEAN NOT NULL DEFAULT false,
ADD COLUMN is_staff BOOLEAN NOT NULL DEFAULT false;

-- Create an index for quick admin user queries
CREATE INDEX idx_users_admin_status ON users(is_superuser, is_staff) WHERE is_superuser = true OR is_staff = true;

-- Add some useful comments
COMMENT ON COLUMN users.is_superuser IS 'Designates that this user has all permissions without explicitly assigning them (Django equivalent)';
COMMENT ON COLUMN users.is_staff IS 'Designates whether the user can access the admin interface';