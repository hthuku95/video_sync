use crate::models::{admin::*, auth::*};
use crate::middleware::admin::{admin_middleware, superuser_middleware};
use crate::middleware::auth::auth_middleware;
use crate::AppState;
use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    response::{Html, Json},
    routing::{get, post, put, delete},
    Router,
};
use bcrypt::{hash, DEFAULT_COST};
use serde::Deserialize;
use serde_json::json;
use sqlx::{FromRow, Row};
use std::sync::Arc;

pub fn admin_routes() -> Router {
    // HTML pages - public routes with JavaScript authentication
    let public_admin = Router::new()
        .route("/admin", get(admin_login_page))
        .route("/admin/login", get(admin_login_page))
        .route("/admin/dashboard", get(admin_dashboard))
        .route("/admin/users", get(admin_users_list))
        .route("/admin/users/:id", get(admin_user_detail))
        .route("/admin/clipping-activity", get(admin_clipping_activity_page))
        .route("/admin/clipping-jobs", get(admin_clipping_jobs_page));
    
    // API endpoints - protected routes with JWT authentication  
    let protected_admin = Router::new()
        .route("/admin/users", post(admin_create_user))
        .route("/admin/users/:id", put(admin_update_user))
        .route("/admin/users/:id", delete(admin_delete_user))
        .route("/api/admin/stats", get(admin_stats_api))
        .route("/api/admin/users", get(admin_users_api))
        .route("/api/admin/users/:id", get(admin_user_api))
        .route("/api/admin/users/:id", put(admin_update_user_api))
        .route("/api/admin/users/:id/toggle-active", post(admin_toggle_user_active))
        .route("/api/admin/users/:id/make-staff", post(admin_make_staff))
        .route("/api/admin/users/:id/remove-staff", post(admin_remove_staff))
        .route("/api/admin/whitelist/status", get(get_whitelist_status))
        .route("/api/admin/whitelist/toggle", post(toggle_whitelist))
        .route("/api/admin/whitelist/emails", get(get_whitelist_emails))
        .route("/api/admin/whitelist/emails", post(add_whitelist_email))
        .route("/api/admin/whitelist/emails/:id", delete(remove_whitelist_email))
        .route("/api/admin/pricing", get(get_model_pricing))
        .route("/api/admin/pricing", post(update_model_pricing))
        .route("/api/admin/default-model", get(get_default_model))
        .route("/api/admin/default-model", post(update_default_model))
        .route("/api/admin/youtube/status", get(get_youtube_feature_status))
        .route("/api/admin/youtube/toggle", post(toggle_youtube_features))
        .route("/api/admin/clipping/stats", get(admin_clipping_stats))
        .route("/api/admin/clipping/user/:user_id/details", get(admin_user_clipping_details))
        .route("/api/admin/clipping/jobs", get(admin_list_all_jobs))
        .route("/api/admin/clipping/jobs/:id", get(admin_get_job_details))
        .route("/api/admin/clipping/jobs/:id/retry", post(admin_retry_job))
        .route("/api/admin/clipping/jobs/:id/cancel", post(admin_cancel_job))
        .route("/api/admin/clipping/jobs/:id/clips", get(admin_get_job_clips))
        .route("/api/admin/performance/viral-factors", get(admin_viral_factor_performance))
        .route("/api/admin/performance/channel-health", get(admin_channel_health))
        .route("/api/admin/performance/recommendations", get(admin_learning_recommendations))
        .layer(axum::middleware::from_fn(admin_middleware))
        .layer(axum::middleware::from_fn(auth_middleware));
    
    let superuser_only = Router::new()
        .route("/api/admin/users/:id/make-superuser", post(admin_make_superuser))
        .route("/api/admin/users/:id/remove-superuser", post(admin_remove_superuser))
        .route("/api/admin/create-superuser", post(create_superuser_api))
        .layer(axum::middleware::from_fn(superuser_middleware))
        .layer(axum::middleware::from_fn(auth_middleware));
    
    public_admin.merge(protected_admin).merge(superuser_only)
}

#[derive(Deserialize)]
pub struct CreateSuperuserRequest {
    pub email: String,
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub email: String,
    pub username: String,
    pub password: String,
    pub is_staff: Option<bool>,
}

#[derive(Deserialize)]
pub struct UpdateUserRequest {
    pub email: Option<String>,
    pub username: Option<String>,
    pub is_active: Option<bool>,
    pub is_staff: Option<bool>,
}

#[derive(Deserialize)]
pub struct UsersQuery {
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub search: Option<String>,
}

#[derive(Deserialize)]
pub struct JobsQuery {
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub status: Option<String>,
    pub user_id: Option<i32>,
    pub sort: Option<String>, // "created_desc", "created_asc", "updated_desc"
}

pub async fn admin_login_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Admin Login - VideoSync</title>
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #f8f9fa; display: flex; align-items: center; justify-content: center; min-height: 100vh; margin: 0; }
        .admin-container { background: white; padding: 3rem; border-radius: 10px; box-shadow: 0 10px 30px rgba(0,0,0,0.1); width: 100%; max-width: 400px; }
        .admin-header { text-align: center; margin-bottom: 2rem; }
        .admin-header h1 { color: #dc3545; font-size: 2rem; margin-bottom: 0.5rem; }
        .admin-header p { color: #6c757d; }
        .form-group { margin-bottom: 1.5rem; }
        .form-group label { display: block; margin-bottom: 0.5rem; color: #2c3e50; font-weight: 600; }
        .form-group input { width: 100%; padding: 0.75rem; border: 2px solid #e9ecef; border-radius: 8px; font-size: 1rem; }
        .form-group input:focus { outline: none; border-color: #dc3545; }
        .btn { width: 100%; padding: 0.75rem; background: #dc3545; color: white; border: none; border-radius: 8px; font-size: 1rem; font-weight: 600; cursor: pointer; }
        .btn:hover { background: #c82333; }
        .warning { background: #fff3cd; border: 1px solid #ffeaa7; padding: 1rem; border-radius: 8px; margin-bottom: 1rem; color: #856404; }
        .error { background: #f8d7da; border: 1px solid #f5c6cb; padding: 1rem; border-radius: 8px; margin-bottom: 1rem; color: #721c24; display: none; }
    </style>
</head>
<body>
    <div class="admin-container">
        <div class="admin-header">
            <h1>🛡️ Admin Login</h1>
            <p>Administrative Access Only</p>
        </div>
        
        <div class="warning">
            <strong>⚠️ Restricted Area:</strong> This area is for administrators only. Unauthorized access is prohibited.
        </div>
        
        <div id="errorMessage" class="error"></div>
        
        <form id="adminLoginForm">
            <div class="form-group">
                <label for="email">Admin Email</label>
                <input type="email" id="email" name="email" required>
            </div>
            
            <div class="form-group">
                <label for="password">Password</label>
                <input type="password" id="password" name="password" required>
            </div>
            
            <button type="submit" class="btn">Access Admin Panel</button>
        </form>
        
        <div style="text-align: center; margin-top: 1.5rem;">
            <a href="/" style="color: #6c757d; text-decoration: none;">← Back to Main Site</a>
        </div>
    </div>
    
    <script>
        document.getElementById('adminLoginForm').addEventListener('submit', async (e) => {
            e.preventDefault();
            
            const email = document.getElementById('email').value;
            const password = document.getElementById('password').value;
            
            try {
                const response = await fetch('/api/auth/login', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ email, password }),
                });
                
                const data = await response.json();
                
                if (data.success) {
                    // Check if user has admin privileges
                    if (data.user.is_staff || data.user.is_superuser) {
                        localStorage.setItem('authToken', data.token);
                        localStorage.setItem('user', JSON.stringify(data.user));
                        window.location.href = '/admin/dashboard';
                    } else {
                        document.getElementById('errorMessage').textContent = 'Access denied. Admin privileges required.';
                        document.getElementById('errorMessage').style.display = 'block';
                    }
                } else {
                    document.getElementById('errorMessage').textContent = data.message;
                    document.getElementById('errorMessage').style.display = 'block';
                }
            } catch (error) {
                document.getElementById('errorMessage').textContent = 'Network error. Please try again.';
                document.getElementById('errorMessage').style.display = 'block';
            }
        });
    </script>
</body>
</html>
    "###;
    
    Html(html.to_string())
}

pub async fn admin_dashboard() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Admin Dashboard - VideoSync</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #f8f9fa; }
        .sidebar { width: 250px; background: #343a40; height: 100vh; position: fixed; left: 0; top: 0; color: white; padding: 1rem; }
        .sidebar h2 { color: #dc3545; margin-bottom: 2rem; }
        .sidebar ul { list-style: none; }
        .sidebar li { margin-bottom: 0.5rem; }
        .sidebar a { color: #adb5bd; text-decoration: none; padding: 0.5rem; display: block; border-radius: 5px; }
        .sidebar a:hover { background: #495057; color: white; }
        .sidebar a.active { background: #dc3545; color: white; }
        .main-content { margin-left: 250px; padding: 2rem; }
        .header { background: white; padding: 1rem 2rem; margin-bottom: 2rem; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); display: flex; justify-content: space-between; align-items: center; }
        .stats-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(250px, 1fr)); gap: 1.5rem; margin-bottom: 2rem; }
        .stat-card { background: white; padding: 1.5rem; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
        .stat-number { font-size: 2rem; font-weight: bold; color: #dc3545; }
        .stat-label { color: #6c757d; margin-top: 0.5rem; }
        .recent-section { background: white; padding: 2rem; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
        .btn { padding: 0.5rem 1rem; background: #dc3545; color: white; border: none; border-radius: 5px; cursor: pointer; text-decoration: none; display: inline-block; }
        .btn:hover { background: #c82333; }
        .btn-secondary { background: #6c757d; }
        .btn-secondary:hover { background: #5a6268; }
        table { width: 100%; border-collapse: collapse; margin-top: 1rem; }
        th, td { padding: 0.75rem; text-align: left; border-bottom: 1px solid #dee2e6; }
        th { background: #f8f9fa; font-weight: 600; }
        .badge { padding: 0.25rem 0.5rem; border-radius: 3px; font-size: 0.8rem; }
        .badge-success { background: #d4edda; color: #155724; }
        .badge-danger { background: #f8d7da; color: #721c24; }
        .badge-warning { background: #fff3cd; color: #856404; }
        .whitelist-section { background: white; padding: 2rem; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); margin-bottom: 2rem; }
        .toggle-switch { position: relative; display: inline-block; width: 60px; height: 34px; }
        .toggle-switch input { opacity: 0; width: 0; height: 0; }
        .slider { position: absolute; cursor: pointer; top: 0; left: 0; right: 0; bottom: 0; background-color: #ccc; transition: .4s; border-radius: 34px; }
        .slider:before { position: absolute; content: ""; height: 26px; width: 26px; left: 4px; bottom: 4px; background-color: white; transition: .4s; border-radius: 50%; }
        input:checked + .slider { background-color: #dc3545; }
        input:checked + .slider:before { transform: translateX(26px); }
        .whitelist-form { display: flex; gap: 1rem; align-items: center; margin: 1rem 0; }
        .whitelist-form input { flex: 1; padding: 0.5rem; border: 1px solid #ddd; border-radius: 5px; }
        .whitelist-table { margin-top: 1rem; }
        .delete-btn { background: #dc3545; color: white; border: none; padding: 0.25rem 0.5rem; border-radius: 3px; cursor: pointer; font-size: 0.8rem; }
        .delete-btn:hover { background: #c82333; }
    </style>
</head>
<body>
    <div class="sidebar">
        <h2>🛡️ Admin Panel</h2>
        <ul>
            <li><a href="/admin/dashboard" class="active">📊 Dashboard</a></li>
            <li><a href="/admin/users">👥 Users</a></li>
            <li><a href="/admin/clipping-activity">🎬 Clipping Activity</a></li>
            <li><a href="#" onclick="showWhitelist()">🛡️ Whitelist</a></li>
            <li><a href="#" onclick="showYoutube()">🎥 YouTube Features</a></li>
            <li><a href="#" onclick="showPricing()">💰 Model Pricing</a></li>
            <li><a href="/api/docs">📚 API Docs</a></li>
            <li><a href="/api/status">⚙️ System Status</a></li>
            <li><a href="/" target="_blank">🌐 View Site</a></li>
        </ul>
        <div style="position: absolute; bottom: 1rem;">
            <button onclick="logout()" class="btn btn-secondary">Logout</button>
        </div>
    </div>
    
    <div class="main-content">
        <div class="header">
            <div>
                <h1>Admin Dashboard</h1>
                <p>Welcome back, <span id="adminName">Admin</span></p>
            </div>
            <div>
                <a href="/admin/users" class="btn">Manage Users</a>
            </div>
        </div>
        
        <div class="stats-grid">
            <div class="stat-card">
                <div class="stat-number" id="totalUsers">Loading...</div>
                <div class="stat-label">Total Users</div>
            </div>
            <div class="stat-card">
                <div class="stat-number" id="activeUsers">Loading...</div>
                <div class="stat-label">Active Users</div>
            </div>
            <div class="stat-card">
                <div class="stat-number" id="totalChats">Loading...</div>
                <div class="stat-label">Chat Sessions</div>
            </div>
            <div class="stat-card">
                <div class="stat-number" id="totalFiles">Loading...</div>
                <div class="stat-label">Uploaded Files</div>
            </div>
        </div>

        <!-- AI Model Configuration Section -->
        <div class="whitelist-section" style="margin-bottom: 2rem;">
            <h2>🤖 Default AI Model Configuration</h2>
            <p style="color: #6c757d; margin-bottom: 1.5rem;">Select which AI model all users will use by default. This affects cost and performance.</p>

            <div style="background: #f8f9fa; padding: 1.5rem; border-radius: 10px;">
                <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 1.5rem; margin-bottom: 1rem;">
                    <label style="display: flex; align-items: center; padding: 1rem; border: 2px solid #ddd; border-radius: 8px; cursor: pointer; transition: all 0.2s;" id="geminiOption">
                        <input type="radio" name="defaultModel" value="gemini" style="margin-right: 1rem; width: 20px; height: 20px;">
                        <div>
                            <div style="font-weight: 600; font-size: 1.1rem;">Gemini 2.5 Flash</div>
                            <div style="color: #6c757d; font-size: 0.9rem; margin-top: 0.25rem;">Cost: $0.30 input / $2.50 output per 1M tokens</div>
                            <div style="color: #28a745; font-size: 0.85rem; margin-top: 0.25rem;">✓ Recommended for cost efficiency</div>
                        </div>
                    </label>
                    <label style="display: flex; align-items: center; padding: 1rem; border: 2px solid #ddd; border-radius: 8px; cursor: pointer; transition: all 0.2s;" id="claudeOption">
                        <input type="radio" name="defaultModel" value="claude" style="margin-right: 1rem; width: 20px; height: 20px;">
                        <div>
                            <div style="font-weight: 600; font-size: 1.1rem;">Claude Sonnet 4.5</div>
                            <div style="color: #6c757d; font-size: 0.9rem; margin-top: 0.25rem;">Cost: $3 input / $15 output per 1M tokens (base)</div>
                            <div style="color: #ffc107; font-size: 0.85rem; margin-top: 0.25rem;">⚠ Higher cost, premium quality</div>
                        </div>
                    </label>
                </div>
                <button onclick="updateDefaultModel()" class="btn" style="width: auto;">Save Default Model</button>
                <div id="modelUpdateStatus" style="margin-top: 1rem; font-weight: 600;"></div>
            </div>
        </div>

        <div id="whitelistSection" class="whitelist-section" style="display: none;">
            <h2>Email Whitelist Management</h2>
            <div style="display: flex; align-items: center; gap: 1rem; margin-bottom: 1.5rem;">
                <span>Whitelist Status:</span>
                <label class="toggle-switch">
                    <input type="checkbox" id="whitelistToggle">
                    <span class="slider"></span>
                </label>
                <span id="whitelistStatus">Loading...</span>
            </div>
            
            <div class="whitelist-form">
                <input type="email" id="newEmail" placeholder="Enter email address to whitelist" required>
                <button onclick="addEmail()" class="btn">Add Email</button>
            </div>
            
            <div class="whitelist-table">
                <h3>Whitelisted Emails (<span id="emailCount">0</span>)</h3>
                <table>
                    <thead>
                        <tr>
                            <th>Email</th>
                            <th>Added On</th>
                            <th>Actions</th>
                        </tr>
                    </thead>
                    <tbody id="whitelistEmails">
                        <tr><td colspan="3" style="text-align: center;">Loading...</td></tr>
                    </tbody>
                </table>
            </div>
        </div>

        <div id="youtubeSection" class="whitelist-section" style="display: none;">
            <h2>🎥 YouTube Integration Control</h2>
            <p style="color: #6c757d; margin-bottom: 1.5rem;">Control access to YouTube features (upload, analytics, playlists, comments).</p>

            <div style="display: flex; align-items: center; gap: 1rem; margin-bottom: 1rem;">
                <label class="toggle-switch">
                    <input type="checkbox" id="youtubeFeatureToggle">
                    <span class="slider"></span>
                </label>
                <span id="youtubeFeatureStatus" style="font-weight: 600;">Loading...</span>
            </div>

            <div style="background: #fff3cd; border: 1px solid #ffeaa7; padding: 1rem; border-radius: 8px; margin-top: 1rem;">
                <strong>When Enabled:</strong> All authenticated users can access YouTube features.<br>
                <strong>When Disabled:</strong> Only admins and whitelisted users have access (testing mode).<br><br>
                <em style="color: #856404;">💡 Tip: Keep disabled during Google OAuth verification testing, then enable for all users after approval.</em>
            </div>
        </div>

        <div id="pricingSection" class="whitelist-section" style="display: none;">
            <h2>💰 Model Pricing Management</h2>
            <p style="color: #6c757d; margin-bottom: 1.5rem;">Update official API pricing for accurate cost tracking. Prices are in USD per 1 million tokens.</p>

            <div id="pricingModels">
                <div style="text-align: center; padding: 2rem; color: #6c757d;">
                    Loading pricing data...
                </div>
            </div>
        </div>

        <div class="recent-section">
            <h2>Recent Users</h2>
            <table>
                <thead>
                    <tr>
                        <th>Username</th>
                        <th>Email</th>
                        <th>Status</th>
                        <th>Role</th>
                        <th>Joined</th>
                        <th>Actions</th>
                    </tr>
                </thead>
                <tbody id="recentUsers">
                    <tr><td colspan="6" style="text-align: center;">Loading...</td></tr>
                </tbody>
            </table>
        </div>
    </div>
    
    <script>
        // Check admin authentication
        const authToken = localStorage.getItem('authToken');
        const user = JSON.parse(localStorage.getItem('user') || '{}');
        
        if (!authToken || (!user.is_staff && !user.is_superuser)) {
            window.location.href = '/admin/login';
        }
        
        document.getElementById('adminName').textContent = user.username || 'Admin';
        
        // Load dashboard data
        async function loadDashboardData() {
            try {
                const response = await fetch('/api/admin/stats', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();
                
                if (data.success) {
                    document.getElementById('totalUsers').textContent = data.stats.total_users;
                    document.getElementById('activeUsers').textContent = data.stats.active_users;
                    document.getElementById('totalChats').textContent = data.stats.total_chat_sessions;
                    document.getElementById('totalFiles').textContent = data.stats.total_files;
                }
            } catch (error) {
                console.error('Error loading stats:', error);
            }
            
            // Load recent users
            try {
                const response = await fetch('/api/admin/users?limit=5', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();
                
                if (data.success) {
                    const tbody = document.getElementById('recentUsers');
                    tbody.innerHTML = data.users.map(user => `
                        <tr>
                            <td>${user.username}</td>
                            <td>${user.email}</td>
                            <td><span class="badge ${user.is_active ? 'badge-success' : 'badge-danger'}">${user.is_active ? 'Active' : 'Inactive'}</span></td>
                            <td><span class="badge ${user.is_superuser ? 'badge-danger' : user.is_staff ? 'badge-warning' : 'badge-success'}">${user.is_superuser ? 'Superuser' : user.is_staff ? 'Staff' : 'User'}</span></td>
                            <td>${new Date(user.created_at).toLocaleDateString()}</td>
                            <td><a href="/admin/users/${user.id}" class="btn" style="padding: 0.25rem 0.5rem; font-size: 0.8rem;">View</a></td>
                        </tr>
                    `).join('');
                }
            } catch (error) {
                console.error('Error loading users:', error);
            }
        }
        
        function logout() {
            localStorage.removeItem('authToken');
            localStorage.removeItem('user');
            window.location.href = '/admin/login';
        }

        // Load current default model selection
        async function loadDefaultModel() {
            try {
                const response = await fetch('/api/admin/default-model', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();

                if (data.success) {
                    const modelRadio = document.querySelector(`input[name="defaultModel"][value="${data.model}"]`);
                    if (modelRadio) {
                        modelRadio.checked = true;
                        // Highlight selected option
                        updateModelHighlight(data.model);
                    }
                }
            } catch (error) {
                console.error('Error loading default model:', error);
            }
        }

        function updateModelHighlight(selectedModel) {
            document.querySelectorAll('input[name="defaultModel"]').forEach(radio => {
                const label = radio.closest('label');
                if (radio.value === selectedModel) {
                    label.style.borderColor = '#dc3545';
                    label.style.background = '#fff5f5';
                } else {
                    label.style.borderColor = '#ddd';
                    label.style.background = 'white';
                }
            });
        }

        async function updateDefaultModel() {
            const selectedModel = document.querySelector('input[name="defaultModel"]:checked');

            if (!selectedModel) {
                alert('Please select a model');
                return;
            }

            const model = selectedModel.value;
            const statusDiv = document.getElementById('modelUpdateStatus');

            try {
                const response = await fetch('/api/admin/default-model', {
                    method: 'POST',
                    headers: {
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify({ model })
                });

                const data = await response.json();
                if (data.success) {
                    statusDiv.textContent = '✅ ' + data.message;
                    statusDiv.style.color = '#28a745';
                    updateModelHighlight(model);
                    setTimeout(() => statusDiv.textContent = '', 3000);
                } else {
                    statusDiv.textContent = '❌ Error: ' + data.message;
                    statusDiv.style.color = '#dc3545';
                }
            } catch (error) {
                console.error('Error updating default model:', error);
                statusDiv.textContent = '❌ Network error';
                statusDiv.style.color = '#dc3545';
            }
        }

        // Add change listeners to radio buttons for visual feedback
        document.addEventListener('DOMContentLoaded', function() {
            loadDefaultModel();
            document.querySelectorAll('input[name="defaultModel"]').forEach(radio => {
                radio.addEventListener('change', function() {
                    updateModelHighlight(this.value);
                });
            });
        });

        // Whitelist Management Functions
        function showWhitelist() {
            document.getElementById('whitelistSection').style.display = 'block';
            document.getElementById('youtubeSection').style.display = 'none';
            document.getElementById('pricingSection').style.display = 'none';
            document.querySelector('.recent-section').style.display = 'none';
            loadWhitelistData();
        }
        
        function hideDashboard() {
            document.getElementById('whitelistSection').style.display = 'none';
            document.getElementById('youtubeSection').style.display = 'none';
            document.getElementById('pricingSection').style.display = 'none';
            document.querySelector('.recent-section').style.display = 'block';
        }
        
        async function loadWhitelistData() {
            try {
                // Load whitelist status
                const statusResponse = await fetch('/api/admin/whitelist/status', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const statusData = await statusResponse.json();
                
                if (statusData.success) {
                    const toggle = document.getElementById('whitelistToggle');
                    toggle.checked = statusData.status.enabled;
                    document.getElementById('whitelistStatus').textContent = 
                        statusData.status.enabled ? 'Enabled' : 'Disabled';
                    document.getElementById('emailCount').textContent = statusData.status.total_emails;
                }
                
                // Load whitelist emails
                const emailsResponse = await fetch('/api/admin/whitelist/emails', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const emailsData = await emailsResponse.json();
                
                if (emailsData.success) {
                    const tbody = document.getElementById('whitelistEmails');
                    if (emailsData.emails.length === 0) {
                        tbody.innerHTML = '<tr><td colspan="3" style="text-align: center;">No emails whitelisted</td></tr>';
                    } else {
                        tbody.innerHTML = emailsData.emails.map(email => `
                            <tr>
                                <td>${email.email}</td>
                                <td>${new Date(email.created_at).toLocaleDateString()}</td>
                                <td><button onclick="removeEmail(${email.id})" class="delete-btn">Remove</button></td>
                            </tr>
                        `).join('');
                    }
                }
            } catch (error) {
                console.error('Error loading whitelist data:', error);
            }
        }
        
        async function toggleWhitelist() {
            const toggle = document.getElementById('whitelistToggle');
            const enabled = toggle.checked;
            
            try {
                const response = await fetch('/api/admin/whitelist/toggle', {
                    method: 'POST',
                    headers: {
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify({ enabled })
                });
                
                const data = await response.json();
                if (data.success) {
                    document.getElementById('whitelistStatus').textContent = 
                        enabled ? 'Enabled' : 'Disabled';
                    alert(data.message);
                } else {
                    alert('Error: ' + data.message);
                    toggle.checked = !enabled; // Revert toggle
                }
            } catch (error) {
                console.error('Error toggling whitelist:', error);
                alert('Network error');
                toggle.checked = !enabled; // Revert toggle
            }
        }
        
        async function addEmail() {
            const emailInput = document.getElementById('newEmail');
            const email = emailInput.value.trim();
            
            if (!email) {
                alert('Please enter an email address');
                return;
            }
            
            try {
                const response = await fetch('/api/admin/whitelist/emails', {
                    method: 'POST',
                    headers: {
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify({ email })
                });
                
                const data = await response.json();
                if (data.success) {
                    emailInput.value = '';
                    alert(data.message);
                    loadWhitelistData(); // Reload the list
                } else {
                    alert('Error: ' + data.message);
                }
            } catch (error) {
                console.error('Error adding email:', error);
                alert('Network error');
            }
        }
        
        async function removeEmail(id) {
            if (!confirm('Are you sure you want to remove this email from the whitelist?')) {
                return;
            }
            
            try {
                const response = await fetch(`/api/admin/whitelist/emails/${id}`, {
                    method: 'DELETE',
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                
                const data = await response.json();
                if (data.success) {
                    alert(data.message);
                    loadWhitelistData(); // Reload the list
                } else {
                    alert('Error: ' + data.message);
                }
            } catch (error) {
                console.error('Error removing email:', error);
                alert('Network error');
            }
        }
        
        // Add event listener to toggle
        document.getElementById('whitelistToggle').addEventListener('change', toggleWhitelist);
        
        // Allow Enter key to add email
        document.getElementById('newEmail').addEventListener('keypress', function(e) {
            if (e.key === 'Enter') {
                addEmail();
            }
        });

        // YouTube Feature Toggle
        document.getElementById('youtubeFeatureToggle').addEventListener('change', async (e) => {
            const enabled = e.target.checked;

            if (!confirm(`Are you sure you want to ${enabled ? 'enable' : 'disable'} YouTube features for ${enabled ? 'all users' : 'testing mode (admins + whitelist only)'}?`)) {
                e.target.checked = !enabled;
                return;
            }

            try {
                const response = await fetch('/api/admin/youtube/toggle', {
                    method: 'POST',
                    headers: {
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify({ enabled })
                });

                const data = await response.json();

                if (data.success) {
                    showNotification(data.message, 'success');
                    await loadYouTubeFeatureStatus();
                } else {
                    showNotification(data.message || 'Failed to toggle YouTube features', 'error');
                    e.target.checked = !enabled;
                }
            } catch (error) {
                showNotification('Network error', 'error');
                e.target.checked = !enabled;
            }
        });

        // Pricing Management Functions
        function showPricing() {
            document.getElementById('pricingSection').style.display = 'block';
            document.getElementById('whitelistSection').style.display = 'none';
            document.getElementById('youtubeSection').style.display = 'none';
            document.querySelector('.recent-section').style.display = 'none';
            loadPricingData();
        }

        function showYoutube() {
            document.getElementById('youtubeSection').style.display = 'block';
            document.getElementById('whitelistSection').style.display = 'none';
            document.getElementById('pricingSection').style.display = 'none';
            document.querySelector('.recent-section').style.display = 'none';
            loadYouTubeFeatureStatus();
        }

        async function loadYouTubeFeatureStatus() {
            try {
                const response = await fetch('/api/admin/youtube/status', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();

                if (data.success) {
                    const toggle = document.getElementById('youtubeFeatureToggle');
                    const status = document.getElementById('youtubeFeatureStatus');

                    toggle.checked = data.status.enabled;
                    status.textContent = data.status.enabled ? '✅ Enabled (All Users)' : '🔒 Disabled (Testing Mode)';
                    status.style.color = data.status.enabled ? '#28a745' : '#dc3545';
                }
            } catch (error) {
                console.error('Failed to load YouTube feature status:', error);
            }
        }

        async function loadPricingData() {
            try {
                const response = await fetch('/api/admin/pricing', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();

                if (data.success) {
                    const container = document.getElementById('pricingModels');
                    container.innerHTML = data.models.map(model => `
                        <div style="background: #f8f9fa; padding: 1.5rem; border-radius: 10px; margin-bottom: 1.5rem;">
                            <h3 style="margin-bottom: 1rem; color: #343a40;">${formatModelName(model.model)}</h3>
                            <form onsubmit="updatePricing(event, '${model.model}')" style="display: grid; grid-template-columns: 1fr 1fr; gap: 1rem;">
                                <div>
                                    <label style="display: block; margin-bottom: 0.5rem; font-weight: 600;">Input Price ($/1M tokens)</label>
                                    <input type="number" step="0.01" name="input_price" value="${model.input_base || model.input || 0}"
                                        style="width: 100%; padding: 0.5rem; border: 1px solid #ddd; border-radius: 5px;" required>
                                </div>
                                <div>
                                    <label style="display: block; margin-bottom: 0.5rem; font-weight: 600;">Output Price ($/1M tokens)</label>
                                    <input type="number" step="0.01" name="output_price" value="${model.output_base || model.output || 0}"
                                        style="width: 100%; padding: 0.5rem; border: 1px solid #ddd; border-radius: 5px;" required>
                                </div>
                                ${model.input_extended ? `
                                <div>
                                    <label style="display: block; margin-bottom: 0.5rem; font-weight: 600;">Input Extended (>200K) ($/1M)</label>
                                    <input type="number" step="0.01" name="input_extended" value="${model.input_extended || ''}"
                                        style="width: 100%; padding: 0.5rem; border: 1px solid #ddd; border-radius: 5px;">
                                </div>
                                <div>
                                    <label style="display: block; margin-bottom: 0.5rem; font-weight: 600;">Output Extended (>200K) ($/1M)</label>
                                    <input type="number" step="0.01" name="output_extended" value="${model.output_extended || ''}"
                                        style="width: 100%; padding: 0.5rem; border: 1px solid #ddd; border-radius: 5px;">
                                </div>
                                ` : ''}
                                <div style="grid-column: span 2;">
                                    <button type="submit" class="btn" style="width: auto;">Update Pricing</button>
                                    <small style="color: #6c757d; margin-left: 1rem;">Last updated: ${new Date(model.last_updated).toLocaleDateString()}</small>
                                </div>
                            </form>
                        </div>
                    `).join('');
                }
            } catch (error) {
                console.error('Error loading pricing data:', error);
                document.getElementById('pricingModels').innerHTML =
                    '<div style="text-align: center; padding: 2rem; color: #dc3545;">Error loading pricing data</div>';
            }
        }

        function formatModelName(model) {
            const names = {
                'claude-sonnet-4-5': 'Claude Sonnet 4.5',
                'claude-3-5-sonnet': 'Claude Sonnet 3.5',
                'gemini-2.0-flash': 'Gemini 2.0 Flash',
                'gemini-2.5-flash': 'Gemini 2.5 Flash'
            };
            return names[model] || model;
        }

        async function updatePricing(event, modelKey) {
            event.preventDefault();
            const form = event.target;
            const formData = new FormData(form);

            const payload = {
                model_key: modelKey,
                input_price: parseFloat(formData.get('input_price')),
                output_price: parseFloat(formData.get('output_price'))
            };

            if (formData.has('input_extended') && formData.get('input_extended')) {
                payload.input_price_extended = parseFloat(formData.get('input_extended'));
            }
            if (formData.has('output_extended') && formData.get('output_extended')) {
                payload.output_price_extended = parseFloat(formData.get('output_extended'));
            }

            try {
                const response = await fetch('/api/admin/pricing', {
                    method: 'POST',
                    headers: {
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify(payload)
                });

                const data = await response.json();
                if (data.success) {
                    alert('✅ ' + data.message);
                    loadPricingData(); // Reload to show updated timestamp
                } else {
                    alert('❌ Error: ' + data.message);
                }
            } catch (error) {
                console.error('Error updating pricing:', error);
                alert('❌ Network error');
            }
        }

        loadDashboardData();
    </script>
</body>
</html>
    "###;
    
    Html(html.to_string())
}

// API Endpoints
pub async fn admin_stats_api(Extension(state): Extension<Arc<AppState>>) -> Result<Json<serde_json::Value>, StatusCode> {
    let total_users = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
        .fetch_one(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    let active_users = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE is_active = true")
        .fetch_one(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    
    let total_chat_sessions = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM chat_sessions")
        .fetch_one(&state.db_pool)
        .await
        .unwrap_or(0);
    
    let total_files = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM uploaded_files")
        .fetch_one(&state.db_pool)
        .await
        .unwrap_or(0);
    
    Ok(Json(json!({
        "success": true,
        "stats": {
            "total_users": total_users,
            "active_users": active_users,
            "total_chat_sessions": total_chat_sessions,
            "total_files": total_files
        }
    })))
}

pub async fn admin_users_api(
    Query(params): Query<UsersQuery>,
    Extension(state): Extension<Arc<AppState>>
) -> Result<Json<serde_json::Value>, StatusCode> {
    let page = params.page.unwrap_or(1);
    let limit = params.limit.unwrap_or(20);
    let offset = (page - 1) * limit;
    
    let mut query = "SELECT id, email, username, is_active, is_superuser, is_staff, created_at, updated_at FROM users".to_string();
    let mut count_query = "SELECT COUNT(*) FROM users".to_string();
    
    if let Some(_search) = &params.search {
        let search_condition = " WHERE username ILIKE $1 OR email ILIKE $1";
        query.push_str(search_condition);
        count_query.push_str(search_condition);
    }
    
    query.push_str(&format!(" ORDER BY created_at DESC LIMIT {} OFFSET {}", limit, offset));
    
    let users: Vec<User> = if let Some(search) = &params.search {
        let search_term = format!("%{}%", search);
        sqlx::query_as(&query)
            .bind(&search_term)
            .fetch_all(&state.db_pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    } else {
        sqlx::query_as(&query)
            .fetch_all(&state.db_pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };
    
    let total_count: i64 = if let Some(search) = &params.search {
        let search_term = format!("%{}%", search);
        sqlx::query_scalar(&count_query)
            .bind(&search_term)
            .fetch_one(&state.db_pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    } else {
        sqlx::query_scalar(&count_query)
            .fetch_one(&state.db_pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };
    
    let user_responses: Vec<UserResponse> = users.into_iter().map(UserResponse::from).collect();
    
    Ok(Json(json!({
        "success": true,
        "users": user_responses,
        "pagination": {
            "page": page,
            "limit": limit,
            "total": total_count,
            "total_pages": (total_count as f64 / limit as f64).ceil() as u32
        }
    })))
}

pub async fn create_superuser_api(
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<CreateSuperuserRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorResponse>)> {
    // Validate input
    if payload.email.is_empty() || payload.username.is_empty() || payload.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                success: false,
                message: "Email, username, and password are required".to_string(),
            }),
        ));
    }

    if payload.password.len() < 6 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                success: false,
                message: "Password must be at least 6 characters long".to_string(),
            }),
        ));
    }

    // Check if user already exists
    let existing_user = sqlx::query("SELECT id FROM users WHERE email = $1 OR username = $2")
        .bind(&payload.email)
        .bind(&payload.username)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    success: false,
                    message: "Database error".to_string(),
                }),
            )
        })?;

    if existing_user.is_some() {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                success: false,
                message: "User with this email or username already exists".to_string(),
            }),
        ));
    }

    // Hash password
    let password_hash = hash(&payload.password, DEFAULT_COST).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                success: false,
                message: "Failed to hash password".to_string(),
            }),
        )
    })?;

    // Create superuser
    let user_row = sqlx::query(
        "INSERT INTO users (email, username, password_hash, is_active, is_superuser, is_staff) 
         VALUES ($1, $2, $3, true, true, true) 
         RETURNING id, email, username, is_active, is_superuser, is_staff, created_at, updated_at"
    )
    .bind(&payload.email)
    .bind(&payload.username)
    .bind(&password_hash)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                success: false,
                message: "Failed to create superuser".to_string(),
            }),
        )
    })?;

    let user = User {
        id: user_row.get("id"),
        email: user_row.get("email"),
        username: user_row.get("username"),
        password_hash: "".to_string(), // Don't include password hash in response
        is_active: user_row.get("is_active"),
        is_superuser: user_row.get("is_superuser"),
        is_staff: user_row.get("is_staff"),
        created_at: user_row.get("created_at"),
        updated_at: user_row.get("updated_at"),
    };

    Ok(Json(json!({
        "success": true,
        "message": "Superuser created successfully",
        "user": UserResponse::from(user)
    })))
}

// ============================================================================
// USER MANAGEMENT ENDPOINTS
// ============================================================================

// HTML PAGES

pub async fn admin_users_list() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>User Management - Admin Panel</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #f8f9fa; }
        .sidebar { width: 250px; background: #343a40; height: 100vh; position: fixed; left: 0; top: 0; color: white; padding: 1rem; overflow-y: auto; }
        .sidebar h2 { color: #dc3545; margin-bottom: 2rem; }
        .sidebar ul { list-style: none; }
        .sidebar li { margin-bottom: 0.5rem; }
        .sidebar a { color: #adb5bd; text-decoration: none; padding: 0.5rem; display: block; border-radius: 5px; }
        .sidebar a:hover { background: #495057; color: white; }
        .sidebar a.active { background: #dc3545; color: white; }
        .main-content { margin-left: 250px; padding: 2rem; }
        .header { background: white; padding: 1rem 2rem; margin-bottom: 2rem; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); display: flex; justify-content: space-between; align-items: center; }
        .search-bar { display: flex; gap: 1rem; margin-bottom: 1rem; }
        .search-bar input { flex: 1; padding: 0.75rem; border: 1px solid #ddd; border-radius: 5px; font-size: 1rem; }
        .btn { padding: 0.75rem 1.5rem; background: #dc3545; color: white; border: none; border-radius: 5px; cursor: pointer; font-weight: 600; text-decoration: none; display: inline-block; }
        .btn:hover { background: #c82333; }
        .btn-secondary { background: #6c757d; }
        .btn-secondary:hover { background: #5a6268; }
        .btn-sm { padding: 0.4rem 0.8rem; font-size: 0.875rem; }
        .users-section { background: white; padding: 2rem; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
        table { width: 100%; border-collapse: collapse; margin-top: 1rem; }
        th, td { padding: 0.75rem; text-align: left; border-bottom: 1px solid #dee2e6; }
        th { background: #f8f9fa; font-weight: 600; color: #495057; }
        .badge { padding: 0.25rem 0.6rem; border-radius: 3px; font-size: 0.75rem; font-weight: 600; }
        .badge-success { background: #d4edda; color: #155724; }
        .badge-danger { background: #f8d7da; color: #721c24; }
        .badge-warning { background: #fff3cd; color: #856404; }
        .pagination { display: flex; justify-content: space-between; align-items: center; margin-top: 1.5rem; }
        .pagination button { padding: 0.5rem 1rem; background: #6c757d; color: white; border: none; border-radius: 5px; cursor: pointer; }
        .pagination button:disabled { opacity: 0.5; cursor: not-allowed; }
        .modal { display: none; position: fixed; top: 0; left: 0; width: 100%; height: 100%; background: rgba(0,0,0,0.5); z-index: 1000; }
        .modal-content { background: white; max-width: 500px; margin: 5% auto; padding: 2rem; border-radius: 10px; box-shadow: 0 10px 30px rgba(0,0,0,0.3); }
        .modal-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.5rem; }
        .modal-header h2 { color: #343a40; }
        .close { font-size: 2rem; cursor: pointer; color: #6c757d; }
        .form-group { margin-bottom: 1.5rem; }
        .form-group label { display: block; margin-bottom: 0.5rem; font-weight: 600; color: #495057; }
        .form-group input { width: 100%; padding: 0.75rem; border: 1px solid #ddd; border-radius: 5px; font-size: 1rem; }
        .form-group input:focus { outline: none; border-color: #dc3545; }
        .checkbox-group { display: flex; align-items: center; gap: 0.5rem; }
        .checkbox-group input[type="checkbox"] { width: auto; }
        .error-message { background: #f8d7da; border: 1px solid #f5c6cb; padding: 1rem; border-radius: 5px; color: #721c24; margin-bottom: 1rem; display: none; }
    </style>
</head>
<body>
    <div class="sidebar">
        <h2>🛡️ Admin Panel</h2>
        <ul>
            <li><a href="/admin/dashboard">📊 Dashboard</a></li>
            <li><a href="/admin/users" class="active">👥 Users</a></li>
            <li><a href="/admin/clipping-activity">🎬 Clipping Activity</a></li>
            <li><a href="/api/docs">📚 API Docs</a></li>
            <li><a href="/api/status">⚙️ System Status</a></li>
        </ul>
        <div style="position: absolute; bottom: 1rem;">
            <button onclick="logout()" class="btn btn-secondary">Logout</button>
        </div>
    </div>

    <div class="main-content">
        <div class="header">
            <div>
                <h1>User Management</h1>
                <p>Manage user accounts, roles, and permissions</p>
            </div>
            <button onclick="openCreateModal()" class="btn">+ Create User</button>
        </div>

        <div class="search-bar">
            <input type="text" id="searchInput" placeholder="Search by username or email..." onkeyup="searchUsers()">
        </div>

        <div class="users-section">
            <h2>All Users</h2>
            <table>
                <thead>
                    <tr>
                        <th>ID</th>
                        <th>Username</th>
                        <th>Email</th>
                        <th>Status</th>
                        <th>Role</th>
                        <th>Created</th>
                        <th>Actions</th>
                    </tr>
                </thead>
                <tbody id="usersTable">
                    <tr><td colspan="7" style="text-align: center;">Loading...</td></tr>
                </tbody>
            </table>

            <div class="pagination">
                <button onclick="prevPage()" id="prevBtn" disabled>← Previous</button>
                <span id="pageInfo">Page 1</span>
                <button onclick="nextPage()" id="nextBtn">Next →</button>
            </div>
        </div>
    </div>

    <!-- Create User Modal -->
    <div id="createModal" class="modal">
        <div class="modal-content">
            <div class="modal-header">
                <h2>Create New User</h2>
                <span class="close" onclick="closeCreateModal()">&times;</span>
            </div>

            <div id="createError" class="error-message"></div>

            <form id="createUserForm" onsubmit="createUser(event)">
                <div class="form-group">
                    <label for="createEmail">Email *</label>
                    <input type="email" id="createEmail" required>
                </div>

                <div class="form-group">
                    <label for="createUsername">Username *</label>
                    <input type="text" id="createUsername" required>
                </div>

                <div class="form-group">
                    <label for="createPassword">Password * (min 6 characters)</label>
                    <input type="password" id="createPassword" minlength="6" required>
                </div>

                <div class="form-group checkbox-group">
                    <input type="checkbox" id="createIsStaff">
                    <label for="createIsStaff" style="margin: 0;">Make this user a staff member</label>
                </div>

                <button type="submit" class="btn" style="width: 100%;">Create User</button>
            </form>
        </div>
    </div>

    <script>
        const authToken = localStorage.getItem('authToken');
        const user = JSON.parse(localStorage.getItem('user') || '{}');

        if (!authToken || (!user.is_staff && !user.is_superuser)) {
            window.location.href = '/admin/login';
        }

        let currentPage = 1;
        let totalPages = 1;
        let searchTerm = '';

        async function loadUsers() {
            try {
                const params = new URLSearchParams({
                    page: currentPage,
                    limit: 20
                });

                if (searchTerm) {
                    params.append('search', searchTerm);
                }

                const response = await fetch(`/api/admin/users?${params}`, {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });

                const data = await response.json();

                if (data.success) {
                    renderUsers(data.users);
                    totalPages = data.pagination.total_pages;
                    updatePagination();
                }
            } catch (error) {
                console.error('Error loading users:', error);
            }
        }

        function renderUsers(users) {
            const tbody = document.getElementById('usersTable');

            if (users.length === 0) {
                tbody.innerHTML = '<tr><td colspan="7" style="text-align: center;">No users found</td></tr>';
                return;
            }

            tbody.innerHTML = users.map(user => `
                <tr>
                    <td>${user.id}</td>
                    <td>${user.username}</td>
                    <td>${user.email}</td>
                    <td><span class="badge ${user.is_active ? 'badge-success' : 'badge-danger'}">${user.is_active ? 'Active' : 'Inactive'}</span></td>
                    <td><span class="badge ${user.is_superuser ? 'badge-danger' : user.is_staff ? 'badge-warning' : 'badge-success'}">${user.is_superuser ? 'Superuser' : user.is_staff ? 'Staff' : 'User'}</span></td>
                    <td>${new Date(user.created_at).toLocaleDateString()}</td>
                    <td><a href="/admin/users/${user.id}" class="btn btn-sm">View</a></td>
                </tr>
            `).join('');
        }

        function updatePagination() {
            document.getElementById('pageInfo').textContent = `Page ${currentPage} of ${totalPages}`;
            document.getElementById('prevBtn').disabled = currentPage === 1;
            document.getElementById('nextBtn').disabled = currentPage >= totalPages;
        }

        function prevPage() {
            if (currentPage > 1) {
                currentPage--;
                loadUsers();
            }
        }

        function nextPage() {
            if (currentPage < totalPages) {
                currentPage++;
                loadUsers();
            }
        }

        let searchTimeout;
        function searchUsers() {
            clearTimeout(searchTimeout);
            searchTerm = document.getElementById('searchInput').value.trim();
            searchTimeout = setTimeout(() => {
                currentPage = 1;
                loadUsers();
            }, 300);
        }

        function openCreateModal() {
            document.getElementById('createModal').style.display = 'block';
        }

        function closeCreateModal() {
            document.getElementById('createModal').style.display = 'none';
            document.getElementById('createUserForm').reset();
            document.getElementById('createError').style.display = 'none';
        }

        async function createUser(event) {
            event.preventDefault();

            const email = document.getElementById('createEmail').value;
            const username = document.getElementById('createUsername').value;
            const password = document.getElementById('createPassword').value;
            const is_staff = document.getElementById('createIsStaff').checked;

            try {
                const response = await fetch('/admin/users', {
                    method: 'POST',
                    headers: {
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify({ email, username, password, is_staff })
                });

                const data = await response.json();

                if (data.success) {
                    closeCreateModal();
                    loadUsers();
                    alert('✅ ' + data.message);
                } else {
                    document.getElementById('createError').textContent = data.message;
                    document.getElementById('createError').style.display = 'block';
                }
            } catch (error) {
                document.getElementById('createError').textContent = 'Network error. Please try again.';
                document.getElementById('createError').style.display = 'block';
            }
        }

        function logout() {
            localStorage.removeItem('authToken');
            localStorage.removeItem('user');
            window.location.href = '/admin/login';
        }

        loadUsers();
    </script>
</body>
</html>
    "###;

    Html(html.to_string())
}

pub async fn admin_user_detail(Path(id): Path<i32>) -> Html<String> {
    let html = format!(r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>User Details - Admin Panel</title>
    <style>
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #f8f9fa; }}
        .sidebar {{ width: 250px; background: #343a40; height: 100vh; position: fixed; left: 0; top: 0; color: white; padding: 1rem; }}
        .sidebar h2 {{ color: #dc3545; margin-bottom: 2rem; }}
        .sidebar ul {{ list-style: none; }}
        .sidebar li {{ margin-bottom: 0.5rem; }}
        .sidebar a {{ color: #adb5bd; text-decoration: none; padding: 0.5rem; display: block; border-radius: 5px; }}
        .sidebar a:hover {{ background: #495057; color: white; }}
        .sidebar a.active {{ background: #dc3545; color: white; }}
        .main-content {{ margin-left: 250px; padding: 2rem; }}
        .header {{ background: white; padding: 1rem 2rem; margin-bottom: 2rem; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }}
        .card {{ background: white; padding: 2rem; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); margin-bottom: 2rem; }}
        .card h2 {{ margin-bottom: 1rem; color: #343a40; }}
        .info-grid {{ display: grid; grid-template-columns: 200px 1fr; gap: 1rem; margin-bottom: 1rem; }}
        .info-label {{ font-weight: 600; color: #6c757d; }}
        .info-value {{ color: #343a40; }}
        .form-group {{ margin-bottom: 1.5rem; }}
        .form-group label {{ display: block; margin-bottom: 0.5rem; font-weight: 600; color: #495057; }}
        .form-group input {{ width: 100%; padding: 0.75rem; border: 1px solid #ddd; border-radius: 5px; font-size: 1rem; }}
        .form-group input:focus {{ outline: none; border-color: #dc3545; }}
        .btn {{ padding: 0.75rem 1.5rem; background: #dc3545; color: white; border: none; border-radius: 5px; cursor: pointer; font-weight: 600; text-decoration: none; display: inline-block; }}
        .btn:hover {{ background: #c82333; }}
        .btn-secondary {{ background: #6c757d; }}
        .btn-secondary:hover {{ background: #5a6268; }}
        .btn-success {{ background: #28a745; }}
        .btn-success:hover {{ background: #218838; }}
        .btn-warning {{ background: #ffc107; color: #000; }}
        .btn-warning:hover {{ background: #e0a800; }}
        .btn:disabled {{ opacity: 0.5; cursor: not-allowed; }}
        .danger-zone {{ border: 2px solid #dc3545; border-radius: 10px; padding: 2rem; margin-top: 2rem; }}
        .danger-zone h3 {{ color: #dc3545; margin-bottom: 1rem; }}
        .badge {{ padding: 0.25rem 0.6rem; border-radius: 3px; font-size: 0.875rem; font-weight: 600; }}
        .badge-success {{ background: #d4edda; color: #155724; }}
        .badge-danger {{ background: #f8d7da; color: #721c24; }}
        .badge-warning {{ background: #fff3cd; color: #856404; }}
        .error-message {{ background: #f8d7da; border: 1px solid #f5c6cb; padding: 1rem; border-radius: 5px; color: #721c24; margin-bottom: 1rem; display: none; }}
        .success-message {{ background: #d4edda; border: 1px solid #c3e6cb; padding: 1rem; border-radius: 5px; color: #155724; margin-bottom: 1rem; display: none; }}
    </style>
</head>
<body>
    <div class="sidebar">
        <h2>🛡️ Admin Panel</h2>
        <ul>
            <li><a href="/admin/dashboard">📊 Dashboard</a></li>
            <li><a href="/admin/users" class="active">👥 Users</a></li>
            <li><a href="/admin/clipping-activity">🎬 Clipping Activity</a></li>
            <li><a href="/api/docs">📚 API Docs</a></li>
            <li><a href="/api/status">⚙️ System Status</a></li>
        </ul>
        <div style="position: absolute; bottom: 1rem;">
            <button onclick="logout()" class="btn btn-secondary">Logout</button>
        </div>
    </div>

    <div class="main-content">
        <div class="header">
            <a href="/admin/users" style="color: #6c757d; text-decoration: none; margin-bottom: 0.5rem; display: inline-block;">← Back to Users</a>
            <h1>User Details</h1>
        </div>

        <div id="errorMessage" class="error-message"></div>
        <div id="successMessage" class="success-message"></div>

        <div class="card">
            <h2>User Information</h2>
            <div class="info-grid">
                <div class="info-label">User ID:</div>
                <div class="info-value" id="userId">Loading...</div>

                <div class="info-label">Username:</div>
                <div class="info-value" id="username">Loading...</div>

                <div class="info-label">Email:</div>
                <div class="info-value" id="email">Loading...</div>

                <div class="info-label">Status:</div>
                <div class="info-value" id="status">Loading...</div>

                <div class="info-label">Role:</div>
                <div class="info-value" id="role">Loading...</div>

                <div class="info-label">Created:</div>
                <div class="info-value" id="created">Loading...</div>

                <div class="info-label">Last Updated:</div>
                <div class="info-value" id="updated">Loading...</div>
            </div>
        </div>

        <div class="card">
            <h2>Edit User</h2>
            <form onsubmit="updateUser(event)">
                <div class="form-group">
                    <label for="editEmail">Email</label>
                    <input type="email" id="editEmail" required>
                </div>

                <div class="form-group">
                    <label for="editUsername">Username</label>
                    <input type="text" id="editUsername" required>
                </div>

                <button type="submit" class="btn">Save Changes</button>
            </form>
        </div>

        <div class="card">
            <h2>Account Status</h2>
            <p style="color: #6c757d; margin-bottom: 1rem;">Enable or disable this user's account access.</p>
            <button onclick="toggleActive()" id="toggleActiveBtn" class="btn btn-warning">Loading...</button>
        </div>

        <div class="card">
            <h2>Staff Access</h2>
            <p style="color: #6c757d; margin-bottom: 1rem;">Grant or revoke staff privileges for this user.</p>
            <button onclick="toggleStaff()" id="toggleStaffBtn" class="btn btn-warning">Loading...</button>
        </div>

        <div class="card" id="superuserSection" style="display: none;">
            <h2>Superuser Access</h2>
            <p style="color: #6c757d; margin-bottom: 1rem;">Grant or revoke superuser privileges. Only superusers can modify this.</p>
            <button onclick="toggleSuperuser()" id="toggleSuperuserBtn" class="btn btn-warning">Loading...</button>
        </div>

        <div class="danger-zone">
            <h3>⚠️ Danger Zone</h3>
            <p style="color: #6c757d; margin-bottom: 1rem;">Permanently delete this user account. This action cannot be undone.</p>
            <button onclick="deleteUser()" id="deleteBtn" class="btn">Delete User</button>
        </div>
    </div>

    <script>
        const authToken = localStorage.getItem('authToken');
        const currentUser = JSON.parse(localStorage.getItem('user') || '{{}}');
        const userId = {id};
        let userData = null;

        if (!authToken || (!currentUser.is_staff && !currentUser.is_superuser)) {{
            window.location.href = '/admin/login';
        }}

        async function loadUser() {{
            try {{
                const response = await fetch(`/api/admin/users/${{userId}}`, {{
                    headers: {{ 'Authorization': 'Bearer ' + authToken }}
                }});

                const data = await response.json();

                if (data.success) {{
                    userData = data.user;
                    renderUser(userData);
                    updateButtons(userData);
                }} else {{
                    showError(data.message);
                }}
            }} catch (error) {{
                showError('Failed to load user data');
            }}
        }}

        function renderUser(user) {{
            document.getElementById('userId').textContent = user.id;
            document.getElementById('username').textContent = user.username;
            document.getElementById('email').textContent = user.email;
            document.getElementById('status').innerHTML = `<span class="badge ${{user.is_active ? 'badge-success' : 'badge-danger'}}">${{user.is_active ? 'Active' : 'Inactive'}}</span>`;
            document.getElementById('role').innerHTML = `<span class="badge ${{user.is_superuser ? 'badge-danger' : user.is_staff ? 'badge-warning' : 'badge-success'}}">${{user.is_superuser ? 'Superuser' : user.is_staff ? 'Staff' : 'User'}}</span>`;
            document.getElementById('created').textContent = new Date(user.created_at).toLocaleString();
            document.getElementById('updated').textContent = new Date(user.updated_at).toLocaleString();

            document.getElementById('editEmail').value = user.email;
            document.getElementById('editUsername').value = user.username;
        }}

        function updateButtons(user) {{
            const isSelf = currentUser.id === user.id;

            // Toggle Active button
            const activeBtn = document.getElementById('toggleActiveBtn');
            activeBtn.textContent = user.is_active ? 'Deactivate Account' : 'Activate Account';
            activeBtn.className = user.is_active ? 'btn' : 'btn btn-success';
            activeBtn.disabled = isSelf;

            // Toggle Staff button
            const staffBtn = document.getElementById('toggleStaffBtn');
            staffBtn.textContent = user.is_staff ? 'Remove Staff Access' : 'Grant Staff Access';
            staffBtn.className = user.is_staff ? 'btn' : 'btn btn-success';
            staffBtn.disabled = isSelf;

            // Superuser section (only visible to superusers)
            if (currentUser.is_superuser) {{
                document.getElementById('superuserSection').style.display = 'block';
                const superuserBtn = document.getElementById('toggleSuperuserBtn');
                superuserBtn.textContent = user.is_superuser ? 'Remove Superuser Access' : 'Grant Superuser Access';
                superuserBtn.className = user.is_superuser ? 'btn' : 'btn btn-success';
                superuserBtn.disabled = isSelf;
            }}

            // Delete button
            document.getElementById('deleteBtn').disabled = isSelf;
        }}

        async function updateUser(event) {{
            event.preventDefault();

            const email = document.getElementById('editEmail').value;
            const username = document.getElementById('editUsername').value;

            try {{
                const response = await fetch(`/api/admin/users/${{userId}}`, {{
                    method: 'PUT',
                    headers: {{
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    }},
                    body: JSON.stringify({{ email, username }})
                }});

                const data = await response.json();

                if (data.success) {{
                    showSuccess(data.message);
                    loadUser();
                }} else {{
                    showError(data.message);
                }}
            }} catch (error) {{
                showError('Network error');
            }}
        }}

        async function toggleActive() {{
            if (!userData) return;

            const newStatus = !userData.is_active;
            const action = newStatus ? 'activate' : 'deactivate';

            if (!confirm(`Are you sure you want to ${{action}} this user?`)) return;

            try {{
                const response = await fetch(`/api/admin/users/${{userId}}/toggle-active`, {{
                    method: 'POST',
                    headers: {{
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    }},
                    body: JSON.stringify({{ is_active: newStatus }})
                }});

                const data = await response.json();

                if (data.success) {{
                    showSuccess(data.message);
                    loadUser();
                }} else {{
                    showError(data.message);
                }}
            }} catch (error) {{
                showError('Network error');
            }}
        }}

        async function toggleStaff() {{
            if (!userData) return;

            const endpoint = userData.is_staff ? 'remove-staff' : 'make-staff';
            const action = userData.is_staff ? 'remove staff access from' : 'grant staff access to';

            if (!confirm(`Are you sure you want to ${{action}} this user?`)) return;

            try {{
                const response = await fetch(`/api/admin/users/${{userId}}/${{endpoint}}`, {{
                    method: 'POST',
                    headers: {{ 'Authorization': 'Bearer ' + authToken }}
                }});

                const data = await response.json();

                if (data.success) {{
                    showSuccess(data.message);
                    loadUser();
                }} else {{
                    showError(data.message);
                }}
            }} catch (error) {{
                showError('Network error');
            }}
        }}

        async function toggleSuperuser() {{
            if (!userData) return;

            const endpoint = userData.is_superuser ? 'remove-superuser' : 'make-superuser';
            const action = userData.is_superuser ? 'remove superuser access from' : 'grant superuser access to';

            if (!confirm(`Are you sure you want to ${{action}} this user? This is a critical security permission.`)) return;

            try {{
                const response = await fetch(`/api/admin/users/${{userId}}/${{endpoint}}`, {{
                    method: 'POST',
                    headers: {{ 'Authorization': 'Bearer ' + authToken }}
                }});

                const data = await response.json();

                if (data.success) {{
                    showSuccess(data.message);
                    loadUser();
                }} else {{
                    showError(data.message);
                }}
            }} catch (error) {{
                showError('Network error');
            }}
        }}

        async function deleteUser() {{
            if (!userData) return;

            const confirmation = prompt(`Type the username "${{userData.username}}" to confirm deletion:`);
            if (confirmation !== userData.username) {{
                showError('Username does not match. Deletion cancelled.');
                return;
            }}

            try {{
                const response = await fetch(`/api/admin/users/${{userId}}`, {{
                    method: 'DELETE',
                    headers: {{ 'Authorization': 'Bearer ' + authToken }}
                }});

                const data = await response.json();

                if (data.success) {{
                    alert('✅ User deleted successfully');
                    window.location.href = '/admin/users';
                }} else {{
                    showError(data.message);
                }}
            }} catch (error) {{
                showError('Network error');
            }}
        }}

        function showError(message) {{
            const el = document.getElementById('errorMessage');
            el.textContent = message;
            el.style.display = 'block';
            document.getElementById('successMessage').style.display = 'none';
            window.scrollTo(0, 0);
        }}

        function showSuccess(message) {{
            const el = document.getElementById('successMessage');
            el.textContent = message;
            el.style.display = 'block';
            document.getElementById('errorMessage').style.display = 'none';
            window.scrollTo(0, 0);
        }}

        function logout() {{
            localStorage.removeItem('authToken');
            localStorage.removeItem('user');
            window.location.href = '/admin/login';
        }}

        loadUser();
    </script>
</body>
</html>
    "###, id = id);

    Html(html)
}

// API ENDPOINTS

pub async fn admin_user_api(
    Path(id): Path<i32>,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("❌ Database error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Database error"
            }))
        )
    })?;

    match user {
        Some(user) => {
            tracing::info!("👤 Admin fetched user details: {}", user.username);
            Ok(Json(json!({
                "success": true,
                "user": UserResponse::from(user)
            })))
        }
        None => {
            Err((
                StatusCode::NOT_FOUND,
                Json(json!({
                    "success": false,
                    "message": "User not found"
                }))
            ))
        }
    }
}

pub async fn admin_update_user_api(
    Path(id): Path<i32>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<UpdateUserRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Validate email format if provided
    if let Some(ref email) = payload.email {
        if email.is_empty() || !email.contains('@') {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "Invalid email format"
                }))
            ));
        }
    }

    // Get current user data first
    let current_user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|e| {
            tracing::error!("❌ Database error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "message": "Database error"
                }))
            )
        })?;

    let current_user = current_user.ok_or((
        StatusCode::NOT_FOUND,
        Json(json!({
            "success": false,
            "message": "User not found"
        }))
    ))?;

    // Check uniqueness if email or username is being changed
    let email_to_check = payload.email.as_ref().unwrap_or(&current_user.email);
    let username_to_check = payload.username.as_ref().unwrap_or(&current_user.username);

    // Only check if values are actually changing
    if payload.email.is_some() || payload.username.is_some() {
        let existing = sqlx::query(
            "SELECT id FROM users WHERE (email = $1 OR username = $2) AND id != $3"
        )
        .bind(email_to_check)
        .bind(username_to_check)
        .bind(id)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|e| {
            tracing::error!("❌ Database error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "message": "Database error"
                }))
            )
        })?;

        if existing.is_some() {
            return Err((
                StatusCode::CONFLICT,
                Json(json!({
                    "success": false,
                    "message": "Email or username already exists"
                }))
            ));
        }
    }

    // Update user
    let email = payload.email.unwrap_or(current_user.email);
    let username = payload.username.unwrap_or(current_user.username);

    let updated_user = sqlx::query_as::<_, User>(
        "UPDATE users SET email = $1, username = $2, updated_at = NOW() WHERE id = $3 RETURNING *"
    )
    .bind(&email)
    .bind(&username)
    .bind(id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("❌ Database error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Failed to update user"
            }))
        )
    })?;

    tracing::info!("👤 User {} updated by admin {}", username, claims.username);

    Ok(Json(json!({
        "success": true,
        "message": "User updated successfully",
        "user": UserResponse::from(updated_user)
    })))
}

pub async fn admin_toggle_user_active(
    Path(id): Path<i32>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<ToggleActiveRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let current_user_id = claims.sub.parse::<i32>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Invalid user ID"
            }))
        )
    })?;

    // Self-protection
    if id == current_user_id {
        tracing::warn!("⚠️ Attempt to modify own account status by {}", claims.username);
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Cannot modify your own account status"
            }))
        ));
    }

    // If deactivating, check if user is a superuser and last one
    if !payload.is_active {
        let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db_pool)
            .await
            .map_err(|e| {
                tracing::error!("❌ Database error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "success": false,
                        "message": "Database error"
                    }))
                )
            })?;

        if let Some(user) = user {
            if user.is_superuser {
                let superuser_count: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM users WHERE is_superuser = true AND is_active = true"
                )
                .fetch_one(&state.db_pool)
                .await
                .map_err(|e| {
                    tracing::error!("❌ Database error: {}", e);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "success": false,
                            "message": "Database error"
                        }))
                    )
                })?;

                if superuser_count <= 1 {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "success": false,
                            "message": "Cannot deactivate the last active superuser"
                        }))
                    ));
                }
            }
        }
    }

    let updated_user = sqlx::query_as::<_, User>(
        "UPDATE users SET is_active = $1, updated_at = NOW() WHERE id = $2 RETURNING *"
    )
    .bind(payload.is_active)
    .bind(id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("❌ Database error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Failed to update user status"
            }))
        )
    })?;

    tracing::info!(
        "👤 User {} {} by admin {}",
        updated_user.username,
        if payload.is_active { "activated" } else { "deactivated" },
        claims.username
    );

    Ok(Json(json!({
        "success": true,
        "message": format!("User {} successfully", if payload.is_active { "activated" } else { "deactivated" }),
        "user": UserResponse::from(updated_user)
    })))
}

pub async fn admin_make_staff(
    Path(id): Path<i32>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|e| {
            tracing::error!("❌ Database error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "message": "Database error"
                }))
            )
        })?;

    let user = user.ok_or((
        StatusCode::NOT_FOUND,
        Json(json!({
            "success": false,
            "message": "User not found"
        }))
    ))?;

    if user.is_staff {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "User is already a staff member"
            }))
        ));
    }

    let updated_user = sqlx::query_as::<_, User>(
        "UPDATE users SET is_staff = true, updated_at = NOW() WHERE id = $1 RETURNING *"
    )
    .bind(id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("❌ Database error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Failed to update user"
            }))
        )
    })?;

    tracing::info!("👤 User {} granted staff access by admin {}", updated_user.username, claims.username);

    Ok(Json(json!({
        "success": true,
        "message": "Staff access granted successfully",
        "user": UserResponse::from(updated_user)
    })))
}

pub async fn admin_remove_staff(
    Path(id): Path<i32>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let current_user_id = claims.sub.parse::<i32>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Invalid user ID"
            }))
        )
    })?;

    // Self-protection
    if id == current_user_id {
        tracing::warn!("⚠️ Attempt to remove own staff status by {}", claims.username);
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Cannot remove your own staff status"
            }))
        ));
    }

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|e| {
            tracing::error!("❌ Database error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "message": "Database error"
                }))
            )
        })?;

    let user = user.ok_or((
        StatusCode::NOT_FOUND,
        Json(json!({
            "success": false,
            "message": "User not found"
        }))
    ))?;

    if !user.is_staff {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "User is not a staff member"
            }))
        ));
    }

    let updated_user = sqlx::query_as::<_, User>(
        "UPDATE users SET is_staff = false, updated_at = NOW() WHERE id = $1 RETURNING *"
    )
    .bind(id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("❌ Database error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Failed to update user"
            }))
        )
    })?;

    tracing::info!("👤 User {} staff access removed by admin {}", updated_user.username, claims.username);

    Ok(Json(json!({
        "success": true,
        "message": "Staff access removed successfully",
        "user": UserResponse::from(updated_user)
    })))
}

pub async fn admin_make_superuser(
    Path(id): Path<i32>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|e| {
            tracing::error!("❌ Database error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "message": "Database error"
                }))
            )
        })?;

    let user = user.ok_or((
        StatusCode::NOT_FOUND,
        Json(json!({
            "success": false,
            "message": "User not found"
        }))
    ))?;

    if user.is_superuser {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "User is already a superuser"
            }))
        ));
    }

    let updated_user = sqlx::query_as::<_, User>(
        "UPDATE users SET is_superuser = true, is_staff = true, updated_at = NOW() WHERE id = $1 RETURNING *"
    )
    .bind(id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("❌ Database error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Failed to update user"
            }))
        )
    })?;

    tracing::info!("🔐 User {} granted superuser access by {}", updated_user.username, claims.username);

    Ok(Json(json!({
        "success": true,
        "message": "Superuser access granted successfully",
        "user": UserResponse::from(updated_user)
    })))
}

pub async fn admin_remove_superuser(
    Path(id): Path<i32>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let current_user_id = claims.sub.parse::<i32>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Invalid user ID"
            }))
        )
    })?;

    // Self-protection
    if id == current_user_id {
        tracing::warn!("⚠️ Attempt to remove own superuser status by {}", claims.username);
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Cannot remove your own superuser status"
            }))
        ));
    }

    // Check if this is the last superuser
    let superuser_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE is_superuser = true AND is_active = true"
    )
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("❌ Database error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Database error"
            }))
        )
    })?;

    if superuser_count <= 1 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Cannot remove the last superuser"
            }))
        ));
    }

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|e| {
            tracing::error!("❌ Database error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "message": "Database error"
                }))
            )
        })?;

    let user = user.ok_or((
        StatusCode::NOT_FOUND,
        Json(json!({
            "success": false,
            "message": "User not found"
        }))
    ))?;

    if !user.is_superuser {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "User is not a superuser"
            }))
        ));
    }

    let updated_user = sqlx::query_as::<_, User>(
        "UPDATE users SET is_superuser = false, updated_at = NOW() WHERE id = $1 RETURNING *"
    )
    .bind(id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("❌ Database error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Failed to update user"
            }))
        )
    })?;

    tracing::info!("🔐 User {} superuser access removed by {}", updated_user.username, claims.username);

    Ok(Json(json!({
        "success": true,
        "message": "Superuser access removed successfully",
        "user": UserResponse::from(updated_user)
    })))
}

pub async fn admin_delete_user(
    Path(id): Path<i32>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let current_user_id = claims.sub.parse::<i32>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Invalid user ID"
            }))
        )
    })?;

    // Self-protection
    if id == current_user_id {
        tracing::warn!("⚠️ Attempt to delete own account by {}", claims.username);
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Cannot delete your own account"
            }))
        ));
    }

    // Check if user is last superuser
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|e| {
            tracing::error!("❌ Database error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "message": "Database error"
                }))
            )
        })?;

    let user = user.ok_or((
        StatusCode::NOT_FOUND,
        Json(json!({
            "success": false,
            "message": "User not found"
        }))
    ))?;

    if user.is_superuser {
        let superuser_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE is_superuser = true AND is_active = true"
        )
        .fetch_one(&state.db_pool)
        .await
        .map_err(|e| {
            tracing::error!("❌ Database error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "message": "Database error"
                }))
            )
        })?;

        if superuser_count <= 1 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "success": false,
                    "message": "Cannot delete the last superuser"
                }))
            ));
        }
    }

    let username = user.username.clone();

    // Delete user (CASCADE will handle related data)
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(&state.db_pool)
        .await
        .map_err(|e| {
            tracing::error!("❌ Database error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "message": "Failed to delete user"
                }))
            )
        })?;

    tracing::info!("🗑️ User {} deleted by admin {}", username, claims.username);

    Ok(Json(json!({
        "success": true,
        "message": "User deleted successfully"
    })))
}

pub async fn admin_create_user(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<CreateUserRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Validate input
    if payload.email.is_empty() || payload.username.is_empty() || payload.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Email, username, and password are required"
            }))
        ));
    }

    if !payload.email.contains('@') {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Invalid email format"
            }))
        ));
    }

    if payload.password.len() < 6 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Password must be at least 6 characters long"
            }))
        ));
    }

    // Check uniqueness
    let existing = sqlx::query("SELECT id FROM users WHERE email = $1 OR username = $2")
        .bind(&payload.email)
        .bind(&payload.username)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|e| {
            tracing::error!("❌ Database error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "success": false,
                    "message": "Database error"
                }))
            )
        })?;

    if existing.is_some() {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({
                "success": false,
                "message": "User with this email or username already exists"
            }))
        ));
    }

    // Hash password
    let password_hash = hash(&payload.password, DEFAULT_COST).map_err(|e| {
        tracing::error!("❌ Password hashing error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Failed to hash password"
            }))
        )
    })?;

    let is_staff = payload.is_staff.unwrap_or(false);

    // Create user
    let user = sqlx::query_as::<_, User>(
        "INSERT INTO users (email, username, password_hash, is_active, is_staff, is_superuser, created_at, updated_at)
         VALUES ($1, $2, $3, true, $4, false, NOW(), NOW())
         RETURNING *"
    )
    .bind(&payload.email)
    .bind(&payload.username)
    .bind(&password_hash)
    .bind(is_staff)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("❌ Database error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Failed to create user"
            }))
        )
    })?;

    tracing::info!("👤 User {} created by admin {}", user.username, claims.username);

    Ok(Json(json!({
        "success": true,
        "message": "User created successfully",
        "user": UserResponse::from(user)
    })))
}

pub async fn admin_update_user(
    Path(id): Path<i32>,
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<UpdateUserRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // This is the form submission version (not API)
    // Reuse the same logic as admin_update_user_api
    admin_update_user_api(Path(id), Extension(state), Extension(claims), Json(payload)).await
}

// Whitelist Management Functions
pub async fn get_whitelist_status(
    Extension(state): Extension<Arc<AppState>>
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Get whitelist enabled status
    let setting = sqlx::query_as::<_, SystemSetting>(
        "SELECT * FROM system_settings WHERE setting_key = 'whitelist_enabled'"
    )
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let enabled = setting
        .map(|s| s.as_bool().unwrap_or(false))
        .unwrap_or(false);

    // Get total whitelist emails count
    let total_emails = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM whitelist_emails")
        .fetch_one(&state.db_pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({
        "success": true,
        "status": {
            "enabled": enabled,
            "total_emails": total_emails
        }
    })))
}

pub async fn toggle_whitelist(
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<WhitelistToggleRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let setting_value = if payload.enabled { "true" } else { "false" };
    
    // Update or insert the whitelist_enabled setting
    sqlx::query(
        "INSERT INTO system_settings (setting_key, setting_value, setting_type, description, updated_at) 
         VALUES ('whitelist_enabled', $1, 'boolean', 'Enable email whitelist restriction for user registration and login', NOW())
         ON CONFLICT (setting_key) 
         DO UPDATE SET setting_value = $1, updated_at = NOW()"
    )
    .bind(setting_value)
    .execute(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({
        "success": true,
        "message": format!("Whitelist {}", if payload.enabled { "enabled" } else { "disabled" }),
        "enabled": payload.enabled
    })))
}

pub async fn get_whitelist_emails(
    Extension(state): Extension<Arc<AppState>>
) -> Result<Json<serde_json::Value>, StatusCode> {
    let emails = sqlx::query_as::<_, WhitelistEmail>(
        "SELECT id, email, added_by, created_at, updated_at FROM whitelist_emails ORDER BY created_at DESC"
    )
    .fetch_all(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let email_responses: Vec<WhitelistEmailResponse> = emails.into_iter()
        .map(WhitelistEmailResponse::from)
        .collect();

    Ok(Json(json!({
        "success": true,
        "emails": email_responses
    })))
}

pub async fn add_whitelist_email(
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<WhitelistEmailRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Validate email format
    if payload.email.is_empty() || !payload.email.contains('@') {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Invalid email format"
            }))
        ));
    }

    // Check if email already exists in whitelist
    let existing = sqlx::query("SELECT id FROM whitelist_emails WHERE email = $1")
        .bind(&payload.email)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|_| (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Database error"
            }))
        ))?;

    if existing.is_some() {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({
                "success": false,
                "message": "Email already exists in whitelist"
            }))
        ));
    }

    // Insert new whitelist email
    let row = sqlx::query(
        "INSERT INTO whitelist_emails (email, created_at, updated_at) 
         VALUES ($1, NOW(), NOW()) 
         RETURNING id, email, added_by, created_at, updated_at"
    )
    .bind(&payload.email)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|_| (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "success": false,
            "message": "Failed to add email to whitelist"
        }))
    ))?;

    let whitelist_email = WhitelistEmail::from_row(&row).map_err(|_| (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "success": false,
            "message": "Database error"
        }))
    ))?;

    Ok(Json(json!({
        "success": true,
        "message": "Email added to whitelist successfully",
        "email": WhitelistEmailResponse::from(whitelist_email)
    })))
}

pub async fn remove_whitelist_email(
    Path(id): Path<i32>,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let result = sqlx::query("DELETE FROM whitelist_emails WHERE id = $1")
        .bind(id)
        .execute(&state.db_pool)
        .await
        .map_err(|_| (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Database error"
            }))
        ))?;

    if result.rows_affected() == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({
                "success": false,
                "message": "Email not found in whitelist"
            }))
        ));
    }

    Ok(Json(json!({
        "success": true,
        "message": "Email removed from whitelist successfully"
    })))
}

// ============================================================================
// MODEL PRICING MANAGEMENT
// ============================================================================

#[derive(Deserialize)]
pub struct UpdatePricingRequest {
    pub model_key: String,
    pub input_price: f64,
    pub output_price: f64,
    pub input_price_extended: Option<f64>,
    pub output_price_extended: Option<f64>,
}

/// Get all model pricing settings
pub async fn get_model_pricing(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let pricing_settings = sqlx::query_as::<_, SystemSetting>(
        "SELECT * FROM system_settings WHERE setting_key LIKE 'model_pricing.%' ORDER BY setting_key"
    )
    .fetch_all(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Group by model
    let mut models: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();

    for setting in pricing_settings {
        let parts: Vec<&str> = setting.setting_key.split('.').collect();
        if parts.len() >= 3 {
            let model_name = parts[1];
            let price_type = parts[2];

            let entry = models.entry(model_name.to_string())
                .or_insert_with(|| json!({"model": model_name}));

            if let Some(obj) = entry.as_object_mut() {
                obj.insert(price_type.to_string(), json!(setting.setting_value.parse::<f64>().unwrap_or(0.0)));
                obj.insert(format!("{}_description", price_type), json!(setting.description.unwrap_or_default()));
                obj.insert("last_updated".to_string(), json!(setting.updated_at.to_rfc3339()));
            }
        }
    }

    Ok(Json(json!({
        "success": true,
        "models": models.values().collect::<Vec<_>>()
    })))
}

/// Update model pricing
pub async fn update_model_pricing(
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<UpdatePricingRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Validate inputs
    if payload.input_price < 0.0 || payload.output_price < 0.0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Prices cannot be negative"
            }))
        ));
    }

    // Update base pricing
    let input_key = format!("model_pricing.{}.input", payload.model_key);
    let output_key = format!("model_pricing.{}.output", payload.model_key);
    let input_base_key = format!("model_pricing.{}.input_base", payload.model_key);
    let output_base_key = format!("model_pricing.{}.output_base", payload.model_key);

    // Try base keys first (for Claude 4.5), fallback to simple keys
    let input_setting_key = if payload.input_price_extended.is_some() {
        &input_base_key
    } else {
        &input_key
    };

    let output_setting_key = if payload.output_price_extended.is_some() {
        &output_base_key
    } else {
        &output_key
    };

    sqlx::query(
        "INSERT INTO system_settings (setting_key, setting_value, setting_type, description, updated_at)
         VALUES ($1, $2, 'decimal', $3, NOW())
         ON CONFLICT (setting_key)
         DO UPDATE SET setting_value = $2, updated_at = NOW()"
    )
    .bind(input_setting_key)
    .bind(payload.input_price.to_string())
    .bind(format!("Input cost per 1M tokens for {}", payload.model_key))
    .execute(&state.db_pool)
    .await
    .map_err(|_| (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "success": false,
            "message": "Failed to update input pricing"
        }))
    ))?;

    sqlx::query(
        "INSERT INTO system_settings (setting_key, setting_value, setting_type, description, updated_at)
         VALUES ($1, $2, 'decimal', $3, NOW())
         ON CONFLICT (setting_key)
         DO UPDATE SET setting_value = $2, updated_at = NOW()"
    )
    .bind(output_setting_key)
    .bind(payload.output_price.to_string())
    .bind(format!("Output cost per 1M tokens for {}", payload.model_key))
    .execute(&state.db_pool)
    .await
    .map_err(|_| (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "success": false,
            "message": "Failed to update output pricing"
        }))
    ))?;

    // Update extended pricing if provided
    if let Some(input_ext) = payload.input_price_extended {
        let input_ext_key = format!("model_pricing.{}.input_extended", payload.model_key);
        sqlx::query(
            "INSERT INTO system_settings (setting_key, setting_value, setting_type, description, updated_at)
             VALUES ($1, $2, 'decimal', $3, NOW())
             ON CONFLICT (setting_key)
             DO UPDATE SET setting_value = $2, updated_at = NOW()"
        )
        .bind(&input_ext_key)
        .bind(input_ext.to_string())
        .bind(format!("Extended input cost per 1M tokens for {} (>200K context)", payload.model_key))
        .execute(&state.db_pool)
        .await
        .map_err(|_| (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Failed to update extended input pricing"
            }))
        ))?;
    }

    if let Some(output_ext) = payload.output_price_extended {
        let output_ext_key = format!("model_pricing.{}.output_extended", payload.model_key);
        sqlx::query(
            "INSERT INTO system_settings (setting_key, setting_value, setting_type, description, updated_at)
             VALUES ($1, $2, 'decimal', $3, NOW())
             ON CONFLICT (setting_key)
             DO UPDATE SET setting_value = $2, updated_at = NOW()"
        )
        .bind(&output_ext_key)
        .bind(output_ext.to_string())
        .bind(format!("Extended output cost per 1M tokens for {} (>200K context)", payload.model_key))
        .execute(&state.db_pool)
        .await
        .map_err(|_| (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "message": "Failed to update extended output pricing"
            }))
        ))?;
    }

    // Update last_updated timestamp
    let last_updated_key = format!("model_pricing.{}.last_updated", payload.model_key);
    sqlx::query(
        "INSERT INTO system_settings (setting_key, setting_value, setting_type, description, updated_at)
         VALUES ($1, $2, 'string', 'Last pricing update date', NOW())
         ON CONFLICT (setting_key)
         DO UPDATE SET setting_value = $2, updated_at = NOW()"
    )
    .bind(&last_updated_key)
    .bind(chrono::Utc::now().format("%Y-%m-%d").to_string())
    .execute(&state.db_pool)
    .await
    .ok();

    Ok(Json(json!({
        "success": true,
        "message": format!("Pricing updated for {}", payload.model_key)
    })))
}

// ============================================================================
// DEFAULT AI MODEL MANAGEMENT
// ============================================================================

#[derive(Deserialize)]
pub struct UpdateDefaultModelRequest {
    pub model: String,
}

/// Get the current default AI model
pub async fn get_default_model(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let setting = sqlx::query_as::<_, SystemSetting>(
        "SELECT * FROM system_settings WHERE setting_key = 'default_ai_model'"
    )
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let model = setting
        .map(|s| s.setting_value)
        .unwrap_or_else(|| "gemini".to_string());

    Ok(Json(json!({
        "success": true,
        "model": model
    })))
}

/// Update the default AI model
pub async fn update_default_model(
    Extension(state): Extension<Arc<AppState>>,
    Json(payload): Json<UpdateDefaultModelRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    // Validate model selection
    if payload.model != "claude" && payload.model != "gemini" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "success": false,
                "message": "Invalid model. Must be 'claude' or 'gemini'"
            }))
        ));
    }

    sqlx::query(
        "INSERT INTO system_settings (setting_key, setting_value, setting_type, description, updated_at)
         VALUES ('default_ai_model', $1, 'string', 'Default AI model for all users (claude or gemini)', NOW())
         ON CONFLICT (setting_key)
         DO UPDATE SET setting_value = $1, updated_at = NOW()"
    )
    .bind(&payload.model)
    .execute(&state.db_pool)
    .await
    .map_err(|_| (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "success": false,
            "message": "Failed to update default model"
        }))
    ))?;

    let model_name = match payload.model.as_str() {
        "claude" => "Claude Sonnet 4.5",
        "gemini" => "Gemini 2.5 Flash",
        _ => &payload.model,
    };

    tracing::info!("🤖 Default AI model updated to: {}", model_name);

    Ok(Json(json!({
        "success": true,
        "message": format!("Default AI model updated to {}", model_name)
    })))
}

// ============================================================================
// YouTube Feature Control
// ============================================================================

pub async fn get_youtube_feature_status(
    Extension(state): Extension<Arc<AppState>>
) -> Result<Json<serde_json::Value>, StatusCode> {
    let setting = sqlx::query_as::<_, SystemSetting>(
        "SELECT * FROM system_settings WHERE setting_key = 'youtube_features_enabled'"
    )
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let enabled = setting
        .map(|s| s.as_bool().unwrap_or(false))
        .unwrap_or(false);

    Ok(Json(json!({
        "success": true,
        "status": {
            "enabled": enabled,
            "description": "When enabled, all users can access YouTube features. When disabled, only admins and whitelisted users have access."
        }
    })))
}

#[derive(Deserialize)]
pub struct YouTubeFeatureToggleRequest {
    pub enabled: bool,
}

pub async fn toggle_youtube_features(
    Extension(state): Extension<Arc<AppState>>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<YouTubeFeatureToggleRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let setting_value = if payload.enabled { "true" } else { "false" };

    // UPSERT: Insert or update
    sqlx::query(
        "INSERT INTO system_settings (setting_key, setting_value, setting_type, description, updated_by, updated_at)
         VALUES ('youtube_features_enabled', $1, 'boolean',
                 'Enable YouTube integration for all users. When disabled, only admins and whitelisted users have access.',
                 $2, NOW())
         ON CONFLICT (setting_key)
         DO UPDATE SET setting_value = $1, updated_by = $2, updated_at = NOW()"
    )
    .bind(setting_value)
    .bind(claims.sub.parse::<i32>().unwrap_or(0))
    .execute(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to toggle YouTube features: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tracing::info!(
        "YouTube features {} by admin user {} ({})",
        if payload.enabled { "enabled" } else { "disabled" },
        claims.username,
        claims.email
    );

    Ok(Json(json!({
        "success": true,
        "message": format!("YouTube features {}", if payload.enabled { "enabled for all users" } else { "disabled (testing mode)" }),
        "enabled": payload.enabled
    })))
}

// ============================================================================
// CLIPPING ACTIVITY DASHBOARD - Admin monitoring for YouTube clipping jobs
// ============================================================================

/// Admin API: Get clipping statistics overview and per-user breakdown
pub async fn admin_clipping_stats(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Query 1: Overview stats
    let overview_row = sqlx::query(
        r#"
        SELECT
            COUNT(*) FILTER (WHERE status = 'completed') as completed,
            COUNT(*) FILTER (WHERE status = 'failed') as failed,
            COUNT(*) FILTER (WHERE status IN ('pending', 'downloading', 'analyzing', 'extracting_clips', 'posting')) as active,
            COUNT(*) as total,
            COALESCE(ROUND(100.0 * COUNT(*) FILTER (WHERE status = 'completed') / NULLIF(COUNT(*), 0), 2), 0.0) as success_rate
        FROM clipping_jobs
        "#
    )
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch clipping overview stats: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let completed: i64 = overview_row.try_get("completed").unwrap_or(0);
    let failed: i64 = overview_row.try_get("failed").unwrap_or(0);
    let active: i64 = overview_row.try_get("active").unwrap_or(0);
    let total: i64 = overview_row.try_get("total").unwrap_or(0);
    let success_rate: rust_decimal::Decimal = overview_row.try_get("success_rate").unwrap_or_else(|_| rust_decimal::Decimal::new(0, 0));

    // Query 2: Per-user breakdown
    let user_rows = sqlx::query(
        r#"
        SELECT
            u.id as user_id,
            u.username,
            u.email,
            u.is_active,
            COUNT(DISTINCT ycl.id) as linkage_count,
            COUNT(DISTINCT ycl.source_channel_id) as source_channel_count,
            COUNT(DISTINCT ycl.destination_channel_id) as destination_channel_count,
            COUNT(cj.id) FILTER (WHERE cj.status = 'completed') as completed_jobs,
            COUNT(cj.id) FILTER (WHERE cj.status = 'failed') as failed_jobs,
            COUNT(cj.id) FILTER (WHERE cj.status IN ('pending', 'downloading', 'analyzing', 'extracting_clips', 'posting')) as active_jobs,
            COUNT(cj.id) as total_jobs,
            CASE
                WHEN COUNT(cj.id) > 0 THEN
                    ROUND(100.0 * COUNT(cj.id) FILTER (WHERE cj.status = 'completed') / COUNT(cj.id), 2)
                ELSE 0.0
            END as success_rate,
            COUNT(ec.id) FILTER (WHERE ec.upload_status = 'published') as published_clips,
            MAX(cj.created_at) as last_job_created,
            MAX(ec.published_at) as last_clip_published
        FROM users u
        LEFT JOIN youtube_channel_linkages ycl ON u.id = ycl.user_id
        LEFT JOIN clipping_jobs cj ON ycl.id = cj.linkage_id
        LEFT JOIN extracted_clips ec ON cj.id = ec.clipping_job_id
        GROUP BY u.id, u.username, u.email, u.is_active
        HAVING COUNT(DISTINCT ycl.id) > 0
        ORDER BY COUNT(cj.id) DESC
        "#
    )
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch per-user clipping stats: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let users: Vec<serde_json::Value> = user_rows.iter().map(|row| {
        use sqlx::Row;
        json!({
            "user_id": row.get::<i32, _>("user_id"),
            "username": row.get::<String, _>("username"),
            "email": row.get::<String, _>("email"),
            "is_active": row.get::<bool, _>("is_active"),
            "linkage_count": row.get::<i64, _>("linkage_count"),
            "source_channel_count": row.get::<i64, _>("source_channel_count"),
            "destination_channel_count": row.get::<i64, _>("destination_channel_count"),
            "completed_jobs": row.get::<i64, _>("completed_jobs"),
            "failed_jobs": row.get::<i64, _>("failed_jobs"),
            "active_jobs": row.get::<i64, _>("active_jobs"),
            "total_jobs": row.get::<i64, _>("total_jobs"),
            "success_rate": row.get::<rust_decimal::Decimal, _>("success_rate"),
            "published_clips": row.get::<i64, _>("published_clips"),
            "last_job_created": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_job_created"),
            "last_clip_published": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_clip_published"),
        })
    }).collect();

    Ok(Json(json!({
        "success": true,
        "overview": {
            "total_jobs": total,
            "completed_jobs": completed,
            "failed_jobs": failed,
            "active_jobs": active,
            "success_rate": success_rate
        },
        "users": users
    })))
}

/// Admin API: Get detailed clipping information for a specific user
pub async fn admin_user_clipping_details(
    Extension(state): Extension<Arc<AppState>>,
    Path(user_id): Path<i32>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Query 1: User's linkages with channel details
    let linkage_rows = sqlx::query(
        r#"
        SELECT
            ycl.id as linkage_id,
            ycl.is_active,
            ysc.channel_id as source_channel_id,
            ysc.channel_name as source_channel_name,
            ysc.channel_thumbnail_url as source_thumbnail,
            cyc.channel_id as dest_channel_id,
            cyc.channel_name as dest_channel_name,
            cyc.channel_thumbnail_url as dest_thumbnail,
            ycl.clips_per_video,
            ycl.total_clips_generated,
            ycl.total_clips_posted,
            ycl.last_clip_generated_at,
            ycl.created_at as linkage_created
        FROM youtube_channel_linkages ycl
        JOIN youtube_source_channels ysc ON ycl.source_channel_id = ysc.id
        JOIN connected_youtube_channels cyc ON ycl.destination_channel_id = cyc.id
        WHERE ycl.user_id = $1
        ORDER BY ycl.created_at DESC
        "#
    )
    .bind(user_id)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch user linkages: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let linkages: Vec<serde_json::Value> = linkage_rows.iter().map(|row| {
        use sqlx::Row;
        json!({
            "linkage_id": row.get::<i32, _>("linkage_id"),
            "is_active": row.get::<bool, _>("is_active"),
            "source_channel_id": row.get::<String, _>("source_channel_id"),
            "source_channel_name": row.get::<String, _>("source_channel_name"),
            "source_thumbnail": row.get::<Option<String>, _>("source_thumbnail"),
            "dest_channel_id": row.get::<String, _>("dest_channel_id"),
            "dest_channel_name": row.get::<String, _>("dest_channel_name"),
            "dest_thumbnail": row.get::<Option<String>, _>("dest_thumbnail"),
            "clips_per_video": row.get::<Option<i32>, _>("clips_per_video"),
            "total_clips_generated": row.get::<i32, _>("total_clips_generated"),
            "total_clips_posted": row.get::<i32, _>("total_clips_posted"),
            "last_clip_generated_at": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_clip_generated_at"),
            "linkage_created": row.get::<chrono::DateTime<chrono::Utc>, _>("linkage_created"),
        })
    }).collect();

    // Query 2: Recent jobs
    let job_rows = sqlx::query(
        r#"
        SELECT
            cj.id,
            cj.source_video_id,
            cj.source_video_title,
            cj.status,
            cj.current_step,
            cj.progress_percent,
            cj.error_message,
            cj.retry_count,
            cj.created_at,
            cj.updated_at,
            cj.completed_at,
            ycl.id as linkage_id,
            ysc.channel_name as source_channel
        FROM clipping_jobs cj
        JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
        JOIN youtube_source_channels ysc ON ycl.source_channel_id = ysc.id
        WHERE ycl.user_id = $1
        ORDER BY cj.created_at DESC
        LIMIT 10
        "#
    )
    .bind(user_id)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch recent jobs: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let recent_jobs: Vec<serde_json::Value> = job_rows.iter().map(|row| {
        use sqlx::Row;
        json!({
            "id": row.get::<i32, _>("id"),
            "source_video_id": row.get::<String, _>("source_video_id"),
            "source_video_title": row.get::<Option<String>, _>("source_video_title"),
            "status": row.get::<String, _>("status"),
            "current_step": row.get::<Option<String>, _>("current_step"),
            "progress_percent": row.get::<Option<i32>, _>("progress_percent"),
            "error_message": row.get::<Option<String>, _>("error_message"),
            "retry_count": row.get::<i32, _>("retry_count"),
            "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            "updated_at": row.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
            "completed_at": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("completed_at"),
            "linkage_id": row.get::<i32, _>("linkage_id"),
            "source_channel": row.get::<String, _>("source_channel"),
        })
    }).collect();

    Ok(Json(json!({
        "success": true,
        "linkages": linkages,
        "recent_jobs": recent_jobs
    })))
}

/// Admin HTML Page: Clipping Activity Dashboard
pub async fn admin_clipping_activity_page() -> Html<String> {
    let html = r###"
    <!DOCTYPE html>
    <html lang="en">
    <head>
        <meta charset="UTF-8">
        <meta name="viewport" content="width=device-width, initial-scale=1.0">
        <title>Clipping Activity - Admin Dashboard</title>
        <style>
            * { margin: 0; padding: 0; box-sizing: border-box; }
            body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #f8f9fa; }
            .sidebar { width: 250px; background: #343a40; height: 100vh; position: fixed; left: 0; top: 0; color: white; padding: 1rem; }
            .sidebar h2 { color: #dc3545; margin-bottom: 2rem; }
            .sidebar ul { list-style: none; }
            .sidebar li { margin-bottom: 0.5rem; }
            .sidebar a { color: #adb5bd; text-decoration: none; padding: 0.5rem; display: block; border-radius: 5px; }
            .sidebar a:hover { background: #495057; color: white; }
            .sidebar a.active { background: #dc3545; color: white; }
            .main-content { margin-left: 250px; padding: 2rem; }
            .header { background: white; padding: 1rem 2rem; margin-bottom: 2rem; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
            .header h1 { color: #343a40; margin-bottom: 0.5rem; }
            .header p { color: #6c757d; }
            .stats-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 1.5rem; margin-bottom: 2rem; }
            .stat-card { background: white; padding: 1.5rem; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
            .stat-number { font-size: 2rem; font-weight: bold; }
            .stat-label { color: #6c757d; margin-top: 0.5rem; }
            .recent-section { background: white; padding: 2rem; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); margin-bottom: 2rem; }
            .recent-section h2 { color: #343a40; margin-bottom: 1rem; }
            table { width: 100%; border-collapse: collapse; background: white; border-radius: 8px; overflow: hidden; }
            th, td { padding: 1rem; text-align: left; border-bottom: 1px solid #eee; }
            th { background: #f8f9fa; font-weight: 600; }
            tr:hover { background: #f8f9fa; cursor: pointer; }
            .badge { padding: 0.25rem 0.75rem; border-radius: 12px; font-size: 0.85rem; font-weight: 500; }
            .badge-success { background: #d4edda; color: #155724; }
            .badge-danger { background: #f8d7da; color: #721c24; }
            .badge-warning { background: #fff3cd; color: #856404; }
            .badge-secondary { background: #e2e3e5; color: #383d41; }
            .btn { padding: 0.5rem 1rem; background: #dc3545; color: white; border: none; border-radius: 5px; cursor: pointer; text-decoration: none; display: inline-block; }
            .btn-secondary { background: #6c757d; }
            .btn-secondary:hover { background: #5a6268; }
        </style>
    </head>
    <body>
        <div class="sidebar">
            <h2>🛡️ Admin Panel</h2>
            <ul>
                <li><a href="/admin/dashboard">📊 Dashboard</a></li>
                <li><a href="/admin/users">👥 Users</a></li>
                <li><a href="/admin/clipping-activity" class="active">🎬 Clipping Activity</a></li>
                <li><a href="/api/docs">📚 API Docs</a></li>
                <li><a href="/api/status">⚙️ System Status</a></li>
            </ul>
            <div style="position: absolute; bottom: 1rem;">
                <button onclick="logout()" class="btn btn-secondary">Logout</button>
            </div>
        </div>

        <div class="main-content">
            <div class="header">
                <h1>🎬 Clipping Activity Dashboard</h1>
                <p>Real-time monitoring of YouTube clipping operations</p>
            </div>

            <div class="stats-grid">
                <div class="stat-card">
                    <div class="stat-number" id="totalJobs">-</div>
                    <div class="stat-label">Total Jobs</div>
                </div>
                <div class="stat-card">
                    <div class="stat-number" id="completedJobs" style="color: #28a745;">-</div>
                    <div class="stat-label">Completed</div>
                </div>
                <div class="stat-card">
                    <div class="stat-number" id="failedJobs" style="color: #dc3545;">-</div>
                    <div class="stat-label">Failed</div>
                </div>
                <div class="stat-card">
                    <div class="stat-number" id="activeJobs" style="color: #ffc107;">-</div>
                    <div class="stat-label">Active</div>
                </div>
                <div class="stat-card">
                    <div class="stat-number" id="successRate">-</div>
                    <div class="stat-label">Success Rate</div>
                </div>
            </div>

            <div class="recent-section">
                <h2>User Activity Breakdown</h2>
                <p style="color: #6c757d; margin-bottom: 1rem;">Click on a user to see detailed linkages and recent jobs</p>
                <table id="userTable">
                    <thead>
                        <tr>
                            <th>User</th>
                            <th>Linkages</th>
                            <th>Total Jobs</th>
                            <th>Completed</th>
                            <th>Failed</th>
                            <th>Active</th>
                            <th>Success Rate</th>
                            <th>Last Activity</th>
                        </tr>
                    </thead>
                    <tbody id="userTableBody">
                        <tr><td colspan="8" style="text-align: center; padding: 2rem; color: #6c757d;">Loading...</td></tr>
                    </tbody>
                </table>
            </div>

            <div id="userDetails" style="display: none; background: white; padding: 2rem; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.1);">
                <h2>User Details: <span id="detailUsername"></span></h2>
                <h3 style="margin-top: 1.5rem;">Recent Jobs (Last 10)</h3>
                <table id="jobsTable">
                    <thead>
                        <tr>
                            <th>Job ID</th>
                            <th>Video</th>
                            <th>Status</th>
                            <th>Progress</th>
                            <th>Retries</th>
                            <th>Created</th>
                            <th>Error</th>
                        </tr>
                    </thead>
                    <tbody id="jobsTableBody"></tbody>
                </table>
            </div>
        </div>

        <script>
            const token = localStorage.getItem('authToken');
            const user = JSON.parse(localStorage.getItem('user') || '{}');
            if (!token || (!user.is_staff && !user.is_superuser)) {
                window.location.href = '/admin/login';
            }

            loadClippingStats();

            async function loadClippingStats() {
                try {
                    const response = await fetch('/api/admin/clipping/stats', {
                        headers: { 'Authorization': `Bearer ${token}` }
                    });
                    const data = await response.json();

                    document.getElementById('totalJobs').textContent = data.overview.total_jobs;
                    document.getElementById('completedJobs').textContent = data.overview.completed_jobs;
                    document.getElementById('failedJobs').textContent = data.overview.failed_jobs;
                    document.getElementById('activeJobs').textContent = data.overview.active_jobs;
                    document.getElementById('successRate').textContent = data.overview.success_rate + '%';

                    const tbody = document.getElementById('userTableBody');
                    tbody.innerHTML = '';

                    if (data.users.length === 0) {
                        tbody.innerHTML = '<tr><td colspan="8" style="text-align: center; padding: 2rem; color: #6c757d;">No clipping activity yet</td></tr>';
                        return;
                    }

                    data.users.forEach(user => {
                        const row = document.createElement('tr');
                        row.onclick = () => loadUserDetails(user.user_id, user.username);
                        row.innerHTML = `
                            <td><strong>${user.username}</strong><br><small>${user.email}</small></td>
                            <td>${user.linkage_count}</td>
                            <td>${user.total_jobs}</td>
                            <td><span class="badge badge-success">${user.completed_jobs}</span></td>
                            <td><span class="badge badge-danger">${user.failed_jobs}</span></td>
                            <td><span class="badge badge-warning">${user.active_jobs}</span></td>
                            <td>${user.success_rate}%</td>
                            <td>${formatDate(user.last_job_created)}</td>
                        `;
                        tbody.appendChild(row);
                    });
                } catch (error) {
                    console.error('Failed to load stats:', error);
                    document.getElementById('userTableBody').innerHTML = '<tr><td colspan="8" style="text-align: center; padding: 2rem; color: #dc3545;">Failed to load data</td></tr>';
                }
            }

            async function loadUserDetails(userId, username) {
                try {
                    const response = await fetch(`/api/admin/clipping/user/${userId}/details`, {
                        headers: { 'Authorization': `Bearer ${token}` }
                    });
                    const data = await response.json();

                    document.getElementById('userDetails').style.display = 'block';
                    document.getElementById('detailUsername').textContent = username;

                    const jobsBody = document.getElementById('jobsTableBody');
                    jobsBody.innerHTML = '';

                    if (data.recent_jobs.length === 0) {
                        jobsBody.innerHTML = '<tr><td colspan="7" style="text-align: center; padding: 2rem; color: #6c757d;">No jobs found</td></tr>';
                        return;
                    }

                    data.recent_jobs.forEach(job => {
                        const row = document.createElement('tr');
                        row.innerHTML = `
                            <td>#${job.id}</td>
                            <td><a href="https://youtube.com/watch?v=${job.source_video_id}" target="_blank" style="color: #dc3545; text-decoration: none;">
                                ${job.source_video_title || job.source_video_id}
                            </a></td>
                            <td><span class="badge badge-${getStatusColor(job.status)}">${job.status}</span></td>
                            <td>${job.progress_percent}%</td>
                            <td>${job.retry_count > 0 ? '🔄 ' + job.retry_count : '-'}</td>
                            <td>${formatDate(job.created_at)}</td>
                            <td style="max-width: 300px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;" title="${job.error_message || ''}">
                                ${job.error_message ? '⚠️ ' + job.error_message.substring(0, 50) + (job.error_message.length > 50 ? '...' : '') : '-'}
                            </td>
                        `;
                        jobsBody.appendChild(row);
                    });

                    document.getElementById('userDetails').scrollIntoView({ behavior: 'smooth' });
                } catch (error) {
                    console.error('Failed to load user details:', error);
                    alert('Failed to load user details');
                }
            }

            function getStatusColor(status) {
                const colors = {
                    'completed': 'success',
                    'failed': 'danger',
                    'cancelled': 'secondary',
                    'no_clips_found': 'secondary',
                    'pending': 'warning',
                    'processing': 'info',
                    // old sequential pipeline step values (backward compat)
                    'downloading': 'info',
                    'analyzing': 'info',
                    'extracting_clips': 'info',
                    'posting': 'info'
                };
                return colors[status] || 'secondary';
            }

            function formatDate(dateStr) {
                if (!dateStr) return 'Never';
                return new Date(dateStr).toLocaleString();
            }

            function logout() {
                localStorage.removeItem('authToken');
                localStorage.removeItem('user');
                window.location.href = '/admin/login';
            }

            // Auto-refresh every 30 seconds
            setInterval(loadClippingStats, 30000);
        </script>
    </body>
    </html>
    "###;

    Html(html.to_string())
}
// ============================================================================
// ADMIN JOB MANAGEMENT - New Endpoints for Managing ALL Clipping Jobs
// ============================================================================

/// List all clipping jobs with filters and pagination (admin only)
pub async fn admin_list_all_jobs(
    Extension(state): Extension<Arc<AppState>>,
    Query(query): Query<JobsQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(50).min(100); // Max 100 per page
    let offset = (page - 1) * limit;
    let sort = query.sort.as_deref().unwrap_or("created_desc");

    // Count total jobs with filters
    let count_query = if query.status.is_some() || query.user_id.is_some() {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM clipping_jobs cj
             JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
             WHERE ($1::text IS NULL OR cj.status = $1)
             AND ($2::int IS NULL OR ycl.user_id = $2)"
        )
        .bind(&query.status)
        .bind(query.user_id)
    } else {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM clipping_jobs")
    };

    let total: i64 = count_query
        .fetch_one(&state.db_pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to count jobs: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Build dynamic ORDER BY clause
    let order_clause = match sort {
        "created_asc" => "cj.created_at ASC",
        "updated_desc" => "cj.updated_at DESC",
        "updated_asc" => "cj.updated_at ASC",
        _ => "cj.created_at DESC", // Default: newest first
    };

    // Fetch jobs with joins
    let jobs_query = format!(
        "SELECT
            cj.id, cj.linkage_id, cj.source_video_id, cj.source_video_title,
            cj.source_video_duration_seconds, cj.status, cj.current_step,
            cj.progress_percent, cj.error_message, cj.retry_count, cj.last_retry_at,
            cj.claimed_by, cj.claimed_at, cj.created_at, cj.updated_at, cj.completed_at,
            cj.stuck_detection_count,
            u.id as user_id, u.username, u.email,
            ysc.channel_name as source_channel_name,
            cyc.channel_name as dest_channel_name,
            EXTRACT(EPOCH FROM (COALESCE(cj.completed_at, NOW()) - cj.created_at))/60 as duration_minutes
         FROM clipping_jobs cj
         JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
         JOIN users u ON ycl.user_id = u.id
         JOIN youtube_source_channels ysc ON ycl.source_channel_id = ysc.id
         JOIN connected_youtube_channels cyc ON ycl.destination_channel_id = cyc.id
         WHERE ($1::text IS NULL OR cj.status = $1)
         AND ($2::int IS NULL OR u.id = $2)
         ORDER BY {}
         LIMIT $3 OFFSET $4",
        order_clause
    );

    let rows = sqlx::query(&jobs_query)
        .bind(&query.status)
        .bind(query.user_id)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&state.db_pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch jobs: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let jobs: Vec<serde_json::Value> = rows.iter().map(|row| {
        use sqlx::Row;
        json!({
            "id": row.get::<i32, _>("id"),
            "user_id": row.get::<i32, _>("user_id"),
            "username": row.get::<String, _>("username"),
            "email": row.get::<String, _>("email"),
            "linkage_id": row.get::<i32, _>("linkage_id"),
            "source_channel_name": row.get::<String, _>("source_channel_name"),
            "dest_channel_name": row.get::<String, _>("dest_channel_name"),
            "source_video_id": row.get::<String, _>("source_video_id"),
            "source_video_title": row.get::<Option<String>, _>("source_video_title"),
            "status": row.get::<String, _>("status"),
            "current_step": row.get::<Option<String>, _>("current_step"),
            "progress_percent": row.get::<i32, _>("progress_percent"),
            "error_message": row.get::<Option<String>, _>("error_message"),
            "retry_count": row.get::<i32, _>("retry_count"),
            "last_retry_at": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_retry_at"),
            "claimed_by": row.get::<Option<String>, _>("claimed_by"),
            "claimed_at": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("claimed_at"),
            "created_at": row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            "updated_at": row.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
            "completed_at": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("completed_at"),
            "duration_minutes": row.get::<Option<rust_decimal::Decimal>, _>("duration_minutes"),
        })
    }).collect();

    let total_pages = ((total as f64) / (limit as f64)).ceil() as u32;

    Ok(Json(json!({
        "success": true,
        "jobs": jobs,
        "pagination": {
            "page": page,
            "limit": limit,
            "total": total,
            "total_pages": total_pages
        },
        "filters_applied": {
            "status": query.status,
            "user_id": query.user_id,
            "sort": sort
        }
    })))
}

/// Get detailed information about a specific job
pub async fn admin_get_job_details(
    Extension(state): Extension<Arc<AppState>>,
    Path(job_id): Path<i32>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Fetch job details with joins
    let job_row = sqlx::query(
        "SELECT
            cj.*,
            u.id as user_id, u.username, u.email,
            ysc.channel_name as source_channel_name,
            cyc.channel_name as dest_channel_name,
            ycl.clips_per_video, ycl.min_clip_duration_seconds, ycl.max_clip_duration_seconds,
            EXTRACT(EPOCH FROM (COALESCE(cj.completed_at, NOW()) - cj.created_at))/60 as duration_minutes
         FROM clipping_jobs cj
         JOIN youtube_channel_linkages ycl ON cj.linkage_id = ycl.id
         JOIN users u ON ycl.user_id = u.id
         JOIN youtube_source_channels ysc ON ycl.source_channel_id = ysc.id
         JOIN connected_youtube_channels cyc ON ycl.destination_channel_id = cyc.id
         WHERE cj.id = $1"
    )
    .bind(job_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch job details: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let job_row = match job_row {
        Some(row) => row,
        None => return Err(StatusCode::NOT_FOUND),
    };

    // Fetch extracted clips
    let clips_rows = sqlx::query(
        "SELECT * FROM extracted_clips WHERE clipping_job_id = $1 ORDER BY clip_number ASC"
    )
    .bind(job_id)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch clips: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    use sqlx::Row;
    let clips: Vec<serde_json::Value> = clips_rows.iter().map(|row| {
        json!({
            "id": row.get::<i32, _>("id"),
            "clip_number": row.get::<i32, _>("clip_number"),
            "start_time_seconds": row.get::<f64, _>("start_time_seconds"),
            "end_time_seconds": row.get::<f64, _>("end_time_seconds"),
            "duration_seconds": row.get::<f64, _>("duration_seconds"),
            "ai_title": row.get::<Option<String>, _>("ai_title"),
            "ai_description": row.get::<Option<String>, _>("ai_description"),
            "ai_confidence_score": row.get::<Option<f64>, _>("ai_confidence_score"),
            "youtube_video_id": row.get::<Option<String>, _>("youtube_video_id"),
            "upload_status": row.get::<String, _>("upload_status"),
            "published_at": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("published_at"),
        })
    }).collect();

    // Build job details JSON
    let job = json!({
        "id": job_row.get::<i32, _>("id"),
        "user_id": job_row.get::<i32, _>("user_id"),
        "username": job_row.get::<String, _>("username"),
        "email": job_row.get::<String, _>("email"),
        "linkage_id": job_row.get::<i32, _>("linkage_id"),
        "source_channel_name": job_row.get::<String, _>("source_channel_name"),
        "dest_channel_name": job_row.get::<String, _>("dest_channel_name"),
        "source_video_id": job_row.get::<String, _>("source_video_id"),
        "source_video_title": job_row.get::<Option<String>, _>("source_video_title"),
        "source_video_duration_seconds": job_row.get::<Option<i32>, _>("source_video_duration_seconds"),
        "local_video_path": job_row.get::<Option<String>, _>("local_video_path"),
        "status": job_row.get::<String, _>("status"),
        "current_step": job_row.get::<Option<String>, _>("current_step"),
        "progress_percent": job_row.get::<i32, _>("progress_percent"),
        "error_message": job_row.get::<Option<String>, _>("error_message"),
        "retry_count": job_row.get::<i32, _>("retry_count"),
        "last_retry_at": job_row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_retry_at"),
        "stuck_detection_count": job_row.get::<i32, _>("stuck_detection_count"),
        "claimed_by": job_row.get::<Option<String>, _>("claimed_by"),
        "claimed_at": job_row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("claimed_at"),
        "started_at": job_row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("started_at"),
        "created_at": job_row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        "updated_at": job_row.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
        "completed_at": job_row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("completed_at"),
        "duration_minutes": job_row.get::<Option<rust_decimal::Decimal>, _>("duration_minutes"),
        "clips_per_video": job_row.get::<i32, _>("clips_per_video"),
        "min_clip_duration": job_row.get::<i32, _>("min_clip_duration_seconds"),
        "max_clip_duration": job_row.get::<i32, _>("max_clip_duration_seconds"),
    });

    Ok(Json(json!({
        "success": true,
        "job": job,
        "clips": clips,
        "clips_count": clips.len()
    })))
}

/// Retry a failed or cancelled job (admin can retry ANY job).
///
/// Phase-aware: reads current_step to determine resume_from so the job skips
/// already-completed phases instead of restarting from Phase A every time.
pub async fn admin_retry_job(
    Extension(state): Extension<Arc<AppState>>,
    Path(job_id): Path<i32>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Step 1: Read current_step so we can compute the resume phase.
    let current_step: Option<String> = sqlx::query_scalar(
        "SELECT current_step FROM clipping_jobs WHERE id = $1 AND status IN ('failed', 'cancelled')"
    )
    .bind(job_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to read job current_step: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .flatten();

    if current_step.is_none() {
        // Job doesn't exist or isn't in a retryable status — check which.
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM clipping_jobs WHERE id = $1)")
            .bind(job_id)
            .fetch_one(&state.db_pool)
            .await
            .unwrap_or(false);
        return Ok(Json(json!({
            "success": false,
            "message": if exists {
                "Job is not in failed or cancelled status"
            } else {
                "Job not found"
            }
        })));
    }

    // Step 2: Map current_step → resume_from (same logic as auto_retry_failed_jobs).
    let step = current_step.as_deref().unwrap_or("");
    let resume_from: Option<&str> = match step {
        s if s.contains("posting") || s.contains("upload") => Some("clips_extracted"),
        s if s.contains("vectoriz")                         => Some("clips_extracted"),
        s if s.contains("extracting") || s == "clips_extracted" => Some("downloaded"),
        s if s.contains("download")                         => Some("analyzed"),
        _                                                   => None,
    };

    if let Some(phase) = resume_from {
        tracing::info!("Admin retrying job {} — resuming from '{}' (was at: {:?})", job_id, phase, step);
    } else {
        tracing::info!("Admin retrying job {} — restarting from Phase A (current_step: {:?})", job_id, step);
    }

    // Step 3: Reset to pending with the computed resume_from.
    let result = sqlx::query(
        "UPDATE clipping_jobs SET
            status = 'pending',
            resume_from = $2,
            error_message = NULL,
            progress_percent = 0,
            current_step = 'queued',
            retry_count = COALESCE(retry_count, 0) + 1,
            last_retry_at = NOW(),
            started_at = NULL,
            completed_at = NULL,
            claimed_by = NULL,
            claimed_at = NULL,
            updated_at = NOW()
         WHERE id = $1 AND status IN ('failed', 'cancelled')
         RETURNING retry_count"
    )
    .bind(job_id)
    .bind(resume_from)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to retry job: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    match result {
        Some(row) => {
            let retry_count: i32 = row.get("retry_count");
            let phase_msg = resume_from
                .map(|p| format!(" (resuming from {})", p))
                .unwrap_or_else(|| " (starting from Phase A)".to_string());
            Ok(Json(json!({
                "success": true,
                "message": format!("Job #{} queued for retry attempt #{}{}", job_id, retry_count, phase_msg)
            })))
        }
        None => Ok(Json(json!({
            "success": false,
            "message": "Job not found or not in failed/cancelled status"
        }))),
    }
}

/// Cancel a running or stuck job.
///
/// Preserves current_step so that a subsequent admin retry can resume from the
/// correct phase rather than restarting from Phase A.
pub async fn admin_cancel_job(
    Extension(state): Extension<Arc<AppState>>,
    Path(job_id): Path<i32>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Cancel job (admin can cancel any non-terminal job).
    // current_step is intentionally NOT overwritten — the retry handler reads it
    // to determine which pipeline phase to resume from.
    let result = sqlx::query(
        "UPDATE clipping_jobs SET
            status = 'cancelled',
            updated_at = NOW(),
            completed_at = NOW(),
            claimed_by = NULL,
            claimed_at = NULL
         WHERE id = $1 AND status NOT IN ('completed', 'failed', 'cancelled')"
    )
    .bind(job_id)
    .execute(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to cancel job: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if result.rows_affected() > 0 {
        tracing::info!("Admin cancelled job {}", job_id);
        Ok(Json(json!({
            "success": true,
            "message": format!("Job #{} cancelled successfully", job_id)
        })))
    } else {
        Ok(Json(json!({
            "success": false,
            "message": "Job not found or already in terminal status"
        })))
    }
}

/// Get all clips for a specific job
pub async fn admin_get_job_clips(
    Extension(state): Extension<Arc<AppState>>,
    Path(job_id): Path<i32>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let clips_rows = sqlx::query(
        "SELECT * FROM extracted_clips WHERE clipping_job_id = $1 ORDER BY clip_number ASC"
    )
    .bind(job_id)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch clips: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    use sqlx::Row;
    let clips: Vec<serde_json::Value> = clips_rows.iter().map(|row| {
        json!({
            "id": row.get::<i32, _>("id"),
            "clip_number": row.get::<i32, _>("clip_number"),
            "local_clip_path": row.get::<String, _>("local_clip_path"),
            "start_time_seconds": row.get::<f64, _>("start_time_seconds"),
            "end_time_seconds": row.get::<f64, _>("end_time_seconds"),
            "duration_seconds": row.get::<f64, _>("duration_seconds"),
            "ai_title": row.get::<Option<String>, _>("ai_title"),
            "ai_description": row.get::<Option<String>, _>("ai_description"),
            "ai_confidence_score": row.get::<Option<f64>, _>("ai_confidence_score"),
            "youtube_video_id": row.get::<Option<String>, _>("youtube_video_id"),
            "youtube_url": row.get::<Option<String>, _>("youtube_url"),
            "upload_status": row.get::<String, _>("upload_status"),
            "published_at": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("published_at"),
            "views_24h": row.get::<i32, _>("views_24h"),
            "likes_24h": row.get::<i32, _>("likes_24h"),
            "comments_24h": row.get::<i32, _>("comments_24h"),
        })
    }).collect();

    Ok(Json(json!({
        "success": true,
        "clips": clips,
        "total": clips.len()
    })))
}

/// Admin Clipping Jobs Management Page - HTML Dashboard
pub async fn admin_clipping_jobs_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Clipping Jobs Management - Admin</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
            background: #f8f9fa;
            color: #343a40;
        }

        /* Sidebar */
        .sidebar {
            position: fixed;
            top: 0;
            left: 0;
            width: 250px;
            height: 100vh;
            background: #343a40;
            padding: 2rem 0;
            color: white;
        }

        .sidebar h2 {
            padding: 0 1.5rem 2rem;
            border-bottom: 1px solid #495057;
            margin-bottom: 1rem;
            color: #dc3545;
        }

        .sidebar ul {
            list-style: none;
        }

        .sidebar li a {
            display: block;
            padding: 0.75rem 1.5rem;
            color: white;
            text-decoration: none;
            transition: background 0.2s;
        }

        .sidebar li a:hover,
        .sidebar li a.active {
            background: #495057;
        }

        .sidebar li a.active {
            border-left: 3px solid #dc3545;
        }

        /* Main Content */
        .main-content {
            margin-left: 250px;
            padding: 2rem;
        }

        .header {
            background: white;
            padding: 1.5rem;
            border-radius: 8px;
            box-shadow: 0 2px 4px rgba(0,0,0,0.1);
            margin-bottom: 2rem;
            display: flex;
            justify-content: space-between;
            align-items: center;
        }

        .filters {
            background: white;
            padding: 1.5rem;
            border-radius: 8px;
            box-shadow: 0 2px 4px rgba(0,0,0,0.1);
            margin-bottom: 2rem;
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 1rem;
        }

        .filter-group {
            display: flex;
            flex-direction: column;
            gap: 0.5rem;
        }

        .filter-group label {
            font-weight: 600;
            color: #495057;
            font-size: 0.9rem;
        }

        .filter-group select,
        .filter-group input {
            padding: 0.5rem;
            border: 1px solid #ced4da;
            border-radius: 4px;
            font-size: 1rem;
        }

        .btn {
            padding: 0.5rem 1rem;
            border: none;
            border-radius: 4px;
            cursor: pointer;
            font-size: 0.9rem;
            transition: all 0.2s;
        }

        .btn-primary {
            background: #dc3545;
            color: white;
        }

        .btn-primary:hover {
            background: #c82333;
        }

        .btn-secondary {
            background: #6c757d;
            color: white;
        }

        .btn-secondary:hover {
            background: #5a6268;
        }

        .btn-success {
            background: #28a745;
            color: white;
        }

        .btn-sm {
            padding: 0.25rem 0.5rem;
            font-size: 0.85rem;
        }

        /* Jobs Table */
        .jobs-table {
            background: white;
            border-radius: 8px;
            box-shadow: 0 2px 4px rgba(0,0,0,0.1);
            overflow: hidden;
        }

        table {
            width: 100%;
            border-collapse: collapse;
        }

        thead {
            background: #343a40;
            color: white;
        }

        th, td {
            padding: 1rem;
            text-align: left;
        }

        tbody tr {
            border-bottom: 1px solid #dee2e6;
            transition: background 0.2s;
        }

        tbody tr:hover {
            background: #f8f9fa;
        }

        /* Status Badges */
        .badge {
            padding: 0.25rem 0.75rem;
            border-radius: 12px;
            font-size: 0.85rem;
            font-weight: 600;
            display: inline-block;
        }

        .badge-success {
            background: #d4edda;
            color: #155724;
        }

        .badge-danger {
            background: #f8d7da;
            color: #721c24;
        }

        .badge-warning {
            background: #fff3cd;
            color: #856404;
        }

        .badge-info {
            background: #d1ecf1;
            color: #0c5460;
        }

        .badge-secondary {
            background: #e2e3e5;
            color: #383d41;
        }

        /* Pagination */
        .pagination {
            display: flex;
            justify-content: center;
            align-items: center;
            gap: 1rem;
            padding: 1.5rem;
        }

        /* Job Details Modal */
        .modal {
            display: none;
            position: fixed;
            top: 0;
            left: 0;
            width: 100%;
            height: 100%;
            background: rgba(0,0,0,0.5);
            z-index: 1000;
        }

        .modal-content {
            background: white;
            margin: 2% auto;
            padding: 2rem;
            max-width: 800px;
            max-height: 90vh;
            overflow-y: auto;
            border-radius: 8px;
            position: relative;
        }

        .modal-close {
            position: absolute;
            top: 1rem;
            right: 1rem;
            font-size: 2rem;
            cursor: pointer;
            color: #6c757d;
        }

        .job-detail-section {
            margin-bottom: 1.5rem;
        }

        .job-detail-section h3 {
            color: #dc3545;
            margin-bottom: 0.75rem;
        }

        .detail-row {
            display: grid;
            grid-template-columns: 150px 1fr;
            padding: 0.5rem 0;
            border-bottom: 1px solid #f1f1f1;
        }

        .detail-label {
            font-weight: 600;
            color: #6c757d;
        }

        .error-box {
            background: #f8d7da;
            border: 1px solid #f5c6cb;
            border-radius: 4px;
            padding: 1rem;
            margin-top: 0.5rem;
            color: #721c24;
        }

        .actions {
            display: flex;
            gap: 0.5rem;
        }

        .loading {
            text-align: center;
            padding: 2rem;
            color: #6c757d;
        }
    </style>
</head>
<body>
    <!-- Sidebar -->
    <div class="sidebar">
        <h2>Admin Panel</h2>
        <ul>
            <li><a href="/admin/dashboard">Dashboard</a></li>
            <li><a href="/admin/users">Users</a></li>
            <li><a href="/admin/clipping-jobs" class="active">Clipping Jobs</a></li>
            <li><a href="/admin/clipping-activity">Activity</a></li>
            <li><a href="#" onclick="logout()">Logout</a></li>
        </ul>
    </div>

    <!-- Main Content -->
    <div class="main-content">
        <div class="header">
            <div>
                <h1>Clipping Jobs Management</h1>
                <p>View and manage all YouTube clipping jobs</p>
            </div>
            <button class="btn btn-secondary" onclick="loadJobs()">Refresh</button>
        </div>

        <!-- Filters -->
        <div class="filters">
            <div class="filter-group">
                <label>Status</label>
                <select id="statusFilter" onchange="filterChanged()">
                    <option value="">All Statuses</option>
                    <option value="pending">Pending</option>
                    <option value="downloading">Downloading</option>
                    <option value="analyzing">Analyzing</option>
                    <option value="extracting_clips">Extracting Clips</option>
                    <option value="posting">Posting</option>
                    <option value="completed">Completed</option>
                    <option value="failed">Failed</option>
                    <option value="cancelled">Cancelled</option>
                </select>
            </div>
            <div class="filter-group">
                <label>Sort By</label>
                <select id="sortFilter" onchange="filterChanged()">
                    <option value="created_desc">Newest First</option>
                    <option value="created_asc">Oldest First</option>
                    <option value="updated_desc">Recently Updated</option>
                    <option value="updated_asc">Least Recently Updated</option>
                </select>
            </div>
            <div class="filter-group">
                <label>&nbsp;</label>
                <button class="btn btn-primary" onclick="filterChanged()">Apply Filters</button>
            </div>
        </div>

        <!-- Jobs Table -->
        <div class="jobs-table">
            <table>
                <thead>
                    <tr>
                        <th>ID</th>
                        <th>User</th>
                        <th>Video</th>
                        <th>Channels</th>
                        <th>Status</th>
                        <th>Progress</th>
                        <th>Error</th>
                        <th>Retries</th>
                        <th>Created</th>
                        <th>Actions</th>
                    </tr>
                </thead>
                <tbody id="jobsTableBody">
                    <tr>
                        <td colspan="10" class="loading">Loading jobs...</td>
                    </tr>
                </tbody>
            </table>
        </div>

        <!-- Pagination -->
        <div class="pagination">
            <button class="btn btn-secondary" onclick="prevPage()" id="prevBtn">« Previous</button>
            <span id="pageInfo">Page 1 of 1</span>
            <button class="btn btn-secondary" onclick="nextPage()" id="nextBtn">Next »</button>
        </div>
    </div>

    <!-- Job Details Modal -->
    <div id="jobModal" class="modal">
        <div class="modal-content">
            <span class="modal-close" onclick="closeModal()">&times;</span>
            <div id="modalBody">
                <!-- Details loaded here -->
            </div>
        </div>
    </div>

    <script>
        // State
        let currentPage = 1;
        let totalPages = 1;
        let jobs = [];
        const authToken = localStorage.getItem('authToken');
        const user = JSON.parse(localStorage.getItem('user') || '{}');

        // Check auth
        if (!authToken || (!user.is_staff && !user.is_superuser)) {
            window.location.href = '/admin/login';
        }

        // Per-job WebSocket connections for live progress in table rows
        const adminJobSockets = {};

        function openAdminJobSocket(jobId) {
            if (adminJobSockets[jobId]) return;
            const proto = location.protocol === 'https:' ? 'wss' : 'ws';
            const ws = new WebSocket(proto + '://' + location.host + '/ws/clipping-jobs/' + jobId);
            adminJobSockets[jobId] = ws;

            ws.onmessage = function(event) {
                try { updateAdminTableRow(jobId, JSON.parse(event.data)); } catch(e) {}
            };

            ws.onclose = function() {
                delete adminJobSockets[jobId];
                const row = document.getElementById('job-row-' + jobId);
                if (row && row.dataset.status === 'processing') {
                    setTimeout(function() { openAdminJobSocket(jobId); }, 3000);
                }
            };
        }

        function updateAdminTableRow(jobId, update) {
            const s = update.status;
            if (!s) return;
            const status = s.status;
            const step = s.current_step || '';
            const pct = s.progress_percent != null ? Math.round(s.progress_percent) : null;

            const statusCell = document.getElementById('job-status-' + jobId);
            if (statusCell) statusCell.innerHTML = getStatusBadge(status === 'running' ? 'processing' : status);

            const progressCell = document.getElementById('job-progress-' + jobId);
            if (progressCell && pct !== null) {
                progressCell.innerHTML = pct + '%' +
                    (step ? '<br><small style="color: #6c757d;">' + step + '</small>' : '');
            }

            const row = document.getElementById('job-row-' + jobId);
            if (row) row.dataset.status = status;

            if (status === 'completed' || status === 'failed') {
                if (adminJobSockets[jobId]) { adminJobSockets[jobId].close(); delete adminJobSockets[jobId]; }
                setTimeout(loadJobs, 2000);
            }
        }

        // Load jobs
        async function loadJobs() {
            const status = document.getElementById('statusFilter').value;
            const sort = document.getElementById('sortFilter').value;

            const params = new URLSearchParams({
                page: currentPage,
                limit: 50
            });
            if (status) params.append('status', status);
            if (sort) params.append('sort', sort);

            try {
                const response = await fetch(`/api/admin/clipping/jobs?${params}`, {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();

                if (data.success) {
                    jobs = data.jobs;
                    totalPages = data.pagination.total_pages;
                    renderJobs();
                    updatePagination();
                }
            } catch (error) {
                console.error('Error loading jobs:', error);
                document.getElementById('jobsTableBody').innerHTML = 
                    '<tr><td colspan="10" class="loading">Error loading jobs</td></tr>';
            }
        }

        // Render jobs table
        function renderJobs() {
            const tbody = document.getElementById('jobsTableBody');
            if (jobs.length === 0) {
                tbody.innerHTML = '<tr><td colspan="10" class="loading">No jobs found</td></tr>';
                return;
            }

            tbody.innerHTML = jobs.map(job => `
                <tr id="job-row-${job.id}" data-status="${job.status}" onclick="viewJobDetails(${job.id})">
                    <td>#${job.id}</td>
                    <td>${job.username}</td>
                    <td>
                        <div>${job.source_video_title || job.source_video_id}</div>
                        <small style="color: #6c757d;">${job.source_video_id}</small>
                    </td>
                    <td>
                        <div>${job.source_channel_name}</div>
                        <small style="color: #6c757d;">→ ${job.dest_channel_name}</small>
                    </td>
                    <td id="job-status-${job.id}">${getStatusBadge(job.status)}</td>
                    <td id="job-progress-${job.id}">
                        ${job.progress_percent}%
                        ${job.current_step ? `<br><small style="color: #6c757d;">${job.current_step}</small>` : ''}
                    </td>
                    <td>${job.error_message ? '<span style="color: #dc3545">✗</span>' : ''}</td>
                    <td>${job.retry_count > 0 ? job.retry_count : '-'}</td>
                    <td>${formatDate(job.created_at)}</td>
                    <td class="actions" onclick="event.stopPropagation()">
                        ${job.status === 'failed' || job.status === 'cancelled' ?
                            `<button class="btn btn-success btn-sm" onclick="retryJob(${job.id})">Retry</button>` : ''}
                        ${job.status !== 'completed' && job.status !== 'failed' && job.status !== 'cancelled' ?
                            `<button class="btn btn-secondary btn-sm" onclick="cancelJob(${job.id})">Cancel</button>` : ''}
                    </td>
                </tr>
            `).join('');

            // Open per-job WebSocket for currently processing jobs
            jobs.filter(j => j.status === 'processing').forEach(job => {
                openAdminJobSocket(job.id);
            });
        }

        // View job details in modal
        async function viewJobDetails(jobId) {
            const modal = document.getElementById('jobModal');
            const modalBody = document.getElementById('modalBody');
            modal.style.display = 'block';
            modalBody.innerHTML = '<div class="loading">Loading job details...</div>';

            try {
                const response = await fetch(`/api/admin/clipping/jobs/${jobId}`, {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();

                if (data.success) {
                    const job = data.job;
                    const clips = data.clips;

                    modalBody.innerHTML = `
                        <h2>Job #${job.id} Details</h2>
                        
                        <div class="job-detail-section">
                            <h3>Basic Information</h3>
                            <div class="detail-row"><div class="detail-label">Status:</div><div>${getStatusBadge(job.status)}</div></div>
                            <div class="detail-row"><div class="detail-label">User:</div><div>${job.username} (${job.email})</div></div>
                            <div class="detail-row"><div class="detail-label">Video ID:</div><div>${job.source_video_id}</div></div>
                            <div class="detail-row"><div class="detail-label">Video Title:</div><div>${job.source_video_title || 'N/A'}</div></div>
                            <div class="detail-row"><div class="detail-label">Progress:</div><div>${job.progress_percent}%</div></div>
                            <div class="detail-row"><div class="detail-label">Current Step:</div><div>${job.current_step || 'N/A'}</div></div>
                        </div>

                        <div class="job-detail-section">
                            <h3>Channels</h3>
                            <div class="detail-row"><div class="detail-label">Source:</div><div>${job.source_channel_name}</div></div>
                            <div class="detail-row"><div class="detail-label">Destination:</div><div>${job.dest_channel_name}</div></div>
                            <div class="detail-row"><div class="detail-label">Clips Per Video:</div><div>${job.clips_per_video}</div></div>
                        </div>

                        <div class="job-detail-section">
                            <h3>Timing</h3>
                            <div class="detail-row"><div class="detail-label">Created:</div><div>${formatDate(job.created_at)}</div></div>
                            <div class="detail-row"><div class="detail-label">Updated:</div><div>${formatDate(job.updated_at)}</div></div>
                            <div class="detail-row"><div class="detail-label">Completed:</div><div>${formatDate(job.completed_at)}</div></div>
                            <div class="detail-row"><div class="detail-label">Duration:</div><div>${job.duration_minutes ? Math.round(job.duration_minutes) + ' minutes' : 'N/A'}</div></div>
                        </div>

                        <div class="job-detail-section">
                            <h3>Retry & Recovery</h3>
                            <div class="detail-row"><div class="detail-label">Retry Count:</div><div>${job.retry_count}</div></div>
                            <div class="detail-row"><div class="detail-label">Last Retry:</div><div>${formatDate(job.last_retry_at)}</div></div>
                            <div class="detail-row"><div class="detail-label">Stuck Count:</div><div>${job.stuck_detection_count}</div></div>
                            <div class="detail-row"><div class="detail-label">Claimed By:</div><div>${job.claimed_by || 'None'}</div></div>
                            <div class="detail-row"><div class="detail-label">Claimed At:</div><div>${formatDate(job.claimed_at)}</div></div>
                            <div class="detail-row"><div class="detail-label">Resume From:</div><div>${job.resume_from || 'N/A (full restart from Phase A)'}</div></div>
                        </div>

                        ${job.error_message ? `
                            <div class="job-detail-section">
                                <h3>Error</h3>
                                <div class="error-box">${job.error_message}</div>
                            </div>
                        ` : ''}

                        <div class="job-detail-section">
                            <h3>Extracted Clips (${clips.length})</h3>
                            ${clips.length === 0 ? '<p>No clips extracted yet</p>' : clips.map(clip => `
                                <div class="detail-row">
                                    <div class="detail-label">Clip ${clip.clip_number}:</div>
                                    <div>
                                        ${clip.ai_title || 'Untitled'} (${Math.round(clip.duration_seconds)}s)<br>
                                        <small>Upload: ${getStatusBadge(clip.upload_status)}</small>
                                        ${clip.youtube_video_id ? `<br><small>ID: ${clip.youtube_video_id}</small>` : ''}
                                    </div>
                                </div>
                            `).join('')}
                        </div>

                        <div class="actions" style="margin-top: 2rem;">
                            ${job.status === 'failed' || job.status === 'cancelled' ? 
                                `<button class="btn btn-success" onclick="retryJob(${job.id}); closeModal(); setTimeout(loadJobs, 500);">Retry Job</button>` : ''}
                            ${job.status !== 'completed' && job.status !== 'failed' && job.status !== 'cancelled' ? 
                                `<button class="btn btn-secondary" onclick="cancelJob(${job.id}); closeModal(); setTimeout(loadJobs, 500);">Cancel Job</button>` : ''}
                            <button class="btn btn-primary" onclick="closeModal()">Close</button>
                        </div>
                    `;
                }
            } catch (error) {
                console.error('Error loading job details:', error);
                modalBody.innerHTML = '<div class="loading">Error loading details</div>';
            }
        }

        function closeModal() {
            document.getElementById('jobModal').style.display = 'none';
        }

        // Retry job
        async function retryJob(jobId) {
            if (!confirm(`Retry job #${jobId}?`)) return;

            try {
                const response = await fetch(`/api/admin/clipping/jobs/${jobId}/retry`, {
                    method: 'POST',
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();
                alert(data.message);
                if (data.success) loadJobs();
            } catch (error) {
                alert('Error retrying job');
            }
        }

        // Cancel job
        async function cancelJob(jobId) {
            if (!confirm(`Cancel job #${jobId}? This cannot be undone.`)) return;

            try {
                const response = await fetch(`/api/admin/clipping/jobs/${jobId}/cancel`, {
                    method: 'POST',
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();
                alert(data.message);
                if (data.success) loadJobs();
            } catch (error) {
                alert('Error cancelling job');
            }
        }

        // Pagination
        function prevPage() {
            if (currentPage > 1) {
                currentPage--;
                loadJobs();
            }
        }

        function nextPage() {
            if (currentPage < totalPages) {
                currentPage++;
                loadJobs();
            }
        }

        function updatePagination() {
            document.getElementById('pageInfo').textContent = `Page ${currentPage} of ${totalPages}`;
            document.getElementById('prevBtn').disabled = currentPage === 1;
            document.getElementById('nextBtn').disabled = currentPage === totalPages;
        }

        function filterChanged() {
            currentPage = 1;
            loadJobs();
        }

        // Helpers
        function getStatusBadge(status) {
            const badges = {
                'completed': 'badge-success',
                'failed': 'badge-danger',
                'cancelled': 'badge-secondary',
                'no_clips_found': 'badge-secondary',
                'pending': 'badge-warning',
                'processing': 'badge-info',
                // old sequential pipeline step values (backward compat)
                'downloading': 'badge-info',
                'analyzing': 'badge-info',
                'extracting_clips': 'badge-info',
                'posting': 'badge-info'
            };
            return `<span class="badge ${badges[status] || 'badge-secondary'}">${status}</span>`;
        }

        function formatDate(dateStr) {
            if (!dateStr) return 'Never';
            return new Date(dateStr).toLocaleString();
        }

        function logout() {
            localStorage.removeItem('authToken');
            localStorage.removeItem('user');
            window.location.href = '/admin/login';
        }

        // Load on page load
        loadJobs();

        // Auto-refresh every 30 seconds
        setInterval(loadJobs, 30000);
    </script>
</body>
</html>
    "###;

    Html(html.to_string())
}

// ── Performance Tracking & Channel Health API endpoints ────────────────────

/// GET /api/admin/performance/viral-factors
/// Returns all viral factors with their performance scores, ordered by best-performing.
pub async fn admin_viral_factor_performance(
    Extension(state): Extension<Arc<crate::AppState>>,
) -> impl axum::response::IntoResponse {
    let rows = sqlx::query(
        "SELECT viral_factor, times_used, total_clips, avg_views, avg_like_rate,
                avg_comment_rate, avg_watch_percentage, performance_score, rank,
                last_calculated_at
         FROM viral_factor_performance
         WHERE viral_factor != 'initialization'
         ORDER BY performance_score DESC"
    )
    .fetch_all(&state.db_pool)
    .await;

    match rows {
        Ok(rows) => {
            let factors: Vec<serde_json::Value> = rows.iter().map(|r| {
                serde_json::json!({
                    "viral_factor": r.try_get::<String, _>("viral_factor").unwrap_or_default(),
                    "times_used": r.try_get::<i32, _>("times_used").unwrap_or(0),
                    "total_clips": r.try_get::<i32, _>("total_clips").unwrap_or(0),
                    "avg_views": r.try_get::<rust_decimal::Decimal, _>("avg_views").map(|v| v.to_string()).unwrap_or_default(),
                    "avg_like_rate": r.try_get::<rust_decimal::Decimal, _>("avg_like_rate").map(|v| v.to_string()).unwrap_or_default(),
                    "avg_comment_rate": r.try_get::<rust_decimal::Decimal, _>("avg_comment_rate").map(|v| v.to_string()).unwrap_or_default(),
                    "performance_score": r.try_get::<rust_decimal::Decimal, _>("performance_score").map(|v| v.to_string()).unwrap_or_default(),
                    "rank": r.try_get::<Option<i32>, _>("rank").unwrap_or(None),
                })
            }).collect();
            axum::Json(serde_json::json!({ "success": true, "factors": factors }))
        }
        Err(e) => axum::Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}

/// GET /api/admin/performance/channel-health
/// Returns health score and job stats for every source channel.
pub async fn admin_channel_health(
    Extension(state): Extension<Arc<crate::AppState>>,
) -> impl axum::response::IntoResponse {
    let rows = sqlx::query(
        "SELECT sc.id as source_id, sc.channel_name, sc.channel_id, sc.is_active,
                COALESCE(sch.jobs_attempted, 0) as jobs_attempted,
                COALESCE(sch.jobs_succeeded, 0) as jobs_succeeded,
                COALESCE(sch.health_score, 1.0) as health_score,
                sch.last_error,
                sch.last_error_at,
                sch.last_calculated_at
         FROM youtube_source_channels sc
         LEFT JOIN source_channel_health sch ON sch.source_channel_id = sc.id
         ORDER BY COALESCE(sch.health_score, 1.0) ASC, sc.channel_name"
    )
    .fetch_all(&state.db_pool)
    .await;

    match rows {
        Ok(rows) => {
            let channels: Vec<serde_json::Value> = rows.iter().map(|r| {
                serde_json::json!({
                    "source_id": r.try_get::<i32, _>("source_id").unwrap_or(0),
                    "channel_name": r.try_get::<String, _>("channel_name").unwrap_or_default(),
                    "channel_id": r.try_get::<String, _>("channel_id").unwrap_or_default(),
                    "is_active": r.try_get::<bool, _>("is_active").unwrap_or(true),
                    "jobs_attempted": r.try_get::<i64, _>("jobs_attempted").unwrap_or(0),
                    "jobs_succeeded": r.try_get::<i64, _>("jobs_succeeded").unwrap_or(0),
                    "health_score": r.try_get::<rust_decimal::Decimal, _>("health_score").map(|v| v.to_string()).unwrap_or("1.0".to_string()),
                    "last_error": r.try_get::<Option<String>, _>("last_error").unwrap_or(None),
                    "last_error_at": r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>("last_error_at").unwrap_or(None),
                })
            }).collect();
            axum::Json(serde_json::json!({ "success": true, "channels": channels }))
        }
        Err(e) => axum::Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}

/// GET /api/admin/performance/recommendations
/// Returns current active learning recommendations.
pub async fn admin_learning_recommendations(
    Extension(state): Extension<Arc<crate::AppState>>,
) -> impl axum::response::IntoResponse {
    let rows = sqlx::query(
        "SELECT recommendation_type, recommendation, confidence, supporting_data,
                is_active, created_at, updated_at
         FROM learning_recommendations
         WHERE is_active = true
         ORDER BY confidence DESC"
    )
    .fetch_all(&state.db_pool)
    .await;

    match rows {
        Ok(rows) => {
            let recs: Vec<serde_json::Value> = rows.iter().map(|r| {
                serde_json::json!({
                    "type": r.try_get::<String, _>("recommendation_type").unwrap_or_default(),
                    "recommendation": r.try_get::<String, _>("recommendation").unwrap_or_default(),
                    "confidence": r.try_get::<rust_decimal::Decimal, _>("confidence").map(|v| v.to_string()).unwrap_or_default(),
                    "updated_at": r.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at").ok(),
                })
            }).collect();
            axum::Json(serde_json::json!({ "success": true, "recommendations": recs }))
        }
        Err(e) => axum::Json(serde_json::json!({ "success": false, "error": e.to_string() })),
    }
}
