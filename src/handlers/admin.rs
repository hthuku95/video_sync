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
use uuid::Uuid;

pub fn admin_routes() -> Router {
    // HTML pages - public routes with JavaScript authentication
    let public_admin = Router::new()
        .route("/admin", get(admin_login_page))
        .route("/admin/login", get(admin_login_page))
        .route("/admin/dashboard", get(admin_dashboard))
        .route("/admin/users", get(admin_users_list))
        .route("/admin/users/:id", get(admin_user_detail))
        .route("/admin/clipping-activity", get(admin_clipping_activity_page))
        .route("/admin/clipping-jobs", get(admin_clipping_jobs_page))
        .route("/admin/performance", get(admin_performance_page))
        .route("/admin/test-runs", get(admin_test_runs_page))
        .route("/admin/test-runs/:id", get(admin_test_run_detail_page))
        .route("/admin/deliveries", get(admin_deliveries_page))
        .route("/admin/monetization-guide", get(admin_monetization_guide_page))
        .route("/admin/revenue-ledger", get(admin_revenue_ledger_page))
        .route("/admin/how-it-works", get(admin_how_it_works_page))
        .route("/delivery/:id", get(delivery_page))
        // x402 paywall endpoints — public on purpose; auth happens via the
        // signed USDC payment in the X-Payment header, not via JWT.
        .route("/delivery/:id/unlock-spec", get(delivery_unlock_spec))
        .route("/delivery/:id/unlock", post(delivery_unlock));
    
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
        .route("/api/admin/clipping/throughput", get(admin_clipping_throughput))
        .route("/api/admin/performance/viral-factors", get(admin_viral_factor_performance))
        .route("/api/admin/performance/channel-health", get(admin_channel_health))
        .route("/api/admin/performance/recommendations", get(admin_learning_recommendations))
        .route("/api/admin/performance/thumbnails", get(admin_thumbnail_stats))
        .route("/api/admin/test-runs", get(api_list_test_runs).post(api_trigger_test_run))
        .route("/api/admin/test-runs/:id", get(api_get_test_run))
        .route("/api/admin/manual-clipping-tests/trigger", post(api_trigger_manual_clipping_test))
        .route("/api/admin/deliveries", get(api_list_deliveries).post(api_create_delivery))
        .route("/api/admin/revenue-ledger", get(api_revenue_ledger))
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
            <li><a href="/admin/performance">📈 Performance</a></li>
            <li><a href="/admin/test-runs">🧪 Portfolio Tests</a></li>
            <li><a href="/admin/prospect-finder">🎯 Prospect Finder</a></li>
            <li><a href="/admin/monetization-guide">💰 Monetization Guide</a></li>
            <li><a href="/admin/revenue-ledger">💸 Revenue Ledger</a></li>
            <li><a href="/admin/how-it-works">📘 How We Work</a></li>
            <li><a href="#" onclick="showWhitelist()">🛡️ Whitelist</a></li>
            <li><a href="#" onclick="showYoutube()">🎥 YouTube Features</a></li>
            <li><a href="#" onclick="showPricing()">💲 Model Pricing</a></li>
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
            <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:1rem">
                <h2>Recent Users</h2>
                <a href="/admin/users" class="btn">View all users →</a>
            </div>
            <table>
                <thead>
                    <tr>
                        <th>Username</th>
                        <th>Email</th>
                        <th>Status</th>
                        <th>Role</th>
                        <th>Subscription</th>
                        <th>Joined</th>
                        <th>Actions</th>
                    </tr>
                </thead>
                <tbody id="recentUsers">
                    <tr><td colspan="7" style="text-align: center;">Loading...</td></tr>
                </tbody>
            </table>
        </div>
    </div>
    
    <script>
        // Check admin authentication
        const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
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
                    const subBadge = (s) => {
                        if (s === 'active')        return 'badge-success';
                        if (s === 'grandfathered') return 'badge-success';
                        if (s === 'trial')         return 'badge-warning';
                        if (s === 'expired')       return 'badge-danger';
                        return '';
                    };
                    tbody.innerHTML = data.users.map(user => {
                        const status = user.subscription_status || '—';
                        return `
                        <tr>
                            <td>${user.username}</td>
                            <td>${user.email}</td>
                            <td><span class="badge ${user.is_active ? 'badge-success' : 'badge-danger'}">${user.is_active ? 'Active' : 'Inactive'}</span></td>
                            <td><span class="badge ${user.is_superuser ? 'badge-danger' : user.is_staff ? 'badge-warning' : 'badge-success'}">${user.is_superuser ? 'Superuser' : user.is_staff ? 'Staff' : 'User'}</span></td>
                            <td><span class="badge ${subBadge(status)}">${status}</span></td>
                            <td>${new Date(user.created_at).toLocaleDateString()}</td>
                            <td><a href="/admin/users/${user.id}" class="btn" style="padding: 0.25rem 0.5rem; font-size: 0.8rem;">View</a></td>
                        </tr>`;
                    }).join('');
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

    // Full column set including the subscription columns added by the
    // 20260415000001 migration — surfaces trial status + paid-until on
    // the admin users page without having to modify UserResponse.
    let select = "SELECT id, email, username, is_active, is_superuser, is_staff, \
                  created_at, subscription_status, subscription_tier, \
                  trial_ends_at, subscription_active_until, last_payment_at \
                  FROM users";
    let mut query       = select.to_string();
    let mut count_query = "SELECT COUNT(*) FROM users".to_string();

    if params.search.is_some() {
        let where_clause = " WHERE username ILIKE $1 OR email ILIKE $1";
        query.push_str(where_clause);
        count_query.push_str(where_clause);
    }
    query.push_str(&format!(" ORDER BY created_at DESC LIMIT {} OFFSET {}", limit, offset));

    let rows = if let Some(search) = &params.search {
        let term = format!("%{}%", search);
        sqlx::query(&query).bind(&term).fetch_all(&state.db_pool).await
    } else {
        sqlx::query(&query).fetch_all(&state.db_pool).await
    }
    .map_err(|e| { tracing::error!("admin_users_api query failed: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;

    let users: Vec<serde_json::Value> = rows.iter().map(|r| {
        let to_iso = |t: Option<chrono::DateTime<chrono::Utc>>| t.map(|d| d.to_rfc3339());
        json!({
            "id":                        r.get::<i32, _>("id"),
            "email":                     r.get::<String, _>("email"),
            "username":                  r.get::<String, _>("username"),
            "is_active":                 r.get::<bool, _>("is_active"),
            "is_superuser":              r.get::<bool, _>("is_superuser"),
            "is_staff":                  r.get::<bool, _>("is_staff"),
            "created_at":                to_iso(r.try_get("created_at").ok()),
            "subscription_status":       r.try_get::<Option<String>, _>("subscription_status").ok().flatten(),
            "subscription_tier":         r.try_get::<Option<String>, _>("subscription_tier").ok().flatten(),
            "trial_ends_at":             to_iso(r.try_get("trial_ends_at").ok().flatten()),
            "subscription_active_until": to_iso(r.try_get("subscription_active_until").ok().flatten()),
            "last_payment_at":           to_iso(r.try_get("last_payment_at").ok().flatten()),
        })
    }).collect();

    let total_count: i64 = if let Some(search) = &params.search {
        let term = format!("%{}%", search);
        sqlx::query_scalar(&count_query).bind(&term).fetch_one(&state.db_pool).await
    } else {
        sqlx::query_scalar(&count_query).fetch_one(&state.db_pool).await
    }
    .map_err(|e| { tracing::error!("admin_users_api count query failed: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;

    Ok(Json(json!({
        "success": true,
        "users":   users,
        "pagination": {
            "page":        page,
            "limit":       limit,
            "total":       total_count,
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
        password_hash: "".to_string(),
        is_active: user_row.get("is_active"),
        is_superuser: user_row.get("is_superuser"),
        is_staff: user_row.get("is_staff"),
        is_clipper: false,
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
            <li><a href="/admin/performance">📈 Performance</a></li>
            <li><a href="/admin/test-runs">🧪 Portfolio Tests</a></li>
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
                        <th>Subscription</th>
                        <th>Trial / Paid Until</th>
                        <th>Last Payment</th>
                        <th>Created</th>
                        <th>Actions</th>
                    </tr>
                </thead>
                <tbody id="usersTable">
                    <tr><td colspan="10" style="text-align: center;">Loading...</td></tr>
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
        const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
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

                if (response.status === 401 || response.status === 403) {
                    window.location.href = '/admin/login';
                    return;
                }

                const data = await response.json();

                if (data.success) {
                    renderUsers(data.users);
                    totalPages = data.pagination.total_pages;
                    updatePagination();
                } else {
                    document.getElementById('usersTable').innerHTML =
                        `<tr><td colspan="7" style="text-align:center;color:#dc3545;">Error: ${data.message || 'Failed to load users'}</td></tr>`;
                }
            } catch (error) {
                console.error('Error loading users:', error);
                document.getElementById('usersTable').innerHTML =
                    `<tr><td colspan="7" style="text-align:center;color:#dc3545;">Network error — check console</td></tr>`;
            }
        }

        function renderUsers(users) {
            const tbody = document.getElementById('usersTable');

            if (users.length === 0) {
                tbody.innerHTML = '<tr><td colspan="10" style="text-align: center;">No users found</td></tr>';
                return;
            }

            const subBadgeClass = (status) => {
                if (status === 'active')        return 'badge-success';
                if (status === 'grandfathered') return 'badge-success';
                if (status === 'trial')         return 'badge-warning';
                if (status === 'expired')       return 'badge-danger';
                return '';
            };
            const fmtDate = (iso) => iso ? new Date(iso).toLocaleDateString() : '—';

            tbody.innerHTML = users.map(user => {
                const status   = user.subscription_status || '—';
                const untilDate = user.subscription_active_until || user.trial_ends_at;
                return `
                <tr>
                    <td>${user.id}</td>
                    <td>${user.username}</td>
                    <td>${user.email}</td>
                    <td><span class="badge ${user.is_active ? 'badge-success' : 'badge-danger'}">${user.is_active ? 'Active' : 'Inactive'}</span></td>
                    <td><span class="badge ${user.is_superuser ? 'badge-danger' : user.is_staff ? 'badge-warning' : 'badge-success'}">${user.is_superuser ? 'Superuser' : user.is_staff ? 'Staff' : 'User'}</span></td>
                    <td><span class="badge ${subBadgeClass(status)}">${status}</span></td>
                    <td>${fmtDate(untilDate)}</td>
                    <td>${fmtDate(user.last_payment_at)}</td>
                    <td>${fmtDate(user.created_at)}</td>
                    <td><a href="/admin/users/${user.id}" class="btn btn-sm">View</a></td>
                </tr>`;
            }).join('');
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
            <li><a href="/admin/performance">📈 Performance</a></li>
            <li><a href="/admin/test-runs">🧪 Portfolio Tests</a></li>
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
        const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
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

    // Query 3: Extracted clips with analytics for those jobs
    let job_ids: Vec<i32> = {
        use sqlx::Row;
        job_rows.iter().map(|r| r.get::<i32, _>("id")).collect()
    };

    let clip_rows = if job_ids.is_empty() {
        vec![]
    } else {
        sqlx::query(
            r#"
            SELECT
                ec.id,
                ec.clipping_job_id,
                ec.clip_number,
                ec.ai_title,
                ec.ai_description,
                ec.ai_confidence_score,
                ec.viral_factors,
                ec.youtube_video_id,
                ec.youtube_url,
                ec.upload_status,
                ec.published_at,
                ec.views_24h,
                ec.likes_24h,
                ec.comments_24h,
                ec.start_time_seconds,
                ec.end_time_seconds,
                ec.duration_seconds,
                COALESCE(yva.views, ec.views_24h, 0) AS total_views,
                COALESCE(yva.likes, ec.likes_24h, 0) AS total_likes,
                COALESCE(yva.comments, ec.comments_24h, 0) AS total_comments
            FROM extracted_clips ec
            LEFT JOIN LATERAL (
                SELECT views, likes, comments
                FROM youtube_video_analytics
                WHERE youtube_video_id = ec.youtube_video_id
                ORDER BY metric_date DESC
                LIMIT 1
            ) yva ON ec.youtube_video_id IS NOT NULL
            WHERE ec.clipping_job_id = ANY($1)
            ORDER BY ec.clipping_job_id, ec.clip_number
            "#
        )
        .bind(&job_ids)
        .fetch_all(&state.db_pool)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch clips: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
    };

    let clips: Vec<serde_json::Value> = clip_rows.iter().map(|row| {
        use sqlx::Row;
        json!({
            "id": row.get::<i32, _>("id"),
            "clipping_job_id": row.get::<i32, _>("clipping_job_id"),
            "clip_number": row.get::<i32, _>("clip_number"),
            "ai_title": row.get::<Option<String>, _>("ai_title"),
            "ai_description": row.get::<Option<String>, _>("ai_description"),
            "ai_confidence_score": row.get::<Option<f64>, _>("ai_confidence_score"),
            "viral_factors": row.get::<Option<Vec<String>>, _>("viral_factors"),
            "youtube_video_id": row.get::<Option<String>, _>("youtube_video_id"),
            "youtube_url": row.get::<Option<String>, _>("youtube_url"),
            "upload_status": row.get::<String, _>("upload_status"),
            "published_at": row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("published_at"),
            "views_24h": row.get::<i32, _>("views_24h"),
            "likes_24h": row.get::<i32, _>("likes_24h"),
            "comments_24h": row.get::<i32, _>("comments_24h"),
            "start_time_seconds": row.get::<f64, _>("start_time_seconds"),
            "end_time_seconds": row.get::<f64, _>("end_time_seconds"),
            "duration_seconds": row.get::<f64, _>("duration_seconds"),
            "total_views": row.get::<i64, _>("total_views"),
            "total_likes": row.get::<i64, _>("total_likes"),
            "total_comments": row.get::<i64, _>("total_comments"),
        })
    }).collect();

    Ok(Json(json!({
        "success": true,
        "linkages": linkages,
        "recent_jobs": recent_jobs,
        "clips": clips
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
        <script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.0/dist/chart.umd.min.js"></script>
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
                <li><a href="/admin/performance">📈 Performance</a></li>
                <li><a href="/admin/test-runs">🧪 Portfolio Tests</a></li>
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
                <h2>Jobs Per Hour — Last 24h</h2>
                <div style="position:relative;height:280px;">
                    <canvas id="throughputChart"></canvas>
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
                <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:1.5rem;">
                    <h2>Details: <span id="detailUsername"></span></h2>
                    <button onclick="document.getElementById('userDetails').style.display='none'" style="background:none;border:1px solid #dee2e6;padding:0.4rem 0.8rem;border-radius:5px;cursor:pointer;">✕ Close</button>
                </div>

                <!-- Linkages -->
                <h3 style="margin-bottom:1rem;">📡 Channel Linkages</h3>
                <div id="linkagesGrid" style="display:grid;grid-template-columns:repeat(auto-fill,minmax(340px,1fr));gap:1rem;margin-bottom:2rem;"></div>

                <!-- Jobs & Clips -->
                <h3 style="margin-bottom:1rem;">🎬 Recent Jobs &amp; Extracted Clips</h3>
                <div id="jobsContainer"></div>
            </div>
        </div>

        <script>
            const token = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
            const user = JSON.parse(localStorage.getItem('user') || '{}');
            if (!token || (!user.is_staff && !user.is_superuser)) {
                window.location.href = '/admin/login';
            }

            loadClippingStats();
            loadThroughputChart();

            async function loadThroughputChart() {
                try {
                    const response = await fetch('/api/admin/clipping/throughput', {
                        headers: { 'Authorization': `Bearer ${token}` }
                    });
                    const json = await response.json();
                    const rows = json.data || [];

                    const labels = rows.map(r => {
                        const d = new Date(r.hour);
                        return d.getHours().toString().padStart(2, '0') + ':00';
                    });
                    const completed = rows.map(r => r.completed);
                    const failed = rows.map(r => r.failed);

                    if (window.throughputChartInstance) {
                        window.throughputChartInstance.destroy();
                    }

                    const ctx = document.getElementById('throughputChart').getContext('2d');
                    window.throughputChartInstance = new Chart(ctx, {
                        type: 'line',
                        data: {
                            labels,
                            datasets: [
                                {
                                    label: 'Completed',
                                    data: completed,
                                    borderColor: '#28a745',
                                    backgroundColor: 'rgba(40,167,69,0.1)',
                                    tension: 0.3,
                                    fill: false,
                                },
                                {
                                    label: 'Failed',
                                    data: failed,
                                    borderColor: '#dc3545',
                                    backgroundColor: 'rgba(220,53,69,0.1)',
                                    tension: 0.3,
                                    fill: false,
                                },
                            ],
                        },
                        options: {
                            responsive: true,
                            maintainAspectRatio: false,
                            plugins: { legend: { position: 'top' } },
                            scales: {
                                y: { beginAtZero: true, ticks: { stepSize: 1 } },
                            },
                        },
                    });
                } catch (error) {
                    console.error('Failed to load throughput chart:', error);
                }
            }

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
                    if (response.status === 401) { window.location.href = '/admin/login'; return; }
                    const data = await response.json();

                    document.getElementById('userDetails').style.display = 'block';
                    document.getElementById('detailUsername').textContent = username;

                    // --- Linkages ---
                    const linkagesGrid = document.getElementById('linkagesGrid');
                    if (!data.linkages || data.linkages.length === 0) {
                        linkagesGrid.innerHTML = '<p style="color:#6c757d;">No channel linkages set up.</p>';
                    } else {
                        linkagesGrid.innerHTML = data.linkages.map(l => `
                            <div style="border:1px solid #dee2e6;border-radius:8px;padding:1rem;background:#f8f9fa;">
                                <div style="display:flex;align-items:center;gap:0.5rem;margin-bottom:0.75rem;">
                                    ${l.source_thumbnail ? `<img src="${l.source_thumbnail}" style="width:32px;height:32px;border-radius:50%;object-fit:cover;" onerror="this.style.display='none'">` : ''}
                                    <div>
                                        <strong style="font-size:0.9rem;">${l.source_channel_name}</strong>
                                        <div style="font-size:0.75rem;color:#6c757d;">Source</div>
                                    </div>
                                    <span style="margin:0 0.5rem;color:#6c757d;">→</span>
                                    ${l.dest_thumbnail ? `<img src="${l.dest_thumbnail}" style="width:32px;height:32px;border-radius:50%;object-fit:cover;" onerror="this.style.display='none'">` : ''}
                                    <div>
                                        <strong style="font-size:0.9rem;">${l.dest_channel_name}</strong>
                                        <div style="font-size:0.75rem;color:#6c757d;">Destination</div>
                                    </div>
                                    <span class="badge ${l.is_active ? 'badge-success' : 'badge-danger'}" style="margin-left:auto;">${l.is_active ? 'Active' : 'Paused'}</span>
                                </div>
                                <div style="display:flex;gap:1.5rem;font-size:0.85rem;color:#495057;">
                                    <span>📊 Generated: <strong>${l.total_clips_generated}</strong></span>
                                    <span>📤 Posted: <strong>${l.total_clips_posted}</strong></span>
                                    <span>🎯 Per Video: <strong>${l.clips_per_video || 'auto'}</strong></span>
                                </div>
                            </div>
                        `).join('');
                    }

                    // --- Build clips lookup by job_id ---
                    const clipsByJob = {};
                    (data.clips || []).forEach(c => {
                        if (!clipsByJob[c.clipping_job_id]) clipsByJob[c.clipping_job_id] = [];
                        clipsByJob[c.clipping_job_id].push(c);
                    });

                    // --- Jobs ---
                    const jobsContainer = document.getElementById('jobsContainer');
                    if (!data.recent_jobs || data.recent_jobs.length === 0) {
                        jobsContainer.innerHTML = '<p style="color:#6c757d;padding:1rem;">No jobs found for this user.</p>';
                    } else {
                        jobsContainer.innerHTML = data.recent_jobs.map(job => {
                            const jobClips = clipsByJob[job.id] || [];
                            const clipsHtml = jobClips.length === 0 ? '' : `
                                <tr id="clips-${job.id}" style="display:none;">
                                    <td colspan="6" style="background:#f8f9fa;padding:0;">
                                        <table style="width:100%;margin:0;border-radius:0;">
                                            <thead>
                                                <tr style="background:#e9ecef;">
                                                    <th style="padding:0.5rem 1rem;font-size:0.8rem;">#</th>
                                                    <th style="padding:0.5rem 1rem;font-size:0.8rem;">AI Title</th>
                                                    <th style="padding:0.5rem 1rem;font-size:0.8rem;">Upload Status</th>
                                                    <th style="padding:0.5rem 1rem;font-size:0.8rem;">👁 Views</th>
                                                    <th style="padding:0.5rem 1rem;font-size:0.8rem;">👍 Likes</th>
                                                    <th style="padding:0.5rem 1rem;font-size:0.8rem;">💬 Comments</th>
                                                    <th style="padding:0.5rem 1rem;font-size:0.8rem;">Duration</th>
                                                    <th style="padding:0.5rem 1rem;font-size:0.8rem;">YouTube</th>
                                                </tr>
                                            </thead>
                                            <tbody>
                                                ${jobClips.map(c => `
                                                    <tr style="border-bottom:1px solid #dee2e6;">
                                                        <td style="padding:0.5rem 1rem;font-size:0.85rem;">${c.clip_number}</td>
                                                        <td style="padding:0.5rem 1rem;font-size:0.85rem;max-width:220px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;" title="${c.ai_title || ''}">${c.ai_title || '<em style="color:#6c757d">No title</em>'}</td>
                                                        <td style="padding:0.5rem 1rem;"><span class="badge badge-${c.upload_status === 'published' ? 'success' : c.upload_status === 'failed' ? 'danger' : 'warning'}">${c.upload_status}</span></td>
                                                        <td style="padding:0.5rem 1rem;font-size:0.85rem;font-weight:bold;">${c.total_views > 0 ? c.total_views.toLocaleString() : '<span style="color:#6c757d">—</span>'}</td>
                                                        <td style="padding:0.5rem 1rem;font-size:0.85rem;">${c.total_likes > 0 ? c.total_likes.toLocaleString() : '<span style="color:#6c757d">—</span>'}</td>
                                                        <td style="padding:0.5rem 1rem;font-size:0.85rem;">${c.total_comments > 0 ? c.total_comments.toLocaleString() : '<span style="color:#6c757d">—</span>'}</td>
                                                        <td style="padding:0.5rem 1rem;font-size:0.85rem;">${Math.round(c.duration_seconds)}s</td>
                                                        <td style="padding:0.5rem 1rem;">
                                                            ${c.youtube_url ? `<a href="${c.youtube_url}" target="_blank" style="color:#dc3545;font-size:0.8rem;text-decoration:none;border:1px solid #dc3545;padding:2px 6px;border-radius:3px;">▶ Watch</a>` : '<span style="color:#6c757d;font-size:0.8rem;">Not uploaded</span>'}
                                                        </td>
                                                    </tr>
                                                `).join('')}
                                            </tbody>
                                        </table>
                                    </td>
                                </tr>
                            `;

                            const hasClips = jobClips.length > 0;
                            const toggleBtn = hasClips
                                ? `<button onclick="toggleClips(${job.id})" style="background:none;border:1px solid #dee2e6;padding:2px 8px;border-radius:3px;cursor:pointer;font-size:0.8rem;" id="toggle-${job.id}">▼ ${jobClips.length} clip${jobClips.length !== 1 ? 's' : ''}</button>`
                                : `<span style="color:#6c757d;font-size:0.8rem;">no clips</span>`;

                            return `
                                <tr onclick="${hasClips ? `toggleClips(${job.id})` : ''}" style="cursor:${hasClips ? 'pointer' : 'default'};">
                                    <td style="padding:0.75rem 1rem;font-size:0.85rem;">#${job.id}</td>
                                    <td style="padding:0.75rem 1rem;font-size:0.85rem;max-width:200px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;">
                                        <a href="https://youtube.com/watch?v=${job.source_video_id}" target="_blank" onclick="event.stopPropagation()" style="color:#dc3545;text-decoration:none;">${job.source_video_title || job.source_video_id}</a>
                                    </td>
                                    <td style="padding:0.75rem 1rem;"><span class="badge badge-${getStatusColor(job.status)}">${job.status}</span></td>
                                    <td style="padding:0.75rem 1rem;font-size:0.85rem;">${job.source_channel}</td>
                                    <td style="padding:0.75rem 1rem;font-size:0.85rem;">${formatDate(job.created_at)}</td>
                                    <td style="padding:0.75rem 1rem;">${toggleBtn}</td>
                                </tr>
                                ${clipsHtml}
                                ${job.error_message ? `<tr><td colspan="6" style="padding:0.25rem 1rem 0.75rem;font-size:0.8rem;color:#dc3545;">⚠️ ${job.error_message.substring(0, 120)}</td></tr>` : ''}
                            `;
                        }).join('');

                        // Wrap in table
                        jobsContainer.innerHTML = `
                            <table style="width:100%;border-collapse:collapse;">
                                <thead>
                                    <tr style="background:#f8f9fa;">
                                        <th style="padding:0.75rem 1rem;font-size:0.85rem;">Job</th>
                                        <th style="padding:0.75rem 1rem;font-size:0.85rem;">Source Video</th>
                                        <th style="padding:0.75rem 1rem;font-size:0.85rem;">Status</th>
                                        <th style="padding:0.75rem 1rem;font-size:0.85rem;">Channel</th>
                                        <th style="padding:0.75rem 1rem;font-size:0.85rem;">Created</th>
                                        <th style="padding:0.75rem 1rem;font-size:0.85rem;">Clips</th>
                                    </tr>
                                </thead>
                                <tbody style="border:1px solid #dee2e6;">
                                    ${jobsContainer.innerHTML}
                                </tbody>
                            </table>
                        `;
                    }

                    document.getElementById('userDetails').scrollIntoView({ behavior: 'smooth' });
                } catch (error) {
                    console.error('Failed to load user details:', error);
                    document.getElementById('jobsContainer').innerHTML = '<p style="color:#dc3545;">Failed to load details — check console.</p>';
                }
            }

            function toggleClips(jobId) {
                const row = document.getElementById('clips-' + jobId);
                const btn = document.getElementById('toggle-' + jobId);
                if (!row) return;
                const visible = row.style.display !== 'none';
                row.style.display = visible ? 'none' : 'table-row';
                if (btn) btn.textContent = btn.textContent.replace(visible ? '▲' : '▼', visible ? '▼' : '▲');
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
            setInterval(() => { loadClippingStats(); loadThroughputChart(); }, 30000);
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
            "custom_thumbnail_path": row.get::<Option<String>, _>("custom_thumbnail_path"),
            "thumbnail_generation_method": row.get::<Option<String>, _>("thumbnail_generation_method"),
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
        "SELECT current_step FROM clipping_jobs WHERE id = $1 AND status IN ('failed', 'cancelled', 'discarded')"
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
                "Job is not in failed, cancelled, or discarded status"
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
         WHERE id = $1 AND status IN ('failed', 'cancelled', 'discarded')
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
            "message": "Job not found or not in failed/cancelled/discarded status"
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

/// GET /api/admin/clipping/throughput
/// Returns completed/failed job counts per hour for the last 24 hours.
pub async fn admin_clipping_throughput(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    #[derive(sqlx::FromRow)]
    struct ThroughputRow {
        hour_bucket: chrono::DateTime<chrono::Utc>,
        completed: i64,
        failed: i64,
    }

    let rows = sqlx::query_as::<_, ThroughputRow>(
        r#"
        WITH hours AS (
            SELECT generate_series(
                date_trunc('hour', NOW() - INTERVAL '23 hours'),
                date_trunc('hour', NOW()),
                '1 hour'::interval
            ) AS hour_bucket
        ),
        job_counts AS (
            SELECT
                date_trunc('hour', completed_at) AS hour_bucket,
                COUNT(*) FILTER (WHERE status = 'completed') AS completed,
                COUNT(*) FILTER (WHERE status = 'failed')    AS failed
            FROM clipping_jobs
            WHERE completed_at >= NOW() - INTERVAL '24 hours'
            GROUP BY 1
        )
        SELECT
            h.hour_bucket,
            COALESCE(jc.completed, 0) AS completed,
            COALESCE(jc.failed, 0)    AS failed
        FROM hours h
        LEFT JOIN job_counts jc ON jc.hour_bucket = h.hour_bucket
        ORDER BY h.hour_bucket ASC
        "#
    )
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch throughput data: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let data: Vec<serde_json::Value> = rows.iter().map(|r| {
        json!({
            "hour": r.hour_bucket,
            "completed": r.completed,
            "failed": r.failed,
        })
    }).collect();

    Ok(Json(json!({ "success": true, "data": data })))
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
            <li><a href="/admin/performance">Performance</a></li>
            <li><a href="/admin/test-runs">🧪 Portfolio Tests</a></li>
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
                    <option value="discarded">Discarded</option>
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
        const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
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
                        ${job.status === 'discarded' ?
                            `<button class="btn btn-warning btn-sm" onclick="retryJob(${job.id})">Force Retry</button>` : ''}
                        ${job.status !== 'completed' && job.status !== 'failed' && job.status !== 'cancelled' && job.status !== 'discarded' ?
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
                            ${job.status === 'discarded' ?
                                `<button class="btn btn-warning" onclick="retryJob(${job.id}); closeModal(); setTimeout(loadJobs, 500);">Force Retry</button>` : ''}
                            ${job.status !== 'completed' && job.status !== 'failed' && job.status !== 'cancelled' && job.status !== 'discarded' ?
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

/// Thumbnail generation statistics for the performance dashboard.
/// Returns counts by generation method and an AI success rate.
pub async fn admin_thumbnail_stats(
    Extension(state): Extension<Arc<crate::AppState>>,
) -> impl axum::response::IntoResponse {
    let rows = sqlx::query(
        "SELECT
            COALESCE(thumbnail_generation_method, 'none') AS method,
            COUNT(*) AS count
         FROM extracted_clips
         GROUP BY thumbnail_generation_method
         ORDER BY count DESC"
    )
    .fetch_all(&state.db_pool)
    .await;

    match rows {
        Ok(rows) => {
            let stats: Vec<serde_json::Value> = rows.iter().map(|r| {
                serde_json::json!({
                    "method": r.try_get::<String, _>("method").unwrap_or_default(),
                    "count": r.try_get::<i64, _>("count").unwrap_or(0),
                })
            }).collect();

            let total: i64 = stats.iter()
                .map(|s| s["count"].as_i64().unwrap_or(0))
                .sum();
            let ai_count: i64 = stats.iter()
                .filter(|s| s["method"].as_str().map_or(false, |m| m.contains("ai") || m.contains("hybrid")))
                .map(|s| s["count"].as_i64().unwrap_or(0))
                .sum();
            let ai_rate = if total > 0 { ai_count * 100 / total } else { 0 };

            axum::Json(serde_json::json!({
                "success": true,
                "stats": stats,
                "total_clips": total,
                "ai_generated_count": ai_count,
                "ai_success_rate_pct": ai_rate,
            }))
        }
        Err(e) => {
            axum::Json(serde_json::json!({ "success": false, "error": e.to_string() }))
        }
    }
}

// ── Performance Dashboard UI ────────────────────────────────────────────────

pub async fn admin_performance_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Performance Dashboard - Admin</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
            background: #f8f9fa;
            color: #343a40;
        }
        .sidebar {
            position: fixed;
            top: 0; left: 0;
            width: 250px; height: 100vh;
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
        .sidebar ul { list-style: none; }
        .sidebar li a {
            display: block;
            padding: 0.75rem 1.5rem;
            color: white;
            text-decoration: none;
            transition: background 0.2s;
        }
        .sidebar li a:hover, .sidebar li a.active { background: #495057; }
        .sidebar li a.active { border-left: 3px solid #dc3545; }
        .main-content { margin-left: 250px; padding: 2rem; }
        .page-header {
            background: white;
            padding: 1.5rem;
            border-radius: 8px;
            box-shadow: 0 2px 4px rgba(0,0,0,0.1);
            margin-bottom: 2rem;
            display: flex;
            justify-content: space-between;
            align-items: center;
        }
        .page-header h1 { font-size: 1.75rem; }
        .page-header p { color: #6c757d; margin-top: 0.25rem; }
        .tabs {
            display: flex;
            gap: 0;
            margin-bottom: 2rem;
            border-bottom: 2px solid #dee2e6;
        }
        .tab-btn {
            padding: 0.75rem 1.5rem;
            border: none;
            background: none;
            cursor: pointer;
            font-size: 0.95rem;
            color: #6c757d;
            border-bottom: 3px solid transparent;
            margin-bottom: -2px;
            transition: all 0.2s;
        }
        .tab-btn:hover { color: #343a40; }
        .tab-btn.active { color: #dc3545; border-bottom-color: #dc3545; font-weight: 600; }
        .tab-content { display: none; }
        .tab-content.active { display: block; }
        .card {
            background: white;
            border-radius: 8px;
            box-shadow: 0 2px 4px rgba(0,0,0,0.1);
            padding: 1.5rem;
            margin-bottom: 1.5rem;
        }
        .card-title {
            font-size: 1.1rem;
            font-weight: 600;
            margin-bottom: 1rem;
            padding-bottom: 0.75rem;
            border-bottom: 1px solid #dee2e6;
            color: #343a40;
        }
        table { width: 100%; border-collapse: collapse; }
        thead th {
            background: #f8f9fa;
            padding: 0.75rem 1rem;
            text-align: left;
            font-size: 0.85rem;
            font-weight: 600;
            color: #6c757d;
            text-transform: uppercase;
            letter-spacing: 0.05em;
            border-bottom: 2px solid #dee2e6;
        }
        tbody tr { border-bottom: 1px solid #f0f0f0; }
        tbody tr:hover { background: #fafafa; }
        tbody td { padding: 0.75rem 1rem; font-size: 0.9rem; }
        .rank-badge {
            display: inline-flex;
            align-items: center;
            justify-content: center;
            width: 28px; height: 28px;
            border-radius: 50%;
            font-weight: 700;
            font-size: 0.85rem;
        }
        .rank-1 { background: #ffd700; color: #7a5800; }
        .rank-2 { background: #c0c0c0; color: #444; }
        .rank-3 { background: #cd7f32; color: white; }
        .rank-other { background: #e9ecef; color: #6c757d; }
        .score-bar-wrap {
            display: flex;
            align-items: center;
            gap: 0.5rem;
        }
        .score-bar {
            flex: 1;
            height: 8px;
            background: #e9ecef;
            border-radius: 4px;
            overflow: hidden;
            max-width: 120px;
        }
        .score-bar-fill {
            height: 100%;
            border-radius: 4px;
            transition: width 0.6s ease;
        }
        .score-bar-fill.high { background: #28a745; }
        .score-bar-fill.mid  { background: #ffc107; }
        .score-bar-fill.low  { background: #dc3545; }
        .score-val { font-weight: 600; font-size: 0.9rem; min-width: 3rem; }
        .health-grid {
            display: grid;
            grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
            gap: 1rem;
        }
        .health-card {
            border: 1px solid #dee2e6;
            border-radius: 8px;
            padding: 1rem;
            position: relative;
            overflow: hidden;
        }
        .health-card::before {
            content: '';
            position: absolute;
            left: 0; top: 0; bottom: 0;
            width: 4px;
        }
        .health-card.healthy::before  { background: #28a745; }
        .health-card.warning::before  { background: #ffc107; }
        .health-card.critical::before { background: #dc3545; }
        .health-card.unknown::before  { background: #6c757d; }
        .health-channel-name {
            font-weight: 600;
            font-size: 0.95rem;
            margin-bottom: 0.25rem;
        }
        .health-channel-meta {
            font-size: 0.8rem;
            color: #6c757d;
            margin-bottom: 0.75rem;
        }
        .health-score-row {
            display: flex;
            align-items: center;
            justify-content: space-between;
            margin-bottom: 0.5rem;
        }
        .health-score-number {
            font-size: 1.75rem;
            font-weight: 700;
            line-height: 1;
        }
        .health-score-number.healthy  { color: #28a745; }
        .health-score-number.warning  { color: #ffc107; }
        .health-score-number.critical { color: #dc3545; }
        .health-score-number.unknown  { color: #6c757d; }
        .health-stats {
            display: grid;
            grid-template-columns: 1fr 1fr;
            gap: 0.25rem 1rem;
            font-size: 0.8rem;
            color: #6c757d;
            margin-top: 0.5rem;
        }
        .health-stats span { display: flex; gap: 0.3rem; }
        .health-stats span strong { color: #343a40; }
        .rec-card {
            border-left: 4px solid #007bff;
            padding: 1rem 1.25rem;
            background: #f0f8ff;
            border-radius: 0 8px 8px 0;
            margin-bottom: 0.75rem;
        }
        .rec-card.warning { border-left-color: #ffc107; background: #fffdf0; }
        .rec-card.danger  { border-left-color: #dc3545; background: #fff5f5; }
        .rec-type {
            font-size: 0.75rem;
            font-weight: 700;
            text-transform: uppercase;
            letter-spacing: 0.08em;
            color: #6c757d;
            margin-bottom: 0.35rem;
        }
        .rec-text {
            font-size: 0.95rem;
            color: #343a40;
            line-height: 1.5;
        }
        .rec-confidence {
            margin-top: 0.35rem;
            font-size: 0.8rem;
            color: #6c757d;
        }
        .btn {
            padding: 0.5rem 1rem;
            border: none;
            border-radius: 4px;
            cursor: pointer;
            font-size: 0.9rem;
            transition: all 0.2s;
        }
        .btn-secondary { background: #6c757d; color: white; }
        .btn-secondary:hover { background: #545b62; }
        .loading { text-align: center; padding: 3rem; color: #6c757d; }
        .empty { text-align: center; padding: 2rem; color: #adb5bd; font-style: italic; }
        .summary-row {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
            gap: 1rem;
            margin-bottom: 1.5rem;
        }
        .summary-box {
            background: white;
            border-radius: 8px;
            box-shadow: 0 2px 4px rgba(0,0,0,0.1);
            padding: 1.25rem;
            text-align: center;
        }
        .summary-box .number {
            font-size: 2rem;
            font-weight: 700;
            color: #dc3545;
        }
        .summary-box .label {
            font-size: 0.8rem;
            color: #6c757d;
            text-transform: uppercase;
            letter-spacing: 0.05em;
            margin-top: 0.25rem;
        }
        .last-updated {
            font-size: 0.75rem;
            color: #adb5bd;
            text-align: right;
            margin-top: 0.5rem;
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
            <li><a href="/admin/clipping-jobs">Clipping Jobs</a></li>
            <li><a href="/admin/clipping-activity">Activity</a></li>
            <li><a href="/admin/performance" class="active">Performance</a></li>
            <li><a href="/admin/test-runs">🧪 Portfolio Tests</a></li>
            <li><a href="#" onclick="logout()">Logout</a></li>
        </ul>
    </div>

    <!-- Main Content -->
    <div class="main-content">
        <div class="page-header">
            <div>
                <h1>Performance Dashboard</h1>
                <p>Viral factor analytics, channel health scores &amp; AI learning insights</p>
            </div>
            <button class="btn btn-secondary" onclick="refreshAll()">Refresh</button>
        </div>

        <!-- Summary Row -->
        <div class="summary-row">
            <div class="summary-box"><div class="number" id="sumFactors">—</div><div class="label">Tracked Factors</div></div>
            <div class="summary-box"><div class="number" id="sumChannels">—</div><div class="label">Monitored Channels</div></div>
            <div class="summary-box"><div class="number" id="sumHealthy">—</div><div class="label">Healthy Channels</div></div>
            <div class="summary-box"><div class="number" id="sumRecs">—</div><div class="label">Active Recommendations</div></div>
        </div>

        <!-- Tabs -->
        <div class="tabs">
            <button class="tab-btn active" onclick="switchTab('viral', this)">Viral Factors</button>
            <button class="tab-btn" onclick="switchTab('health', this)">Channel Health</button>
            <button class="tab-btn" onclick="switchTab('recs', this)">Recommendations</button>
        </div>

        <!-- Tab: Viral Factors -->
        <div id="tab-viral" class="tab-content active">
            <div class="card">
                <div class="card-title">Top Viral Factors — ranked by performance score</div>
                <div id="viralTable"><div class="loading">Loading...</div></div>
            </div>
        </div>

        <!-- Tab: Channel Health -->
        <div id="tab-health" class="tab-content">
            <div class="card">
                <div class="card-title">Source Channel Health Scores — 30-day rolling window</div>
                <div id="healthGrid"><div class="loading">Loading...</div></div>
            </div>
        </div>

        <!-- Tab: Recommendations -->
        <div id="tab-recs" class="tab-content">
            <div class="card">
                <div class="card-title">Active Learning Recommendations</div>
                <div id="recsList"><div class="loading">Loading...</div></div>
            </div>

            <div class="card" style="margin-top:1.5rem;">
                <div class="card-title">🎨 AI Thumbnail Pipeline Status</div>
                <div id="thumbStats"><div class="loading">Loading...</div></div>
            </div>
        </div>
    </div>

    <script>
        const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
        const user = JSON.parse(localStorage.getItem('user') || '{}');
        if (!authToken || (!user.is_staff && !user.is_superuser)) {
            window.location.href = '/admin/login';
        }

        function switchTab(name, btn) {
            document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
            document.querySelectorAll('.tab-content').forEach(t => t.classList.remove('active'));
            btn.classList.add('active');
            document.getElementById('tab-' + name).classList.add('active');
        }

        async function loadViralFactors() {
            try {
                const res = await fetch('/api/admin/performance/viral-factors', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await res.json();
                if (!data.success) throw new Error(data.error || 'Failed');

                const factors = data.viral_factors || [];
                document.getElementById('sumFactors').textContent = factors.length;

                if (factors.length === 0) {
                    document.getElementById('viralTable').innerHTML =
                        '<div class="empty">No viral factor data yet. Run the analytics sync job to populate this table.</div>';
                    return;
                }

                const maxScore = Math.max(...factors.map(f => parseFloat(f.performance_score) || 0));

                const rows = factors.map(f => {
                    const score = parseFloat(f.performance_score) || 0;
                    const pct = maxScore > 0 ? (score / maxScore * 100).toFixed(1) : 0;
                    const fillClass = score >= 0.7 ? 'high' : score >= 0.4 ? 'mid' : 'low';
                    const rank = parseInt(f.rank) || 99;
                    const rankHtml = rank === 1 ? '<span class="rank-badge rank-1">1</span>'
                                   : rank === 2 ? '<span class="rank-badge rank-2">2</span>'
                                   : rank === 3 ? '<span class="rank-badge rank-3">3</span>'
                                   : '<span class="rank-badge rank-other">' + rank + '</span>';
                    return '<tr><td>' + rankHtml + '</td>'
                        + '<td style="font-weight:500;">' + escHtml(f.viral_factor) + '</td>'
                        + '<td>' + (f.times_used || 0) + '</td>'
                        + '<td>' + (f.total_clips || 0) + '</td>'
                        + '<td>' + formatNum(f.avg_views) + '</td>'
                        + '<td>' + formatPct(f.avg_like_rate) + '</td>'
                        + '<td>' + formatPct(f.avg_watch_percentage) + '</td>'
                        + '<td><div class="score-bar-wrap"><div class="score-bar"><div class="score-bar-fill ' + fillClass + '" style="width:' + pct + '%"></div></div>'
                        + '<span class="score-val">' + score.toFixed(3) + '</span></div></td>'
                        + '<td style="font-size:0.75rem;color:#6c757d;">' + formatDate(f.last_calculated_at) + '</td></tr>';
                }).join('');

                document.getElementById('viralTable').innerHTML =
                    '<table><thead><tr>'
                    + '<th>Rank</th><th>Viral Factor</th><th>Used</th><th>Clips</th>'
                    + '<th>Avg Views</th><th>Like Rate</th><th>Watch %</th><th>Score</th><th>Updated</th>'
                    + '</tr></thead><tbody>' + rows + '</tbody></table>'
                    + '<div class="last-updated">Data from analytics sync job (runs every 6 hours)</div>';
            } catch (e) {
                document.getElementById('viralTable').innerHTML =
                    '<div class="loading">Error: ' + escHtml(e.message) + '</div>';
            }
        }

        async function loadChannelHealth() {
            try {
                const res = await fetch('/api/admin/performance/channel-health', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await res.json();
                if (!data.success) throw new Error(data.error || 'Failed');

                const channels = data.channels || [];
                const healthyCount = channels.filter(c => {
                    const s = parseFloat(c.health_score);
                    return !isNaN(s) && s >= 0.7;
                }).length;
                document.getElementById('sumChannels').textContent = channels.length;
                document.getElementById('sumHealthy').textContent = healthyCount;

                if (channels.length === 0) {
                    document.getElementById('healthGrid').innerHTML =
                        '<div class="empty">No source channels found.</div>';
                    return;
                }

                const cards = channels.map(c => {
                    const score = c.health_score != null ? parseFloat(c.health_score) : null;
                    let tier = 'unknown', tierLabel = 'No Data';
                    if (score !== null) {
                        if (score >= 0.75)      { tier = 'healthy';  tierLabel = 'Healthy'; }
                        else if (score >= 0.45) { tier = 'warning';  tierLabel = 'At Risk'; }
                        else                    { tier = 'critical'; tierLabel = 'Critical'; }
                    }
                    const scoreDisplay = score !== null ? (score * 100).toFixed(0) + '%' : '—';
                    const tierBg = tier==='healthy' ? '#d4edda' : tier==='warning' ? '#fff3cd' : tier==='critical' ? '#f8d7da' : '#e9ecef';
                    const tierColor = tier==='healthy' ? '#155724' : tier==='warning' ? '#856404' : tier==='critical' ? '#721c24' : '#6c757d';
                    const lastErrorHtml = c.last_error
                        ? '<span title="' + escHtml(c.last_error) + '" style="cursor:help;text-decoration:underline dotted;">See last error</span>'
                        : 'None';
                    return '<div class="health-card ' + tier + '">'
                        + '<div class="health-channel-name">' + escHtml(c.channel_name || 'Unknown') + '</div>'
                        + '<div class="health-channel-meta">' + escHtml(c.channel_id || '') + '</div>'
                        + '<div class="health-score-row">'
                        +   '<div><div style="font-size:0.7rem;color:#6c757d;text-transform:uppercase;letter-spacing:0.06em;">Health Score</div>'
                        +   '<div class="health-score-number ' + tier + '">' + scoreDisplay + '</div></div>'
                        +   '<span style="padding:0.2rem 0.6rem;border-radius:12px;font-size:0.75rem;font-weight:600;background:' + tierBg + ';color:' + tierColor + ';">' + tierLabel + '</span>'
                        + '</div>'
                        + '<div class="health-stats">'
                        +   '<span><strong>' + (c.jobs_succeeded || 0) + '</strong> succeeded</span>'
                        +   '<span><strong>' + (c.jobs_attempted || 0) + '</strong> attempted</span>'
                        +   '<span>Last error: ' + lastErrorHtml + '</span>'
                        +   '<span>Updated: ' + formatDateShort(c.last_health_check) + '</span>'
                        + '</div></div>';
                }).join('');

                document.getElementById('healthGrid').innerHTML = '<div class="health-grid">' + cards + '</div>';
            } catch (e) {
                document.getElementById('healthGrid').innerHTML =
                    '<div class="loading">Error: ' + escHtml(e.message) + '</div>';
            }
        }

        async function loadRecommendations() {
            try {
                const res = await fetch('/api/admin/performance/recommendations', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await res.json();
                if (!data.success) throw new Error(data.error || 'Failed');

                const recs = data.recommendations || [];
                document.getElementById('sumRecs').textContent = recs.length;

                if (recs.length === 0) {
                    document.getElementById('recsList').innerHTML =
                        '<div class="empty">No active recommendations. The system will generate insights as more jobs complete.</div>';
                    return;
                }

                const cards = recs.map(r => {
                    const conf = parseFloat(r.confidence) || 0;
                    const tier = conf >= 0.8 ? '' : conf >= 0.5 ? 'warning' : 'danger';
                    return '<div class="rec-card ' + tier + '">'
                        + '<div class="rec-type">' + escHtml(r.type || 'General') + '</div>'
                        + '<div class="rec-text">' + escHtml(r.recommendation) + '</div>'
                        + '<div class="rec-confidence">Confidence: ' + (conf * 100).toFixed(0) + '% · Updated: ' + formatDate(r.updated_at) + '</div>'
                        + '</div>';
                }).join('');

                document.getElementById('recsList').innerHTML = cards;
            } catch (e) {
                document.getElementById('recsList').innerHTML =
                    '<div class="loading">Error: ' + escHtml(e.message) + '</div>';
            }
        }

        function escHtml(s) {
            if (!s) return '';
            return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;')
                            .replace(/"/g,'&quot;').replace(/'/g,'&#39;');
        }

        function formatNum(n) {
            if (n == null || n === '') return '—';
            const v = parseFloat(n);
            if (isNaN(v)) return '—';
            if (v >= 1000000) return (v / 1000000).toFixed(1) + 'M';
            if (v >= 1000) return (v / 1000).toFixed(1) + 'K';
            return v.toFixed(0);
        }

        function formatPct(n) {
            if (n == null || n === '') return '—';
            const v = parseFloat(n);
            if (isNaN(v)) return '—';
            return (v * 100).toFixed(1) + '%';
        }

        function formatDate(d) {
            if (!d) return 'Never';
            return new Date(d).toLocaleString();
        }

        function formatDateShort(d) {
            if (!d) return 'Never';
            return new Date(d).toLocaleDateString();
        }

        function logout() {
            localStorage.removeItem('authToken');
            localStorage.removeItem('user');
            window.location.href = '/admin/login';
        }

        async function loadThumbnailStats() {
            try {
                const res = await fetch('/api/admin/performance/thumbnails', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await res.json();
                if (!data.success) throw new Error(data.error || 'Failed');

                const stats = data.stats || [];
                if (stats.length === 0) {
                    document.getElementById('thumbStats').innerHTML =
                        '<div class="empty">No clips extracted yet.</div>';
                    return;
                }

                const aiRate = data.ai_success_rate_pct || 0;
                const total = data.total_clips || 0;
                const aiCount = data.ai_generated_count || 0;

                let html = '<div style="display:flex;gap:1.5rem;margin-bottom:1rem;flex-wrap:wrap;">'
                    + '<div style="background:#e8f4fd;padding:0.75rem 1.25rem;border-radius:6px;">'
                    + '<div style="font-size:1.5rem;font-weight:700;">' + total + '</div>'
                    + '<div style="color:#6c757d;font-size:0.8rem;">Total Clips</div></div>'
                    + '<div style="background:#d4edda;padding:0.75rem 1.25rem;border-radius:6px;">'
                    + '<div style="font-size:1.5rem;font-weight:700;">' + aiCount + '</div>'
                    + '<div style="color:#6c757d;font-size:0.8rem;">AI-Generated Thumbnails</div></div>'
                    + '<div style="background:#fff3cd;padding:0.75rem 1.25rem;border-radius:6px;">'
                    + '<div style="font-size:1.5rem;font-weight:700;">' + aiRate + '%</div>'
                    + '<div style="color:#6c757d;font-size:0.8rem;">AI Success Rate</div></div>'
                    + '</div>';

                html += '<table style="width:100%;border-collapse:collapse;font-size:0.875rem;">'
                    + '<thead><tr style="background:#f8f9fa;">'
                    + '<th style="text-align:left;padding:0.5rem;">Generation Method</th>'
                    + '<th style="text-align:right;padding:0.5rem;">Clips</th>'
                    + '<th style="text-align:right;padding:0.5rem;">Share</th>'
                    + '</tr></thead><tbody>';

                stats.forEach(function(s) {
                    const pct = total > 0 ? ((s.count / total) * 100).toFixed(1) : '0.0';
                    const methodLabel = s.method === 'ffmpeg_timestamp' ? '📸 FFmpeg frame (fallback)'
                        : s.method === 'ai_gemini_overlay' ? '🤖 AI Gemini overlay'
                        : s.method === 'none' ? '— None'
                        : escHtml(s.method);
                    html += '<tr style="border-top:1px solid #dee2e6;">'
                        + '<td style="padding:0.5rem;">' + methodLabel + '</td>'
                        + '<td style="text-align:right;padding:0.5rem;">' + s.count + '</td>'
                        + '<td style="text-align:right;padding:0.5rem;">' + pct + '%</td>'
                        + '</tr>';
                });

                html += '</tbody></table>';
                document.getElementById('thumbStats').innerHTML = html;
            } catch (e) {
                document.getElementById('thumbStats').innerHTML =
                    '<div class="loading">Error: ' + escHtml(e.message) + '</div>';
            }
        }

        function refreshAll() {
            loadViralFactors();
            loadChannelHealth();
            loadRecommendations();
            loadThumbnailStats();
        }

        refreshAll();
        setInterval(refreshAll, 300000);
    </script>
</body>
</html>
    "###;

    Html(html.to_string())
}

// ═══════════════════════════════════════════════════════════════════════════════
// Portfolio Test Runs — API handlers
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(serde::Deserialize)]
pub struct TriggerTestRunRequest {
    pub name: Option<String>,
}

pub async fn api_trigger_test_run(
    Extension(state): Extension<Arc<AppState>>,
    axum::Json(body): axum::Json<TriggerTestRunRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let name = body.name.unwrap_or_else(|| {
        format!(
            "Portfolio run {}",
            chrono::Utc::now().format("%Y-%m-%d %H:%M")
        )
    });

    match crate::portfolio_tests::PortfolioTestRunner::create_and_spawn(state, name).await {
        Ok(run_id) => Ok(Json(json!({ "run_id": run_id, "status": "running" }))),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )),
    }
}

pub async fn api_trigger_manual_clipping_test(
    Extension(state): Extension<Arc<AppState>>,
    axum::Json(body): axum::Json<TriggerTestRunRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let name = body.name.unwrap_or_else(|| {
        format!(
            "Manual Clipping test {}",
            chrono::Utc::now().format("%Y-%m-%d %H:%M")
        )
    });

    match crate::manual_clipping_tests::ManualClippingTestRunner::create_and_spawn(state, name)
        .await
    {
        Ok(run_id) => Ok(Json(json!({ "run_id": run_id, "status": "running" }))),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )),
    }
}

pub async fn api_list_test_runs(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let rows = sqlx::query(
        "SELECT id, name, status, started_at, completed_at, \
                total_tests, passed_tests, failed_tests \
         FROM test_runs ORDER BY started_at DESC LIMIT 50",
    )
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

    let runs: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            let id: Uuid = r.get("id");
            let started_at: chrono::DateTime<chrono::Utc> = r.get("started_at");
            let completed_at: Option<chrono::DateTime<chrono::Utc>> = r.get("completed_at");
            json!({
                "id": id,
                "name": r.get::<String, _>("name"),
                "status": r.get::<String, _>("status"),
                "started_at": started_at,
                "completed_at": completed_at,
                "total_tests": r.get::<i32, _>("total_tests"),
                "passed_tests": r.get::<i32, _>("passed_tests"),
                "failed_tests": r.get::<i32, _>("failed_tests"),
            })
        })
        .collect();

    Ok(Json(json!({ "runs": runs })))
}

pub async fn api_get_test_run(
    Path(id): Path<uuid::Uuid>,
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let run = sqlx::query(
        "SELECT id, name, status, started_at, completed_at, \
                total_tests, passed_tests, failed_tests \
         FROM test_runs WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?
    .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({ "error": "Test run not found" }))))?;

    let results = sqlx::query(
        "SELECT id, test_name, gig_type, prompt, status, \
                output_r2_key, output_r2_url, output_filename, \
                error_message, llm_review_score, llm_review_feedback, \
                llm_reviewer, started_at, completed_at \
         FROM test_results WHERE run_id = $1 ORDER BY started_at ASC",
    )
    .bind(id)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))))?;

    let results_json: Vec<serde_json::Value> = results
        .into_iter()
        .map(|r| {
            let rid: Uuid = r.get("id");
            let started_at: chrono::DateTime<chrono::Utc> = r.get("started_at");
            let completed_at: Option<chrono::DateTime<chrono::Utc>> = r.get("completed_at");
            json!({
                "id": rid,
                "test_name": r.get::<String, _>("test_name"),
                "gig_type": r.get::<String, _>("gig_type"),
                "prompt": r.get::<String, _>("prompt"),
                "status": r.get::<String, _>("status"),
                "output_r2_key": r.get::<Option<String>, _>("output_r2_key"),
                "output_r2_url": r.get::<Option<String>, _>("output_r2_url"),
                "output_filename": r.get::<Option<String>, _>("output_filename"),
                "error_message": r.get::<Option<String>, _>("error_message"),
                "llm_review_score": r.get::<Option<i32>, _>("llm_review_score"),
                "llm_review_feedback": r.get::<Option<String>, _>("llm_review_feedback"),
                "llm_reviewer": r.get::<Option<String>, _>("llm_reviewer"),
                "started_at": started_at,
                "completed_at": completed_at,
            })
        })
        .collect();

    let run_id: Uuid = run.get("id");
    let run_started: chrono::DateTime<chrono::Utc> = run.get("started_at");
    let run_completed: Option<chrono::DateTime<chrono::Utc>> = run.get("completed_at");

    Ok(Json(json!({
        "run": {
            "id": run_id,
            "name": run.get::<String, _>("name"),
            "status": run.get::<String, _>("status"),
            "started_at": run_started,
            "completed_at": run_completed,
            "total_tests": run.get::<i32, _>("total_tests"),
            "passed_tests": run.get::<i32, _>("passed_tests"),
            "failed_tests": run.get::<i32, _>("failed_tests"),
        },
        "results": results_json,
    })))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Portfolio Test Runs — SSR Pages
// ═══════════════════════════════════════════════════════════════════════════════

pub async fn admin_test_runs_page() -> Html<String> {
    let html = r###"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Portfolio Test Runs — VideoSync Admin</title>
<style>
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
         background: #0f0f13; color: #e0e0e0; display: flex; min-height: 100vh; }
  .sidebar { width: 220px; background: #1a1a24; padding: 24px 0; flex-shrink: 0; }
  .sidebar h2 { color: #fff; font-size: 14px; font-weight: 700; padding: 0 20px 20px;
                border-bottom: 1px solid #2a2a36; letter-spacing: 0.05em; }
  .sidebar a { display: block; padding: 10px 20px; color: #9999bb; text-decoration: none;
               font-size: 13px; transition: all 0.15s; }
  .sidebar a:hover, .sidebar a.active { background: #23233a; color: #fff; }
  .main { flex: 1; padding: 32px; overflow-y: auto; }
  .header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 28px; }
  .header h1 { font-size: 22px; font-weight: 600; color: #fff; }
  .btn { padding: 10px 20px; border: none; border-radius: 8px; cursor: pointer;
         font-size: 13px; font-weight: 600; transition: all 0.2s; }
  .btn-primary { background: #6c5ce7; color: #fff; }
  .btn-primary:hover { background: #5a4bd1; }
  .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
  .runs-grid { display: flex; flex-direction: column; gap: 12px; }
  .run-card { background: #1a1a24; border-radius: 10px; padding: 20px 24px;
              border: 1px solid #2a2a36; cursor: pointer; transition: border-color 0.15s;
              text-decoration: none; color: inherit; display: block; }
  .run-card:hover { border-color: #6c5ce7; }
  .run-card-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 10px; }
  .run-name { font-size: 15px; font-weight: 600; color: #fff; }
  .badge { padding: 3px 10px; border-radius: 20px; font-size: 11px; font-weight: 600;
           letter-spacing: 0.04em; }
  .badge-running { background: #2563eb22; color: #60a5fa; }
  .badge-completed { background: #16a34a22; color: #4ade80; }
  .badge-completed_with_failures { background: #d9770622; color: #fb923c; }
  .badge-failed { background: #dc262622; color: #f87171; }
  .run-meta { font-size: 12px; color: #666680; display: flex; gap: 20px; flex-wrap: wrap; }
  .run-scores { display: flex; gap: 16px; margin-top: 12px; }
  .score-item { text-align: center; }
  .score-num { font-size: 20px; font-weight: 700; color: #fff; }
  .score-label { font-size: 11px; color: #666680; }
  .score-passed .score-num { color: #4ade80; }
  .score-failed .score-num { color: #f87171; }
  .empty { text-align: center; padding: 60px; color: #666680; }
  .modal-overlay { display: none; position: fixed; inset: 0; background: rgba(0,0,0,0.7);
                   z-index: 1000; align-items: center; justify-content: center; }
  .modal-overlay.show { display: flex; }
  .modal { background: #1a1a24; border-radius: 12px; padding: 28px; width: 440px;
           border: 1px solid #2a2a36; }
  .modal h3 { font-size: 17px; font-weight: 600; color: #fff; margin-bottom: 16px; }
  .modal label { display: block; font-size: 12px; color: #9999bb; margin-bottom: 6px; }
  .modal input { width: 100%; background: #0f0f13; border: 1px solid #2a2a36; border-radius: 7px;
                 padding: 10px 14px; color: #fff; font-size: 14px; margin-bottom: 20px; }
  .modal-actions { display: flex; gap: 12px; justify-content: flex-end; }
  .btn-ghost { background: transparent; border: 1px solid #2a2a36; color: #9999bb; }
  .btn-ghost:hover { border-color: #6c5ce7; color: #fff; }
  .spinner { display: inline-block; width: 14px; height: 14px; border: 2px solid rgba(255,255,255,0.3);
             border-top-color: #fff; border-radius: 50%; animation: spin 0.7s linear infinite;
             margin-right: 8px; vertical-align: middle; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .info-banner { background: #2563eb15; border: 1px solid #2563eb40; border-radius: 8px;
                 padding: 14px 18px; margin-bottom: 24px; font-size: 13px; color: #93c5fd; }
</style>
</head>
<body>
<div class="sidebar">
  <h2>VIDEOSYNC ADMIN</h2>
  <a href="/admin/dashboard">Dashboard</a>
  <a href="/admin/users">Users</a>
  <a href="/admin/clipping-jobs">Clipping Jobs</a>
  <a href="/admin/clipping-activity">Activity</a>
  <a href="/admin/performance">Performance</a>
  <a href="/admin/test-runs" class="active">Portfolio Tests</a>
  <a href="/admin/deliveries">Deliveries</a>
</div>
<div class="main">
  <div class="header">
    <h1>Portfolio Test Runs</h1>
    <button class="btn btn-primary" onclick="openModal()">+ New Test Run</button>
  </div>
  <div class="info-banner">
    Each run executes 12 Fiverr gig scenarios (7 Blender tools × 2 variants + thumbnails),
    uploads outputs to R2, and gets a Gemini quality review. Each run takes ~30–40 minutes.
  </div>
  <div id="runs-container" class="runs-grid">
    <div class="empty">Loading...</div>
  </div>
</div>

<!-- New Run Modal -->
<div class="modal-overlay" id="modal">
  <div class="modal">
    <h3>Start New Test Run</h3>
    <label>Run Name</label>
    <input type="text" id="run-name" placeholder="e.g. Portfolio v1 — March 2026">
    <div class="modal-actions">
      <button class="btn btn-ghost" onclick="closeModal()">Cancel</button>
      <button class="btn btn-primary" id="start-btn" onclick="startRun()">Start Run</button>
    </div>
  </div>
</div>

<script>
const token = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
function esc(s) { return String(s||'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;'); }

function statusBadge(s) {
  return `<span class="badge badge-${s}">${s.replace(/_/g,' ')}</span>`;
}

function scoreColor(n) {
  if (n >= 8) return '#4ade80';
  if (n >= 5) return '#fbbf24';
  return '#f87171';
}

async function loadRuns() {
  try {
    const r = await fetch('/api/admin/test-runs', {
      headers: { 'Authorization': 'Bearer ' + token }
    });
    const data = await r.json();
    const c = document.getElementById('runs-container');
    if (!data.runs || data.runs.length === 0) {
      c.innerHTML = '<div class="empty">No test runs yet. Click "New Test Run" to start.</div>';
      return;
    }
    c.innerHTML = data.runs.map(run => {
      const started = new Date(run.started_at).toLocaleString();
      const duration = run.completed_at
        ? Math.round((new Date(run.completed_at) - new Date(run.started_at)) / 60000) + ' min'
        : 'in progress...';
      return `<a class="run-card" href="/admin/test-runs/${run.id}">
        <div class="run-card-header">
          <span class="run-name">${esc(run.name)}</span>
          ${statusBadge(run.status)}
        </div>
        <div class="run-meta">
          <span>Started: ${esc(started)}</span>
          <span>Duration: ${esc(duration)}</span>
        </div>
        <div class="run-scores">
          <div class="score-item score-passed">
            <div class="score-num">${run.passed_tests}</div>
            <div class="score-label">Passed</div>
          </div>
          <div class="score-item score-failed">
            <div class="score-num">${run.failed_tests}</div>
            <div class="score-label">Failed</div>
          </div>
          <div class="score-item">
            <div class="score-num">${run.total_tests}</div>
            <div class="score-label">Total</div>
          </div>
        </div>
      </a>`;
    }).join('');
  } catch(e) {
    document.getElementById('runs-container').innerHTML =
      '<div class="empty">Error loading runs: ' + esc(e.message) + '</div>';
  }
}

function openModal() {
  document.getElementById('run-name').value =
    'Portfolio Run — ' + new Date().toLocaleDateString('en-GB', {day:'2-digit',month:'short',year:'numeric'});
  document.getElementById('modal').classList.add('show');
}
function closeModal() { document.getElementById('modal').classList.remove('show'); }

async function startRun() {
  const name = document.getElementById('run-name').value.trim() || 'Portfolio Run';
  const btn = document.getElementById('start-btn');
  btn.disabled = true;
  btn.innerHTML = '<span class="spinner"></span>Starting...';
  try {
    const r = await fetch('/api/admin/test-runs', {
      method: 'POST',
      headers: { 'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json' },
      body: JSON.stringify({ name })
    });
    const data = await r.json();
    if (data.run_id) {
      closeModal();
      window.location.href = '/admin/test-runs/' + data.run_id;
    } else {
      alert('Error: ' + (data.error || 'Unknown error'));
    }
  } catch(e) {
    alert('Error: ' + e.message);
  } finally {
    btn.disabled = false;
    btn.innerHTML = 'Start Run';
  }
}

loadRuns();
setInterval(loadRuns, 15000);
</script>
</body>
</html>"###;
    Html(html.to_string())
}

pub async fn admin_test_run_detail_page(Path(id): Path<String>) -> Html<String> {
    let html = format!(r###"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Test Run Detail — VideoSync Admin</title>
<style>
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
          background: #0f0f13; color: #e0e0e0; display: flex; min-height: 100vh; }}
  .sidebar {{ width: 220px; background: #1a1a24; padding: 24px 0; flex-shrink: 0; }}
  .sidebar h2 {{ color: #fff; font-size: 14px; font-weight: 700; padding: 0 20px 20px;
                 border-bottom: 1px solid #2a2a36; letter-spacing: 0.05em; }}
  .sidebar a {{ display: block; padding: 10px 20px; color: #9999bb; text-decoration: none;
                font-size: 13px; transition: all 0.15s; }}
  .sidebar a:hover, .sidebar a.active {{ background: #23233a; color: #fff; }}
  .main {{ flex: 1; padding: 32px; overflow-y: auto; max-width: calc(100vw - 220px); }}
  .breadcrumb {{ font-size: 12px; color: #666680; margin-bottom: 20px; }}
  .breadcrumb a {{ color: #9999bb; text-decoration: none; }}
  .run-header {{ display: flex; justify-content: space-between; align-items: flex-start;
                 margin-bottom: 24px; }}
  .run-header h1 {{ font-size: 20px; font-weight: 600; color: #fff; }}
  .badge {{ padding: 4px 12px; border-radius: 20px; font-size: 12px; font-weight: 600; }}
  .badge-running {{ background: #2563eb22; color: #60a5fa; }}
  .badge-completed {{ background: #16a34a22; color: #4ade80; }}
  .badge-completed_with_failures {{ background: #d9770622; color: #fb923c; }}
  .badge-failed {{ background: #dc262622; color: #f87171; }}
  .stats-row {{ display: flex; gap: 16px; margin-bottom: 28px; flex-wrap: wrap; }}
  .stat-box {{ background: #1a1a24; border: 1px solid #2a2a36; border-radius: 10px;
               padding: 16px 22px; min-width: 120px; }}
  .stat-val {{ font-size: 24px; font-weight: 700; color: #fff; }}
  .stat-label {{ font-size: 11px; color: #666680; margin-top: 4px; }}
  .stat-passed .stat-val {{ color: #4ade80; }}
  .stat-failed .stat-val {{ color: #f87171; }}
  .results-grid {{ display: flex; flex-direction: column; gap: 14px; }}
  .result-card {{ background: #1a1a24; border: 1px solid #2a2a36; border-radius: 10px;
                  padding: 20px 24px; }}
  .result-card.passed {{ border-left: 3px solid #4ade80; }}
  .result-card.failed {{ border-left: 3px solid #f87171; }}
  .result-card.running {{ border-left: 3px solid #60a5fa; }}
  .result-header {{ display: flex; justify-content: space-between; align-items: center;
                    margin-bottom: 10px; }}
  .result-name {{ font-size: 14px; font-weight: 600; color: #fff; }}
  .gig-tag {{ background: #23233a; border-radius: 5px; padding: 2px 8px; font-size: 11px;
              color: #9999bb; }}
  .result-prompt {{ font-size: 12px; color: #666680; margin-bottom: 12px; line-height: 1.5; }}
  .result-body {{ display: flex; gap: 20px; align-items: flex-start; flex-wrap: wrap; }}
  .media-preview {{ flex-shrink: 0; }}
  .media-preview video, .media-preview img {{
    max-width: 280px; max-height: 180px; border-radius: 8px;
    background: #0a0a10; border: 1px solid #2a2a36; }}
  .result-info {{ flex: 1; min-width: 200px; }}
  .review-box {{ background: #0f0f1a; border-radius: 8px; padding: 14px; margin-top: 8px; }}
  .review-score {{ display: flex; align-items: center; gap: 10px; margin-bottom: 8px; }}
  .score-circle {{ width: 40px; height: 40px; border-radius: 50%; display: flex;
                   align-items: center; justify-content: center; font-size: 16px;
                   font-weight: 700; border: 2px solid; }}
  .review-label {{ font-size: 11px; color: #666680; }}
  .review-feedback {{ font-size: 13px; color: #b0b0c8; line-height: 1.5; }}
  .download-btn {{ display: inline-block; margin-top: 12px; padding: 8px 16px;
                   background: #6c5ce7; color: #fff; border-radius: 7px;
                   text-decoration: none; font-size: 12px; font-weight: 600; }}
  .download-btn:hover {{ background: #5a4bd1; }}
  .error-msg {{ background: #dc262610; border: 1px solid #dc262630; border-radius: 8px;
                padding: 12px; font-size: 12px; color: #f87171; font-family: monospace;
                line-height: 1.5; word-break: break-all; }}
  .spinner {{ display: inline-block; width: 14px; height: 14px; border: 2px solid rgba(255,255,255,0.3);
              border-top-color: #fff; border-radius: 50%; animation: spin 0.7s linear infinite; }}
  @keyframes spin {{ to {{ transform: rotate(360deg); }} }}
  .refresh-note {{ font-size: 12px; color: #666680; text-align: right; margin-bottom: 12px; }}
</style>
</head>
<body>
<div class="sidebar">
  <h2>VIDEOSYNC ADMIN</h2>
  <a href="/admin/dashboard">Dashboard</a>
  <a href="/admin/users">Users</a>
  <a href="/admin/clipping-jobs">Clipping Jobs</a>
  <a href="/admin/clipping-activity">Activity</a>
  <a href="/admin/performance">Performance</a>
  <a href="/admin/test-runs" class="active">Portfolio Tests</a>
</div>
<div class="main">
  <div class="breadcrumb">
    <a href="/admin/test-runs">Portfolio Tests</a> &rsaquo; Run Detail
  </div>
  <div id="run-header" class="run-header">
    <h1>Loading...</h1>
  </div>
  <div id="stats-row" class="stats-row"></div>
  <div id="refresh-note" class="refresh-note" style="display:none">
    <span class="spinner"></span> Run in progress — auto-refreshing every 20s
  </div>
  <div id="results-container" class="results-grid">
    <div style="color:#666680;text-align:center;padding:40px">Loading results...</div>
  </div>
</div>

<script>
const RUN_ID = "{id}";
const token = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
let autoRefresh = null;

function esc(s) {{ return String(s||'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;'); }}

function scoreStyle(n) {{
  if (!n) return {{ color: '#666680', border: '#333350' }};
  if (n >= 8) return {{ color: '#4ade80', border: '#4ade80' }};
  if (n >= 5) return {{ color: '#fbbf24', border: '#fbbf24' }};
  return {{ color: '#f87171', border: '#f87171' }};
}}

function renderMedia(r) {{
  if (!r.output_r2_url) return '';
  const ext = (r.output_filename || '').split('.').pop().toLowerCase();
  const url = esc(r.output_r2_url);
  const filename = esc(r.output_filename || 'output');
  let preview = '';
  if (ext === 'mp4' || ext === 'webm') {{
    preview = `<video controls muted loop preload="metadata">
      <source src="${{url}}" type="video/${{ext}}">
    </video>`;
  }} else if (ext === 'png' || ext === 'jpg' || ext === 'jpeg') {{
    preview = `<img src="${{url}}" alt="${{filename}}" loading="lazy">`;
  }}
  return `<div class="media-preview">
    ${{preview}}
    <br>
    <a class="download-btn" href="${{url}}" download="${{filename}}" target="_blank">
      ↓ Download ${{filename}}
    </a>
  </div>`;
}}

function renderReview(r) {{
  if (r.status === 'failed') {{
    return `<div class="error-msg">${{esc(r.error_message || 'Unknown error')}}</div>`;
  }}
  if (!r.llm_review_score) return '<div style="color:#666680;font-size:12px">Review pending...</div>';
  const st = scoreStyle(r.llm_review_score);
  return `<div class="review-box">
    <div class="review-score">
      <div class="score-circle" style="color:${{st.color}};border-color:${{st.border}}">
        ${{r.llm_review_score}}
      </div>
      <div>
        <div style="font-size:13px;font-weight:600;color:#fff">Gemini Review Score</div>
        <div class="review-label">out of 10 — reviewed by ${{esc(r.llm_reviewer||'gemini')}}</div>
      </div>
    </div>
    <div class="review-feedback">${{esc(r.llm_review_feedback||'')}}</div>
  </div>`;
}}

async function load() {{
  try {{
    const resp = await fetch(`/api/admin/test-runs/${{RUN_ID}}`, {{
      headers: {{ 'Authorization': 'Bearer ' + token }}
    }});
    const data = await resp.json();
    if (!resp.ok) {{ throw new Error(data.error || 'Server error'); }}

    const run = data.run;
    // Header
    document.getElementById('run-header').innerHTML = `
      <h1>${{esc(run.name)}}</h1>
      <span class="badge badge-${{run.status}}">${{run.status.replace(/_/g,' ')}}</span>`;

    // Stats
    const started = new Date(run.started_at).toLocaleString();
    const duration = run.completed_at
      ? Math.round((new Date(run.completed_at) - new Date(run.started_at)) / 60000) + ' min'
      : 'In progress';
    document.getElementById('stats-row').innerHTML = `
      <div class="stat-box stat-passed"><div class="stat-val">${{run.passed_tests}}</div><div class="stat-label">Passed</div></div>
      <div class="stat-box stat-failed"><div class="stat-val">${{run.failed_tests}}</div><div class="stat-label">Failed</div></div>
      <div class="stat-box"><div class="stat-val">${{run.total_tests}}</div><div class="stat-label">Total</div></div>
      <div class="stat-box"><div class="stat-val" style="font-size:14px">${{esc(started)}}</div><div class="stat-label">Started</div></div>
      <div class="stat-box"><div class="stat-val" style="font-size:14px">${{esc(duration)}}</div><div class="stat-label">Duration</div></div>`;

    // Auto-refresh note
    const note = document.getElementById('refresh-note');
    if (run.status === 'running') {{
      note.style.display = 'block';
      if (!autoRefresh) autoRefresh = setInterval(load, 20000);
    }} else {{
      note.style.display = 'none';
      if (autoRefresh) {{ clearInterval(autoRefresh); autoRefresh = null; }}
    }}

    // Results
    const container = document.getElementById('results-container');
    if (!data.results || data.results.length === 0) {{
      container.innerHTML = '<div style="color:#666680;text-align:center;padding:40px">No results yet...</div>';
      return;
    }}
    container.innerHTML = data.results.map(r => `
      <div class="result-card ${{r.status}}">
        <div class="result-header">
          <div>
            <span class="result-name">${{esc(r.test_name)}}</span>
            &nbsp;<span class="gig-tag">${{esc(r.gig_type)}}</span>
          </div>
          <span class="badge badge-${{r.status}}">${{r.status}}</span>
        </div>
        <div class="result-prompt">${{esc(r.prompt)}}</div>
        <div class="result-body">
          ${{renderMedia(r)}}
          <div class="result-info">
            ${{renderReview(r)}}
          </div>
        </div>
      </div>`).join('');
  }} catch(e) {{
    document.getElementById('results-container').innerHTML =
      `<div style="color:#f87171;text-align:center;padding:40px">Error: ${{esc(e.message)}}</div>`;
  }}
}}

load();
</script>
</body>
</html>"###, id = id);
    Html(html)
}

// ─────────────────────────────────────────────────────────────────────────────
// Public delivery page — shareable link for Fiverr / client handoff
// GET /delivery/:id
// ─────────────────────────────────────────────────────────────────────────────

pub async fn delivery_page(
    Path(id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
) -> Html<String> {
    // Try test_results first, then custom deliveries table
    let (name, gig_type, status, r2_url, filename, score, feedback, unlock_price, unlocked_until) =
        if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
            // 1. Portfolio test result
            let tr = sqlx::query(
                "SELECT test_name, gig_type, status, output_r2_url, output_filename, \
                 llm_review_score, llm_review_feedback FROM test_results WHERE id = $1",
            )
            .bind(uuid)
            .fetch_optional(&state.db_pool)
            .await
            .ok()
            .flatten();

            if let Some(row) = tr {
                let n: String = row.get("test_name");
                let g: String = row.get("gig_type");
                let s: String = row.get("status");
                let u: Option<String> = row.try_get("output_r2_url").ok();
                let f: Option<String> = row.try_get("output_filename").ok();
                let sc: Option<i32> = row.try_get("llm_review_score").ok();
                let fb: Option<String> = row.try_get("llm_review_feedback").ok();
                // Portfolio test results are never paywalled — they're
                // demos that need to be visible to anyone.
                (Some(n), Some(g), Some(s), u, f, sc, fb, None, None)
            } else {
                // 2. Custom delivery — these CAN be paywalled.
                let dr = sqlx::query(
                    "SELECT title, gig_type, status, output_r2_url, output_filename, \
                     unlock_price_usdc, unlocked_until \
                     FROM deliveries WHERE id = $1",
                )
                .bind(uuid)
                .fetch_optional(&state.db_pool)
                .await
                .ok()
                .flatten();

                if let Some(row) = dr {
                    let n: String = row.get("title");
                    let g: String = row.get("gig_type");
                    let s: String = row.get("status");
                    let u: Option<String> = row.try_get("output_r2_url").ok();
                    let f: Option<String> = row.try_get("output_filename").ok();
                    let price: Option<sqlx::types::Decimal> =
                        row.try_get("unlock_price_usdc").ok().flatten();
                    let until: Option<chrono::DateTime<chrono::Utc>> =
                        row.try_get("unlocked_until").ok().flatten();
                    (Some(n), Some(g), Some(s), u, f, None, None, price, until)
                } else {
                    (None, None, None, None, None, None, None, None, None)
                }
            }
        } else {
            (None, None, None, None, None, None, None, None, None)
        };

    // Unlocked = the deliverable can show full-quality download buttons.
    // Either: no price set (free delivery), or unlocked_until is in the future.
    let is_unlocked = unlock_price.is_none()
        || unlocked_until.map(|t| t > chrono::Utc::now()).unwrap_or(false);
    let price_cents: u64 = unlock_price.as_ref()
        .and_then(|d| {
            // Decimal → cents (multiply by 100, truncate fractional).
            use sqlx::types::Decimal;
            let hundred = Decimal::new(100, 0);
            let cents_dec = d * hundred;
            cents_dec.trunc().to_string().parse::<u64>().ok()
        })
        .unwrap_or(500);

    let result: Option<()> = name.as_ref().map(|_| ());

    let html = match result {
        None => format!(r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="UTF-8">
<title>Delivery Not Found — VideoSync</title>
<style>
  body {{ font-family: -apple-system, sans-serif; background: #0f0f13; color: #e0e0e0;
          display: flex; align-items: center; justify-content: center; min-height: 100vh; }}
  .box {{ text-align: center; }}
  h1 {{ font-size: 48px; color: #6366f1; margin-bottom: 12px; }}
  p {{ color: #9999bb; }}
</style></head>
<body><div class="box"><h1>404</h1><p>Delivery not found. The link may have expired.</p></div></body>
</html>"#),
        Some(()) => {
            let name = name.unwrap_or_default();
            let gig_type = gig_type.unwrap_or_default();
            let status = status.unwrap_or_default();
            let r2_url = r2_url;
            let filename = filename;
            let score = score;
            let feedback = feedback;

            let is_image = filename.as_deref()
                .map(|f| f.ends_with(".png") || f.ends_with(".jpg"))
                .unwrap_or(false);

            let media_html = match &r2_url {
                None => r#"<div class="no-media">⏳ Render in progress — check back shortly</div>"#.to_string(),
                Some(url) => {
                    if is_image {
                        let watermark_overlay = if !is_unlocked {
                            r#"<div class="watermark"><div class="watermark-label">PREVIEW · $5 USDC FOR HD</div></div>"#
                        } else { "" };
                        format!(r#"<div class="media-stack"><img src="{url}" alt="Delivered image" style="max-width:100%;border-radius:12px;">{watermark_overlay}</div>"#)
                    } else {
                        let watermark_overlay = if !is_unlocked {
                            r#"<div class="watermark"><div class="watermark-label">PREVIEW · $5 USDC FOR HD</div></div>"#
                        } else { "" };
                        // Disable right-click + downloads on the locked preview
                        // by removing `controlsList=download` and adding
                        // `disablepictureinpicture` etc. Best-effort.
                        let video_attrs = if is_unlocked {
                            "controls"
                        } else {
                            r#"controls controlsList="nodownload" disablepictureinpicture oncontextmenu="return false""#
                        };
                        format!(r#"<div class="media-stack">
<video {video_attrs} style="width:100%;border-radius:12px;background:#000;">
  <source src="{url}" type="video/mp4">
  Your browser does not support the video tag.
</video>{watermark_overlay}</div>"#)
                    }
                }
            };

            // Download button is gated. When LOCKED, show the unlock CTA
            // instead — this is the entire revenue surface.
            let download_btn = match (&r2_url, is_unlocked) {
                (None, _) => String::new(),
                (Some(url), true) => {
                    let fname = filename.as_deref().unwrap_or("output");
                    format!(r#"<a href="{url}" download="{fname}" class="btn-download">⬇ Download HD {fname}</a>"#)
                }
                (Some(_), false) => {
                    let dollars = price_cents as f64 / 100.0;
                    format!(r#"<div class="unlock-cta">
  <div class="unlock-headline">Preview only — unlock the HD download for ${dollars:.2} USDC</div>
  <div class="unlock-sub">Pay once with any wallet (Phantom, Coinbase, MetaMask) on Base. 30-day access. No account needed.</div>
  <button id="x402-unlock-btn" class="btn-download" style="background:#7a4cff;margin-top:8px">🔓 Pay ${dollars:.2} USDC &amp; Unlock</button>
  <div id="x402-status" style="margin-top:10px;font-size:13px;color:#9999bb"></div>
</div>"#)
                }
            };

            let score_html = match score {
                Some(s) if s > 0 => {
                    let stars: String = "★".repeat(s as usize / 2) + &"☆".repeat(5 - s as usize / 2);
                    let fb = feedback.as_deref().unwrap_or("");
                    format!(r#"<div class="review-box">
  <div class="score">AI Quality Score: <strong>{s}/10</strong> <span class="stars">{stars}</span></div>
  <p class="feedback">{fb}</p>
</div>"#)
                }
                _ => String::new(),
            };

            let status_color = match status.as_str() {
                "passed" => "#4ade80",
                "failed" => "#f87171",
                "running" => "#60a5fa",
                _ => "#9ca3af",
            };

            format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{name} — VideoSync Delivery</title>
<style>
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
          background: #0f0f13; color: #e0e0e0; min-height: 100vh;
          display: flex; flex-direction: column; align-items: center; padding: 40px 20px; }}
  .card {{ background: #1a1a24; border: 1px solid #2a2a36; border-radius: 16px;
           padding: 32px; max-width: 860px; width: 100%; }}
  .brand {{ font-size: 13px; font-weight: 700; letter-spacing: 0.1em; color: #6366f1;
             text-transform: uppercase; margin-bottom: 20px; }}
  h1 {{ font-size: 22px; font-weight: 700; color: #fff; margin-bottom: 6px; }}
  .meta {{ font-size: 13px; color: #9999bb; margin-bottom: 24px; }}
  .status-dot {{ display: inline-block; width: 8px; height: 8px; border-radius: 50%;
                  background: {status_color}; margin-right: 6px; vertical-align: middle; }}
  .media-wrap {{ margin-bottom: 24px; border-radius: 12px; overflow: hidden;
                  background: #12121a; padding: 4px; }}
  .no-media {{ padding: 60px; text-align: center; color: #666680; font-size: 15px; }}
  .btn-download {{ display: inline-block; background: #6366f1; color: #fff;
                   padding: 12px 28px; border-radius: 8px; text-decoration: none;
                   font-weight: 600; font-size: 15px; margin-bottom: 24px;
                   transition: background 0.15s; }}
  .btn-download:hover {{ background: #4f46e5; }}
  .review-box {{ background: #12121a; border: 1px solid #2a2a36; border-radius: 10px;
                  padding: 16px 20px; margin-top: 8px; }}
  .score {{ font-size: 14px; color: #9999bb; margin-bottom: 6px; }}
  .score strong {{ color: #fff; font-size: 16px; }}
  .stars {{ color: #facc15; font-size: 16px; margin-left: 6px; }}
  .feedback {{ font-size: 13px; color: #9999bb; line-height: 1.6; }}
  .footer {{ margin-top: 32px; font-size: 12px; color: #666680; text-align: center; }}
  .footer a {{ color: #6366f1; text-decoration: none; }}
  /* x402 paywall styling — preview = full-length, watermarked diagonally.
     Buyer sees the entire clip so they know what they're paying for, but
     the watermark reinforces the "pay $5 for HD" CTA on every frame. */
  .media-stack {{ position: relative; overflow: hidden; }}
  .watermark {{
    position: absolute; top: 0; left: 0; right: 0; bottom: 0;
    display: flex; align-items: center; justify-content: center;
    pointer-events: none; user-select: none;
    background-image: repeating-linear-gradient(
      -22deg,
      rgba(255,255,255,0.10) 0px,
      rgba(255,255,255,0.10) 2px,
      transparent 2px,
      transparent 170px
    );
  }}
  .watermark-label {{
    font-size: 22px; font-weight: 800; color: rgba(255,255,255,0.85);
    background: rgba(122,76,255,0.55); padding: 10px 24px; border-radius: 10px;
    letter-spacing: 0.10em; transform: rotate(-15deg);
    text-shadow: 0 2px 6px rgba(0,0,0,0.4);
    border: 1px solid rgba(255,255,255,0.25);
  }}
  .unlock-cta {{ background: linear-gradient(135deg, rgba(122,76,255,0.12), rgba(99,102,241,0.06));
                  border: 1px solid rgba(122,76,255,0.3); border-radius: 12px;
                  padding: 20px; margin-bottom: 24px; }}
  .unlock-headline {{ font-size: 16px; font-weight: 700; color: #fff; margin-bottom: 4px; }}
  .unlock-sub {{ font-size: 13px; color: #9999bb; line-height: 1.5; margin-bottom: 8px; }}
</style>
</head>
<body>
<div class="card">
  <div class="brand">VideoSync — AI Video Generation</div>
  <h1>{name}</h1>
  <div class="meta">
    <span class="status-dot"></span>
    Gig type: <strong>{gig_type}</strong> &nbsp;·&nbsp; Status: <strong>{status}</strong>
  </div>
  <div class="media-wrap">{media_html}</div>
  {download_btn}
  {score_html}
</div>
<div class="footer">
  Delivered by <a href="https://videosync.video">VideoSync</a> — AI-Powered Video Generation
</div>
<script>
// x402 paywall flow — runs only when the unlock button is rendered (locked state).
(function() {{
  const btn    = document.getElementById('x402-unlock-btn');
  const status = document.getElementById('x402-status');
  if (!btn) return; // Already unlocked; nothing to do.

  const setStatus = (msg, isError) => {{
    status.style.color = isError ? '#f87171' : '#9999bb';
    status.textContent = msg;
  }};

  btn.addEventListener('click', async () => {{
    btn.disabled = true;
    btn.textContent = 'Connecting wallet...';

    // Step 1 — pull the 402 payment spec from the server.
    let spec;
    try {{
      const r = await fetch(window.location.pathname + '/unlock-spec');
      if (!r.ok) throw new Error('unlock-spec returned ' + r.status);
      spec = await r.json();
    }} catch (e) {{
      setStatus('Could not fetch payment spec: ' + e.message, true);
      btn.disabled = false; btn.textContent = '🔓 Try again';
      return;
    }}

    const req = (spec.accepts || [])[0];
    if (!req) {{ setStatus('Payment spec missing requirements.', true); return; }}

    // Step 2 — find an EVM provider. Phantom exposes window.phantom.ethereum
    // when EVM mode is enabled; MetaMask + Coinbase Wallet expose window.ethereum.
    const provider = (window.phantom && window.phantom.ethereum) || window.ethereum;
    if (!provider) {{
      setStatus('No crypto wallet detected. Install Phantom, MetaMask, or Coinbase Wallet.', true);
      btn.disabled = false; btn.textContent = '🔓 Try again';
      return;
    }}

    // Step 3 — request accounts and ensure we're on Base mainnet (chainId 0x2105).
    let accounts;
    try {{
      accounts = await provider.request({{ method: 'eth_requestAccounts' }});
    }} catch (e) {{
      setStatus('Wallet connection rejected.', true);
      btn.disabled = false; btn.textContent = '🔓 Try again';
      return;
    }}
    const from = accounts[0];

    try {{
      await provider.request({{
        method: 'wallet_switchEthereumChain',
        params: [{{ chainId: '0x2105' }}],
      }});
    }} catch (switchErr) {{
      // Chain might not be added — try adding it.
      try {{
        await provider.request({{
          method: 'wallet_addEthereumChain',
          params: [{{
            chainId: '0x2105',
            chainName: 'Base',
            nativeCurrency: {{ name: 'Ether', symbol: 'ETH', decimals: 18 }},
            rpcUrls: ['https://mainnet.base.org'],
            blockExplorerUrls: ['https://basescan.org']
          }}]
        }});
      }} catch {{
        setStatus('Switch your wallet to Base network and try again.', true);
        btn.disabled = false; btn.textContent = '🔓 Try again';
        return;
      }}
    }}

    setStatus('Sign the USDC payment in your wallet to unlock...', false);
    btn.textContent = 'Awaiting signature...';

    // Step 4 — build the EIP-3009 transferWithAuthorization typed data.
    const validAfter  = 0;
    const validBefore = Math.floor(Date.now() / 1000) + req.maxTimeoutSeconds;
    const nonce       = '0x' + Array.from(crypto.getRandomValues(new Uint8Array(32)))
                                    .map(b => b.toString(16).padStart(2, '0')).join('');
    const typedData = {{
      types: {{
        EIP712Domain: [
          {{ name: 'name', type: 'string' }},
          {{ name: 'version', type: 'string' }},
          {{ name: 'chainId', type: 'uint256' }},
          {{ name: 'verifyingContract', type: 'address' }},
        ],
        TransferWithAuthorization: [
          {{ name: 'from',        type: 'address' }},
          {{ name: 'to',          type: 'address' }},
          {{ name: 'value',       type: 'uint256' }},
          {{ name: 'validAfter',  type: 'uint256' }},
          {{ name: 'validBefore', type: 'uint256' }},
          {{ name: 'nonce',       type: 'bytes32' }},
        ],
      }},
      primaryType: 'TransferWithAuthorization',
      domain: {{
        name: req.extra && req.extra.name    || 'USD Coin',
        version: req.extra && req.extra.version || '2',
        chainId: 8453, // Base mainnet
        verifyingContract: req.asset,
      }},
      message: {{
        from, to: req.payTo,
        value: req.maxAmountRequired,
        validAfter, validBefore, nonce,
      }},
    }};

    let signature;
    try {{
      signature = await provider.request({{
        method: 'eth_signTypedData_v4',
        params: [from, JSON.stringify(typedData)],
      }});
    }} catch (e) {{
      setStatus('Signature rejected.', true);
      btn.disabled = false; btn.textContent = '🔓 Try again';
      return;
    }}

    // Step 5 — POST signed authorization back to our /unlock endpoint.
    setStatus('Submitting payment to Base network...', false);
    btn.textContent = 'Settling on-chain...';

    const xPaymentBody = {{
      x402Version: 1,
      scheme:  req.scheme,
      network: req.network,
      payload: {{
        signature,
        authorization: {{
          from, to: req.payTo,
          value: req.maxAmountRequired,
          validAfter: String(validAfter),
          validBefore: String(validBefore),
          nonce,
        }},
      }},
    }};
    const xPaymentB64 = btoa(JSON.stringify(xPaymentBody));

    try {{
      const r = await fetch(window.location.pathname + '/unlock', {{
        method: 'POST',
        headers: {{ 'X-Payment': xPaymentB64 }},
      }});
      const data = await r.json();
      if (!r.ok || !data.success) {{
        throw new Error(data.error || ('HTTP ' + r.status));
      }}
      setStatus('✅ Unlocked! Reloading...', false);
      setTimeout(() => window.location.reload(), 800);
    }} catch (e) {{
      setStatus('Settlement failed: ' + e.message, true);
      btn.disabled = false; btn.textContent = '🔓 Try again';
    }}
  }});
}})();
</script>
</body>
</html>"#)
        }
    };

    Html(html)
}

// =============================================================================
// x402 paywall — public endpoints (no auth, payment IS the auth)
// =============================================================================

/// GET /delivery/:id/unlock-spec — returns the HTTP 402 payment requirements
/// the buyer's wallet needs to sign. We render this as 200 OK with the spec
/// in the body (rather than literal HTTP 402 with the spec) so the browser's
/// fetch() doesn't bin it as an error before the JS can parse it.
pub async fn delivery_unlock_spec(
    Path(id): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return Json(json!({"error": "Invalid delivery id"})),
    };

    // Look up the price + title for the spec description.
    let row = sqlx::query(
        "SELECT title, unlock_price_usdc FROM deliveries WHERE id = $1"
    )
    .bind(uuid)
    .fetch_optional(&state.db_pool)
    .await
    .ok()
    .flatten();

    let (title, price_usd) = match row {
        Some(r) => {
            let t: String = r.get("title");
            let p: Option<sqlx::types::Decimal> = r.try_get("unlock_price_usdc").ok().flatten();
            (t, p)
        }
        None => return Json(json!({"error": "Delivery not found"})),
    };

    let recipient = match std::env::var("X402_RECIPIENT_ADDRESS") {
        Ok(a) if !a.is_empty() => a,
        _ => return Json(json!({"error": "X402_RECIPIENT_ADDRESS env var not set on server"})),
    };

    // Default to $5 if the row has no price (legacy rows from before the
    // migration, or programmer-created deliveries that didn't set one).
    let price_cents: u64 = price_usd
        .and_then(|d| {
            use sqlx::types::Decimal;
            let hundred = Decimal::new(100, 0);
            (d * hundred).trunc().to_string().parse::<u64>().ok()
        })
        .unwrap_or(500);

    let resource_url = format!("{}/delivery/{}/unlock", base_url(), uuid);
    let description  = format!("Unlock HD download — {}", title);

    let spec = crate::x402::build_payment_required(price_cents, &recipient, &resource_url, &description);
    Json(serde_json::to_value(spec).unwrap_or(json!({"error": "spec serialise failed"})))
}

/// POST /delivery/:id/unlock — accepts the X-Payment header, settles via the
/// facilitator, flips the delivery row to unlocked-for-30-days, returns the
/// receipt id.
pub async fn delivery_unlock(
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    Extension(state): Extension<Arc<AppState>>,
) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => return (axum::http::StatusCode::BAD_REQUEST, Json(json!({"success": false, "error": "Invalid delivery id"}))),
    };

    let x_payment = match headers.get("X-Payment").and_then(|h| h.to_str().ok()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return (axum::http::StatusCode::PAYMENT_REQUIRED,
                     Json(json!({"success": false, "error": "Missing X-Payment header. Fetch /unlock-spec first, sign with your wallet, then retry with the signed payload."}))),
    };

    // Re-derive the same requirements we'd put in the 402 spec so the
    // facilitator can sanity-check the signature. Has to match exactly.
    let row = match sqlx::query(
        "SELECT title, unlock_price_usdc FROM deliveries WHERE id = $1"
    )
    .bind(uuid)
    .fetch_optional(&state.db_pool)
    .await {
        Ok(Some(r)) => r,
        Ok(None)    => return (axum::http::StatusCode::NOT_FOUND, Json(json!({"success": false, "error": "Delivery not found"}))),
        Err(e)      => return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"success": false, "error": format!("DB error: {}", e)}))),
    };

    let title: String = row.get("title");
    let price_usd: Option<sqlx::types::Decimal> = row.try_get("unlock_price_usdc").ok().flatten();
    let price_cents: u64 = price_usd
        .and_then(|d| {
            use sqlx::types::Decimal;
            let hundred = Decimal::new(100, 0);
            (d * hundred).trunc().to_string().parse::<u64>().ok()
        })
        .unwrap_or(500);

    let recipient = match std::env::var("X402_RECIPIENT_ADDRESS") {
        Ok(a) if !a.is_empty() => a,
        _ => return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                     Json(json!({"success": false, "error": "X402_RECIPIENT_ADDRESS not configured"}))),
    };

    let resource_url = format!("{}/delivery/{}/unlock", base_url(), uuid);
    let description  = format!("Unlock HD download — {}", title);
    let spec = crate::x402::build_payment_required(price_cents, &recipient, &resource_url, &description);
    let req  = match spec.accepts.first() {
        Some(r) => r.clone(),
        None    => return (axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                           Json(json!({"success": false, "error": "Failed to build payment requirements"}))),
    };

    let tx_hash = match crate::x402::settle_or_reject(&x_payment, &req).await {
        Ok(h)  => h,
        Err(e) => return (axum::http::StatusCode::PAYMENT_REQUIRED,
                          Json(json!({"success": false, "error": e}))),
    };

    // Persist the unlock + receipt. 30-day window.
    let _ = sqlx::query(
        "UPDATE deliveries
         SET unlocked_until = NOW() + INTERVAL '30 days',
             payment_receipt_id = $1
         WHERE id = $2"
    )
    .bind(&tx_hash)
    .bind(uuid)
    .execute(&state.db_pool)
    .await;

    // Admin Telegram ping — fire-and-forget.
    let tx_preview = tx_hash.chars().take(10).collect::<String>();
    let notify_text = format!(
        "💰 *Delivery HD unlock — ${:.2} USDC*\n\
         Delivery: `{}`\n\
         Tx: `{}...`",
        price_cents as f64 / 100.0,
        uuid,
        tx_preview
    );
    tokio::spawn(async move { crate::telegram_bot::notify_admin(&notify_text).await; });

    (axum::http::StatusCode::OK, Json(json!({
        "success":    true,
        "tx_hash":    tx_hash,
        "unlocked_for_days": 30,
    })))
}

/// Read the public base URL from env, falling back to videosync.video.
/// Used in x402 spec responses so the wallet shows the canonical resource.
fn base_url() -> String {
    std::env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "https://videosync.video".to_string())
}

// =============================================================================
// CUSTOM DELIVERIES — Freelance order delivery system
// Freelancer creates a delivery job → BlenderMCPServer renders → shareable link
// =============================================================================

#[derive(serde::Deserialize)]
pub struct CreateDeliveryRequest {
    pub client_ref:  Option<String>,
    pub title:       String,
    pub gig_type:    String,
    pub prompt:      String,
    pub style:       Option<String>,
    pub duration:    Option<f64>,
    pub extra:       Option<serde_json::Value>,
}

pub async fn api_create_delivery(
    Extension(state): Extension<Arc<AppState>>,
    Json(req): Json<CreateDeliveryRequest>,
) -> Json<serde_json::Value> {
    let style    = req.style.unwrap_or_else(|| "cinematic".to_string());
    let duration = req.duration.unwrap_or(10.0);
    let extra    = req.extra.unwrap_or(serde_json::Value::Null);

    let row = sqlx::query(
        "INSERT INTO deliveries (client_ref, title, gig_type, prompt, style, duration, extra_args, status) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending') RETURNING id",
    )
    .bind(&req.client_ref)
    .bind(&req.title)
    .bind(&req.gig_type)
    .bind(&req.prompt)
    .bind(&style)
    .bind(duration)
    .bind(&extra)
    .fetch_one(&state.db_pool)
    .await;

    let delivery_id: Uuid = match row {
        Ok(r) => r.get("id"),
        Err(e) => return Json(json!({"error": format!("DB insert failed: {e}")})),
    };

    // Spawn background render task
    let state_clone = state.clone();
    tokio::spawn(async move {
        run_delivery_job(delivery_id, state_clone).await;
    });

    Json(json!({"delivery_id": delivery_id.to_string()}))
}

pub async fn api_list_deliveries(
    Extension(state): Extension<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let rows = sqlx::query(
        "SELECT id, client_ref, title, gig_type, status, output_r2_url, \
         created_at, completed_at, error_message \
         FROM deliveries ORDER BY created_at DESC LIMIT 100",
    )
    .fetch_all(&state.db_pool)
    .await;

    let rows = match rows {
        Ok(r) => r,
        Err(e) => return Json(json!({"error": format!("DB query failed: {e}")})),
    };

    let deliveries: Vec<serde_json::Value> = rows.iter().map(|r| {
        json!({
            "id":           r.get::<Uuid, _>("id").to_string(),
            "client_ref":   r.try_get::<String, _>("client_ref").ok(),
            "title":        r.get::<String, _>("title"),
            "gig_type":     r.get::<String, _>("gig_type"),
            "status":       r.get::<String, _>("status"),
            "output_r2_url":  r.try_get::<String, _>("output_r2_url").ok(),
            "output_filename": r.try_get::<String, _>("output_filename").ok(),
            "created_at":   r.get::<chrono::DateTime<chrono::Utc>, _>("created_at").to_rfc3339(),
            "completed_at": r.try_get::<chrono::DateTime<chrono::Utc>, _>("completed_at").ok().map(|d| d.to_rfc3339()),
            "error":        r.try_get::<String, _>("error_message").ok(),
        })
    }).collect();

    Json(json!({"deliveries": deliveries}))
}

/// Background task: renders one custom delivery job and updates the DB.
pub async fn run_delivery_job(delivery_id: Uuid, state: Arc<AppState>) {
    // Fetch job details
    let row = sqlx::query(
        "SELECT gig_type, prompt, style, duration, extra_args FROM deliveries WHERE id = $1",
    )
    .bind(delivery_id)
    .fetch_one(&state.db_pool)
    .await;

    let row = match row {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Delivery {delivery_id}: DB fetch failed: {e}");
            return;
        }
    };

    let gig_type: String        = row.get("gig_type");
    let prompt:   String        = row.get("prompt");
    let style:    String        = row.get("style");
    let duration: f64           = row.get("duration");
    let extra: serde_json::Value = row.try_get::<serde_json::Value, _>("extra_args")
        .unwrap_or(serde_json::Value::Null);

    // Check BlenderMCPClient
    let blender = match state.blender_mcp_client.as_ref() {
        Some(c) => c.clone(),
        None => {
            let _ = sqlx::query(
                "UPDATE deliveries SET status='failed', error_message=$1, completed_at=NOW() WHERE id=$2",
            )
            .bind("BlenderMCPServer not configured (BLENDER_MCP_URL not set)")
            .bind(delivery_id)
            .execute(&state.db_pool)
            .await;
            return;
        }
    };

    // Mark running
    let _ = sqlx::query("UPDATE deliveries SET status='running' WHERE id=$1")
        .bind(delivery_id)
        .execute(&state.db_pool)
        .await;

    // Build tool + args
    let (tool, args, url_key, ext) = build_delivery_tool_args(&gig_type, &prompt, &style, duration, &extra);

    // Submit to BlenderMCPServer async job queue
    let job_id = match blender.submit_job(&tool, args).await {
        Ok(id) => id,
        Err(e) => {
            let _ = sqlx::query(
                "UPDATE deliveries SET status='failed', error_message=$1, completed_at=NOW() WHERE id=$2",
            )
            .bind(&e).bind(delivery_id).execute(&state.db_pool).await;
            return;
        }
    };

    // Poll until completed / failed (up to 15 min, 5s interval)
    let mut final_url: Option<String> = None;
    for _ in 0..180u16 {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let status = match blender.poll_job(&job_id).await {
            Ok(s) => s,
            Err(e) => {
                let _ = sqlx::query(
                    "UPDATE deliveries SET status='failed', error_message=$1, completed_at=NOW() WHERE id=$2",
                )
                .bind(&e).bind(delivery_id).execute(&state.db_pool).await;
                return;
            }
        };

        match status.get("state").and_then(|s| s.as_str()) {
            Some("completed") => {
                if let Some(url) = status.get("result")
                    .and_then(|r| r.get(url_key))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                {
                    final_url = Some(url);
                }
                break;
            }
            Some("failed") | Some("error") => {
                let msg = status.get("error").and_then(|v| v.as_str())
                    .unwrap_or("render failed").to_string();
                let _ = sqlx::query(
                    "UPDATE deliveries SET status='failed', error_message=$1, completed_at=NOW() WHERE id=$2",
                )
                .bind(&msg).bind(delivery_id).execute(&state.db_pool).await;
                return;
            }
            _ => {} // pending/running — keep polling
        }
    }

    match final_url {
        Some(url) => {
            // QA review — call Gemini to verify the output matches the
            // original brief. Logs to blender_render_reviews for admin
            // dashboard. Fail-open: if review fails, delivery still
            // completes (we don't want Gemini downtime to block clients).
            let mut review = crate::render_review::review_render(
                &state,
                &url,
                &prompt,
                &tool,
                Some(delivery_id),
            ).await;

            // AUTO-RETRY: if the review fails and Gemini gave a retry_hint,
            // re-render ONCE with the hint prepended to the prompt.
            // Keep whichever render scored higher.
            let mut best_url = url.clone();
            let mut best_score = review.score;
            let mut best_review = review.clone();

            if !review.pass && review.score > 0 {
                if let Some(hint) = review.retry_hint.as_ref() {
                    tracing::info!("🔄 Auto-retrying delivery {} (score {}) with hint: {}", delivery_id, review.score, hint);
                    let improved_prompt = format!("{hint}. {prompt}");
                    let (_, retry_args, _, _) = build_delivery_tool_args(&gig_type, &improved_prompt, &style, duration, &extra);

                    if let Ok(retry_job_id) = blender.submit_job(&tool, retry_args).await {
                        let mut retry_url: Option<String> = None;
                        for _ in 0..180u16 {
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                            if let Ok(status) = blender.poll_job(&retry_job_id).await {
                                match status.get("state").and_then(|s| s.as_str()) {
                                    Some("completed") => {
                                        retry_url = status.get("result")
                                            .and_then(|r| r.get(url_key))
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string());
                                        break;
                                    }
                                    Some("failed") | Some("error") => break,
                                    _ => {}
                                }
                            }
                        }

                        if let Some(retry_url_str) = retry_url {
                            let retry_review = crate::render_review::review_render(
                                &state,
                                &retry_url_str,
                                &improved_prompt,
                                &tool,
                                Some(delivery_id),
                            ).await;
                            tracing::info!("🔄 Retry review: score {} (was {})", retry_review.score, best_score);
                            if retry_review.score > best_score {
                                best_url = retry_url_str;
                                best_score = retry_review.score;
                                best_review = retry_review;
                            }
                        }
                    }
                }
            }

            review = best_review;
            let url = best_url;

            let filename = format!("delivery_{delivery_id}.{ext}");
            let error_note: Option<String> = if !review.pass && review.score > 0 {
                Some(format!("QA warning (score {}): {}", review.score, review.feedback))
            } else { None };

            let _ = sqlx::query(
                "UPDATE deliveries SET status='completed', output_r2_url=$1, output_filename=$2, \
                 error_message=$3, completed_at=NOW() WHERE id=$4",
            )
            .bind(&url).bind(&filename).bind(error_note.as_deref()).bind(delivery_id)
            .execute(&state.db_pool).await;

            if !review.pass {
                tracing::warn!(
                    "⚠️ QA review flagged delivery {} even after retry: score={}, feedback={}",
                    delivery_id, review.score, review.feedback
                );
            }
        }
        None => {
            let _ = sqlx::query(
                "UPDATE deliveries SET status='failed', error_message='Timed out after 900s', \
                 completed_at=NOW() WHERE id=$1",
            )
            .bind(delivery_id).execute(&state.db_pool).await;
        }
    }
}

/// Map gig_type → (tool_name, args, url_key, file_extension)
fn build_delivery_tool_args(
    gig_type: &str,
    prompt: &str,
    style: &str,
    duration: f64,
    extra: &serde_json::Value,
) -> (String, serde_json::Value, &'static str, &'static str) {
    let get = |key: &str| extra.get(key).and_then(|v| v.as_str()).unwrap_or("");
    match gig_type {
        "thumbnail" => (
            "blender_generate_thumbnail".to_string(),
            json!({"prompt": prompt, "title_text": get("title_text"), "style": style}),
            "image_url", "png",
        ),
        "title_card" => (
            "blender_generate_title_card".to_string(),
            json!({"title": prompt, "subtitle": get("subtitle"), "duration": duration, "style": style}),
            "video_url", "mp4",
        ),
        "data_viz" => (
            "blender_generate_data_viz".to_string(),
            json!({"data_json": get("data_json"), "chart_type": get("chart_type"), "title": prompt, "duration": duration}),
            "video_url", "mp4",
        ),
        "lower_third" => (
            "blender_generate_lower_third".to_string(),
            json!({"name_text": prompt, "subtitle_text": get("subtitle"), "style": style, "duration": duration}),
            "video_url", "mp4",
        ),
        "latex" => (
            "blender_generate_latex".to_string(),
            json!({"latex_expression": prompt, "animation_type": get("animation_type"), "duration": duration, "background_style": get("background_style")}),
            "video_url", "mp4",
        ),
        "ui_mockup" => {
            let animation = get("animation");
            let url_key = if animation == "static" { "image_url" } else { "video_url" };
            let ext     = if animation == "static" { "png" }       else { "mp4" };
            (
                "blender_generate_ui_mockup".to_string(),
                json!({"device": get("device"), "animation": animation, "duration": duration, "screenshot_url": get("screenshot_url")}),
                url_key, ext,
            )
        }
        _ => { // "scene" + default — includes landing_page service type
            let mut scene_args = json!({"prompt": prompt, "duration": duration, "style": style});
            let ref_url = get("reference_image_url");
            if !ref_url.is_empty() {
                scene_args["reference_image_url"] = serde_json::Value::String(ref_url.to_string());
            }
            (
                "blender_generate_scene".to_string(),
                scene_args,
                "video_url", "mp4",
            )
        }
    }
}

pub async fn admin_deliveries_page() -> Html<String> {
    let html = r###"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Custom Deliveries — VideoSync Admin</title>
<style>
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
         background: #0f0f13; color: #e0e0e0; display: flex; min-height: 100vh; }
  .sidebar { width: 220px; background: #1a1a24; padding: 24px 0; flex-shrink: 0; }
  .sidebar h2 { color: #fff; font-size: 14px; font-weight: 700; padding: 0 20px 20px;
                border-bottom: 1px solid #2a2a36; letter-spacing: 0.05em; }
  .sidebar a { display: block; padding: 10px 20px; color: #9999bb; text-decoration: none;
               font-size: 13px; transition: all 0.15s; }
  .sidebar a:hover, .sidebar a.active { background: #23233a; color: #fff; }
  .main { flex: 1; padding: 32px; overflow-y: auto; }
  .header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 28px; }
  .header h1 { font-size: 22px; font-weight: 600; color: #fff; }
  .btn { padding: 10px 20px; border: none; border-radius: 8px; cursor: pointer;
         font-size: 13px; font-weight: 600; transition: all 0.2s; }
  .btn-primary { background: #6c5ce7; color: #fff; }
  .btn-primary:hover { background: #5a4bd1; }
  .btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
  .btn-ghost { background: transparent; border: 1px solid #2a2a36; color: #9999bb; }
  .btn-ghost:hover { border-color: #6c5ce7; color: #fff; }
  .btn-sm { padding: 6px 14px; font-size: 12px; }
  .btn-copy { background: #1a3a2a; border: 1px solid #2a5a3a; color: #4ade80; }
  .btn-copy:hover { background: #1f4a33; }
  .form-panel { background: #1a1a24; border: 1px solid #2a2a36; border-radius: 12px;
                padding: 28px; margin-bottom: 32px; }
  .form-panel h2 { font-size: 16px; font-weight: 600; color: #fff; margin-bottom: 20px; }
  .form-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
  .form-full { grid-column: 1 / -1; }
  .form-group { display: flex; flex-direction: column; gap: 6px; }
  .form-group label { font-size: 12px; color: #9999bb; font-weight: 600; text-transform: uppercase; letter-spacing: 0.05em; }
  .form-group input, .form-group select, .form-group textarea {
    background: #0f0f13; border: 1px solid #2a2a36; border-radius: 7px;
    padding: 10px 14px; color: #fff; font-size: 14px; width: 100%;
    font-family: inherit; }
  .form-group textarea { min-height: 80px; resize: vertical; }
  .form-group input:focus, .form-group select:focus, .form-group textarea:focus {
    outline: none; border-color: #6c5ce7; }
  .form-group select option { background: #1a1a24; }
  .hidden { display: none !important; }
  .form-actions { margin-top: 20px; display: flex; gap: 12px; align-items: center; }
  .spinner { display: inline-block; width: 14px; height: 14px; border: 2px solid rgba(255,255,255,0.3);
             border-top-color: #fff; border-radius: 50%; animation: spin 0.7s linear infinite;
             margin-right: 8px; vertical-align: middle; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .success-banner { background: #16a34a22; border: 1px solid #4ade8040; border-radius: 8px;
                    padding: 14px 18px; font-size: 13px; color: #4ade80; display: none; margin-top: 16px; }
  .error-banner  { background: #dc262622; border: 1px solid #f8717140; border-radius: 8px;
                    padding: 14px 18px; font-size: 13px; color: #f87171; display: none; margin-top: 16px; }
  .table-wrap { background: #1a1a24; border: 1px solid #2a2a36; border-radius: 12px; overflow: hidden; }
  .table-header { padding: 16px 20px; border-bottom: 1px solid #2a2a36; display: flex;
                  justify-content: space-between; align-items: center; }
  .table-header h2 { font-size: 15px; font-weight: 600; color: #fff; }
  table { width: 100%; border-collapse: collapse; }
  th { padding: 12px 16px; text-align: left; font-size: 11px; color: #666680;
       font-weight: 600; text-transform: uppercase; letter-spacing: 0.06em;
       border-bottom: 1px solid #2a2a36; }
  td { padding: 14px 16px; font-size: 13px; border-bottom: 1px solid #1f1f2e; vertical-align: middle; }
  tr:last-child td { border-bottom: none; }
  tr:hover td { background: #1f1f2a; }
  .badge { padding: 3px 10px; border-radius: 20px; font-size: 11px; font-weight: 600; }
  .badge-pending  { background: #2a2a3a; color: #9999bb; }
  .badge-running  { background: #2563eb22; color: #60a5fa; }
  .badge-completed { background: #16a34a22; color: #4ade80; }
  .badge-failed   { background: #dc262622; color: #f87171; }
  .gig-tag { display: inline-block; background: #6c5ce722; color: #a99ef7;
             padding: 2px 8px; border-radius: 4px; font-size: 11px; }
  .client-ref { font-size: 12px; color: #666680; }
  .empty { text-align: center; padding: 60px; color: #666680; }
  .link-cell { display: flex; gap: 8px; align-items: center; }
  .link-text { font-size: 11px; color: #666680; font-family: monospace; }
  .separator { height: 1px; background: #2a2a36; margin: 28px 0; }
  .hint { font-size: 12px; color: #666680; margin-top: 4px; }
</style>
</head>
<body>
<div class="sidebar">
  <h2>VIDEOSYNC ADMIN</h2>
  <a href="/admin/dashboard">Dashboard</a>
  <a href="/admin/users">Users</a>
  <a href="/admin/clipping-jobs">Clipping Jobs</a>
  <a href="/admin/clipping-activity">Activity</a>
  <a href="/admin/performance">Performance</a>
  <a href="/admin/test-runs">Portfolio Tests</a>
  <a href="/admin/deliveries" class="active">Deliveries</a>
</div>
<div class="main">
  <div class="header">
    <h1>Custom Deliveries</h1>
    <button class="btn btn-ghost" onclick="loadDeliveries()">↻ Refresh</button>
  </div>

  <!-- Creation Form -->
  <div class="form-panel">
    <h2>Create New Delivery</h2>
    <div class="form-grid">
      <div class="form-group">
        <label>Client Reference (optional)</label>
        <input id="client_ref" type="text" placeholder="e.g. Fiverr #12345 / @username">
      </div>
      <div class="form-group">
        <label>Delivery Title *</label>
        <input id="title" type="text" placeholder="e.g. Title Card for TechBro Channel">
        <span class="hint">Shown on the client's delivery page</span>
      </div>
      <div class="form-group">
        <label>Gig Type *</label>
        <select id="gig_type" onchange="onGigTypeChange()">
          <option value="scene">3D Scene / B-Roll Clip</option>
          <option value="thumbnail">YouTube Thumbnail</option>
          <option value="title_card">Animated Title Card</option>
          <option value="data_viz">Data Visualisation</option>
          <option value="lower_third">Lower Third Overlay</option>
          <option value="latex">LaTeX / Math Animation</option>
          <option value="ui_mockup">UI Mockup (Phone/Screen)</option>
        </select>
      </div>
      <div class="form-group" id="grp-style">
        <label>Style</label>
        <select id="style">
          <option value="cinematic">Cinematic</option>
          <option value="corporate">Corporate</option>
          <option value="minimal">Minimal</option>
          <option value="energetic">Energetic</option>
          <option value="calm">Calm</option>
          <option value="dark">Dark</option>
          <option value="professional">Professional</option>
        </select>
      </div>
      <div class="form-group form-full">
        <label id="prompt-label">Scene Description *</label>
        <textarea id="prompt" placeholder="Describe what you want rendered..."></textarea>
      </div>
      <!-- Thumbnail: overlay title text -->
      <div class="form-group hidden" id="grp-title-text">
        <label>Overlay Title Text</label>
        <input id="title_text" type="text" placeholder="e.g. 10 AI Tools That Will Change Your Life">
      </div>
      <!-- Title Card / Lower Third: subtitle -->
      <div class="form-group hidden" id="grp-subtitle">
        <label id="subtitle-label">Subtitle</label>
        <input id="subtitle" type="text" placeholder="Secondary line of text">
      </div>
      <!-- Duration -->
      <div class="form-group" id="grp-duration">
        <label>Duration (seconds)</label>
        <input id="duration" type="number" value="10" min="3" max="60" step="1">
      </div>
      <!-- Data Viz: chart type + data JSON -->
      <div class="form-group hidden" id="grp-chart-type">
        <label>Chart Type</label>
        <select id="chart_type">
          <option value="bar">Bar Chart</option>
          <option value="line">Line Chart</option>
          <option value="pie">Pie Chart</option>
          <option value="counter">Animated Counter</option>
          <option value="scatter">Scatter Plot</option>
        </select>
      </div>
      <div class="form-group form-full hidden" id="grp-data-json">
        <label>Data JSON</label>
        <textarea id="data_json" placeholder='[{"label":"Q1","value":120},{"label":"Q2","value":185}]' style="min-height:60px;font-family:monospace;font-size:12px;"></textarea>
      </div>
      <!-- LaTeX: animation type + background -->
      <div class="form-group hidden" id="grp-anim-type">
        <label>Animation Type</label>
        <select id="animation_type">
          <option value="step_by_step">Step by Step</option>
          <option value="appear">Appear</option>
          <option value="morph">Morph / Transform</option>
        </select>
      </div>
      <div class="form-group hidden" id="grp-bg-style">
        <label>Background Style</label>
        <select id="background_style">
          <option value="dark">Dark</option>
          <option value="light">Light</option>
          <option value="transparent">Transparent</option>
        </select>
      </div>
      <!-- UI Mockup: device + animation -->
      <div class="form-group hidden" id="grp-device">
        <label>Device</label>
        <select id="device">
          <option value="iPhone">iPhone</option>
          <option value="MacBook">MacBook</option>
          <option value="browser">Browser Window</option>
          <option value="iPad">iPad</option>
        </select>
      </div>
      <div class="form-group hidden" id="grp-mockup-anim">
        <label>Mockup Animation</label>
        <select id="mockup_animation">
          <option value="reveal">Reveal</option>
          <option value="scroll">Scroll</option>
          <option value="tilt">Tilt / 3D Rotate</option>
          <option value="static">Static (image)</option>
        </select>
      </div>
      <div class="form-group hidden" id="grp-screenshot-url">
        <label>Screenshot URL (optional)</label>
        <input id="screenshot_url" type="url" placeholder="https://...">
        <span class="hint">Direct image URL to show in the mockup frame</span>
      </div>
      <div class="form-group hidden" id="grp-ref-image-url">
        <label>Landing Page / Reference Image URL (optional)</label>
        <input id="reference_image_url" type="url" placeholder="https://example.com or https://example.com/hero.png">
        <span class="hint">Pass a website URL or direct image URL — hero image will be scraped and used as the scene background</span>
      </div>
    </div>
    <div class="form-actions">
      <button class="btn btn-primary" id="create-btn" onclick="createDelivery()">🚀 Create Delivery</button>
      <span id="create-status" style="font-size:13px;color:#9999bb;"></span>
    </div>
    <div class="success-banner" id="success-banner"></div>
    <div class="error-banner"   id="error-banner"></div>
  </div>

  <!-- Deliveries Table -->
  <div class="table-wrap">
    <div class="table-header">
      <h2>All Deliveries</h2>
      <span id="delivery-count" style="font-size:12px;color:#666680;"></span>
    </div>
    <table>
      <thead>
        <tr>
          <th>Title</th>
          <th>Gig</th>
          <th>Client Ref</th>
          <th>Status</th>
          <th>Created</th>
          <th>Delivery Link</th>
        </tr>
      </thead>
      <tbody id="deliveries-tbody">
        <tr><td colspan="6" class="empty">Loading…</td></tr>
      </tbody>
    </table>
  </div>
</div>

<script>
const GIG_CONFIG = {
  scene:       { promptLabel:'Scene Description', style:true,  subtitle:false, titleText:false, duration:true,  chartType:false, dataJson:false, animType:false, bgStyle:false, device:false, mockupAnim:false, screenshotUrl:false, refImageUrl:true, defaultDuration:15 },
  thumbnail:   { promptLabel:'Thumbnail Description', style:true,  subtitle:false, titleText:true,  duration:false, chartType:false, dataJson:false, animType:false, bgStyle:false, device:false, mockupAnim:false, screenshotUrl:false, defaultDuration:0  },
  title_card:  { promptLabel:'Main Title Text', style:true,  subtitle:true,  titleText:false, duration:true,  chartType:false, dataJson:false, animType:false, bgStyle:false, device:false, mockupAnim:false, screenshotUrl:false, defaultDuration:5  },
  data_viz:    { promptLabel:'Chart Title',     style:false, subtitle:false, titleText:false, duration:true,  chartType:true,  dataJson:true,  animType:false, bgStyle:false, device:false, mockupAnim:false, screenshotUrl:false, defaultDuration:15 },
  lower_third: { promptLabel:'Name / Main Text', style:true,  subtitle:true,  titleText:false, duration:true,  chartType:false, dataJson:false, animType:false, bgStyle:false, device:false, mockupAnim:false, screenshotUrl:false, defaultDuration:5  },
  latex:       { promptLabel:'LaTeX Expression (e.g. \\\\frac{d}{dx})', style:false, subtitle:false, titleText:false, duration:true,  chartType:false, dataJson:false, animType:true,  bgStyle:true,  device:false, mockupAnim:false, screenshotUrl:false, defaultDuration:10 },
  ui_mockup:   { promptLabel:'App / Product Description', style:false, subtitle:false, titleText:false, duration:true,  chartType:false, dataJson:false, animType:false, bgStyle:false, device:true,  mockupAnim:true,  screenshotUrl:true,  defaultDuration:8  },
};

function show(id, v) { document.getElementById(id).classList.toggle('hidden', !v); }

function onGigTypeChange() {
  const cfg = GIG_CONFIG[document.getElementById('gig_type').value] || GIG_CONFIG.scene;
  document.getElementById('prompt-label').textContent = cfg.promptLabel;
  show('grp-style',         cfg.style);
  show('grp-subtitle',      cfg.subtitle);
  show('grp-title-text',    cfg.titleText);
  show('grp-duration',      cfg.duration);
  show('grp-chart-type',    cfg.chartType);
  show('grp-data-json',     cfg.dataJson);
  show('grp-anim-type',     cfg.animType);
  show('grp-bg-style',      cfg.bgStyle);
  show('grp-device',        cfg.device);
  show('grp-mockup-anim',   cfg.mockupAnim);
  show('grp-screenshot-url',cfg.screenshotUrl);
  if (cfg.defaultDuration) document.getElementById('duration').value = cfg.defaultDuration;
  // Update subtitle label
  const gt = document.getElementById('gig_type').value;
  document.getElementById('subtitle-label').textContent = (gt === 'lower_third') ? 'Subtitle / Role' : 'Subtitle';
}

async function createDelivery() {
  const btn = document.getElementById('create-btn');
  const status = document.getElementById('create-status');
  const successBanner = document.getElementById('success-banner');
  const errorBanner = document.getElementById('error-banner');
  successBanner.style.display = 'none';
  errorBanner.style.display = 'none';

  const gig = document.getElementById('gig_type').value;
  const cfg = GIG_CONFIG[gig];

  const title = document.getElementById('title').value.trim();
  const prompt = document.getElementById('prompt').value.trim();
  if (!title || !prompt) { errorBanner.textContent = 'Title and prompt are required.'; errorBanner.style.display='block'; return; }

  btn.disabled = true;
  btn.innerHTML = '<span class="spinner"></span>Creating…';

  const extra = {};
  if (cfg.titleText)     extra.title_text      = document.getElementById('title_text').value;
  if (cfg.subtitle)      extra.subtitle         = document.getElementById('subtitle').value;
  if (cfg.chartType)     extra.chart_type       = document.getElementById('chart_type').value;
  if (cfg.dataJson)      extra.data_json        = document.getElementById('data_json').value;
  if (cfg.animType)      extra.animation_type   = document.getElementById('animation_type').value;
  if (cfg.bgStyle)       extra.background_style = document.getElementById('background_style').value;
  if (cfg.device)        extra.device           = document.getElementById('device').value;
  if (cfg.mockupAnim)    extra.animation        = document.getElementById('mockup_animation').value;
  if (cfg.screenshotUrl) extra.screenshot_url   = document.getElementById('screenshot_url').value;
  if (cfg.refImageUrl)   extra.reference_image_url = document.getElementById('reference_image_url').value;

  const token = localStorage.getItem('auth_token') || localStorage.getItem('admin_token');
  const body = {
    client_ref: document.getElementById('client_ref').value.trim() || null,
    title, gig_type: gig, prompt,
    style:    cfg.style    ? document.getElementById('style').value    : 'cinematic',
    duration: cfg.duration ? parseFloat(document.getElementById('duration').value) : 10,
    extra: Object.keys(extra).length ? extra : null,
  };

  try {
    const resp = await fetch('/api/admin/deliveries', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'Authorization': `Bearer ${token}` },
      body: JSON.stringify(body),
    });
    const data = await resp.json();
    if (data.error) throw new Error(data.error);
    const url = `${location.origin}/delivery/${data.delivery_id}`;
    successBanner.innerHTML = `✅ Delivery created! Render is in progress (5–15 min).<br><br>
      <strong>Client link:</strong> <a href="${url}" target="_blank" style="color:#a99ef7">${url}</a><br><br>
      <button class="btn btn-copy btn-sm" onclick="navigator.clipboard.writeText('${url}');this.textContent='✓ Copied'">Copy Link</button>
      <span style="font-size:12px;color:#666680;margin-left:8px">Download button will appear in the table below once rendering completes.</span>`;
    successBanner.style.display = 'block';
    loadDeliveries();
  } catch(e) {
    errorBanner.textContent = `Error: ${e.message}`;
    errorBanner.style.display = 'block';
  } finally {
    btn.disabled = false;
    btn.innerHTML = '🚀 Create Delivery';
    status.textContent = '';
  }
}

function fmtDate(iso) {
  const d = new Date(iso);
  return d.toLocaleDateString() + ' ' + d.toLocaleTimeString([], {hour:'2-digit',minute:'2-digit'});
}

async function loadDeliveries() {
  const token = localStorage.getItem('auth_token') || localStorage.getItem('admin_token');
  try {
    const resp = await fetch('/api/admin/deliveries', {
      headers: { 'Authorization': `Bearer ${token}` }
    });
    const data = await resp.json();
    const deliveries = data.deliveries || [];
    document.getElementById('delivery-count').textContent = `${deliveries.length} total`;

    if (!deliveries.length) {
      document.getElementById('deliveries-tbody').innerHTML =
        '<tr><td colspan="6" class="empty">No deliveries yet. Create one above.</td></tr>';
      return;
    }

    document.getElementById('deliveries-tbody').innerHTML = deliveries.map(d => {
      const deliveryUrl = `${location.origin}/delivery/${d.id}`;
      const r2Url = d.output_r2_url || '';
      const fname = d.output_filename || `delivery_${d.id.substring(0,8)}`;
      const linkCell = d.status === 'completed'
        ? `<div class="link-cell">
             <button class="btn btn-copy btn-sm" onclick="navigator.clipboard.writeText('${deliveryUrl}');this.textContent='✓ Copied';setTimeout(()=>this.textContent='Copy Link',2000)">Copy Link</button>
             <a href="${deliveryUrl}" target="_blank" style="font-size:11px;color:#6c5ce7">Open ↗</a>
             ${r2Url ? `<a href="${r2Url}" download="${fname}" class="btn btn-sm" style="background:#1a2a3a;border:1px solid #2a4a6a;color:#60a5fa;text-decoration:none;">⬇ Download</a>` : ''}
           </div>`
        : d.status === 'failed'
          ? `<span style="font-size:11px;color:#f87171">${(d.error||'').substring(0,60)}</span>`
          : `<span class="link-text">rendering…</span>`;
      return `<tr>
        <td><strong>${d.title}</strong></td>
        <td><span class="gig-tag">${d.gig_type}</span></td>
        <td><span class="client-ref">${d.client_ref || '—'}</span></td>
        <td><span class="badge badge-${d.status}">${d.status}</span></td>
        <td style="font-size:12px;color:#666680">${fmtDate(d.created_at)}</td>
        <td>${linkCell}</td>
      </tr>`;
    }).join('');
  } catch(e) {
    document.getElementById('deliveries-tbody').innerHTML =
      `<tr><td colspan="6" class="empty">Error loading deliveries: ${e.message}</td></tr>`;
  }
}

// Init
onGigTypeChange();
loadDeliveries();
// Auto-refresh every 30s to pick up completed renders
setInterval(loadDeliveries, 30000);
</script>
</body>
</html>"###;
    Html(html.to_string())
}

pub async fn admin_monetization_guide_page() -> Html<&'static str> {
    Html(MONETIZATION_GUIDE_HTML)
}

static MONETIZATION_GUIDE_HTML: &str = r###"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Monetization Guide — VideoSync Admin</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:'Inter',system-ui,sans-serif;background:#1a1a2e;color:#e0e0e0;min-height:100vh}
.header{background:#16213e;border-bottom:1px solid #0f3460;padding:14px 24px;display:flex;align-items:center;gap:16px}
.header h1{font-size:1.2rem;color:#dbd8e3}
.back{color:#5c5470;text-decoration:none;font-size:0.9rem}
.back:hover{color:#dbd8e3}
.container{max-width:960px;margin:0 auto;padding:24px}
.card{background:#16213e;border:1px solid #0f3460;border-radius:12px;padding:24px;margin-bottom:24px}
h2{font-size:1.05rem;color:#dbd8e3;margin-bottom:16px;display:flex;align-items:center;gap:8px}
h3{font-size:0.95rem;color:#a78bfa;margin:14px 0 8px}
p,li{font-size:0.88rem;color:#c4b5fd;line-height:1.6}
ul,ol{padding-left:20px;margin-bottom:10px}
li{margin-bottom:4px}
a{color:#818cf8;text-decoration:none}
a:hover{text-decoration:underline}
table{width:100%;border-collapse:collapse;font-size:0.85rem;margin-bottom:12px}
th{text-align:left;padding:8px 12px;color:#9ca3af;border-bottom:1px solid #1e3a5f;font-weight:500}
td{padding:8px 12px;border-bottom:1px solid #0f3460;color:#e0e0e0}
.highlight{color:#6ee7b7;font-weight:600}
.warning{color:#fcd34d}
.dm-box{background:#0f3460;border:1px solid #1e3a5f;border-radius:8px;padding:12px;font-size:0.82rem;color:#ccc;white-space:pre-wrap;margin-bottom:8px;font-family:monospace}
.btn-copy{padding:4px 10px;background:#5c5470;color:#fff;border:none;border-radius:5px;cursor:pointer;font-size:0.78rem}
.day-badge{display:inline-block;padding:2px 10px;border-radius:10px;font-size:0.78rem;font-weight:600;margin-bottom:8px}
.day1{background:#065f46;color:#6ee7b7}
.day2{background:#1e3a5f;color:#93c5fd}
.day3{background:#4c1d95;color:#c4b5fd}
.platform-badge{display:inline-block;padding:2px 8px;border-radius:6px;font-size:0.78rem;font-weight:500;margin:2px}
.pb-up{background:#1e3a5f;color:#93c5fd}
.pb-dc{background:#312e81;color:#c4b5fd}
.total-box{background:#0f3460;border:2px solid #5c5470;border-radius:12px;padding:16px;text-align:center;margin-top:16px}
.total-box .amount{font-size:1.8rem;font-weight:700;color:#6ee7b7}
</style>
</head>
<body>
<div class="header">
  <a href="/admin/dashboard" class="back">← Dashboard</a>
  <h1>Monetization Guide</h1>
  <a href="/admin/prospect-finder" style="margin-left:auto;padding:6px 14px;background:#5c5470;color:#fff;border-radius:6px;font-size:0.85rem;text-decoration:none">Open Prospect Finder</a>
</div>
<div class="container">

  <div class="card">
    <h2>72-Hour Revenue Target</h2>
    <div class="total-box">
      <div class="amount">$3,000</div>
      <div style="color:#9ca3af;font-size:0.85rem;margin-top:4px">Target: April 3-5, 2026</div>
    </div>
    <p style="margin-top:16px">The platform's unique advantage: <span class="highlight">any YouTube or Twitch URL to download-ready clips in ~10 minutes.</span> No manual clipper can match this. Send free demos before the first DM — that is the entire pitch.</p>
  </div>

  <div class="card">
    <h2>Pricing Tiers</h2>
    <table>
      <thead><tr><th>Tier</th><th>Clips/Month</th><th>Price</th><th>Pitch</th></tr></thead>
      <tbody>
        <tr><td>Demo Pack</td><td>10 clips</td><td class="highlight">$97 one-time</td><td>Conversion tool — gets relationship started</td></tr>
        <tr><td>Starter</td><td>30 clips</td><td class="highlight">$297/month</td><td>"30 viral moments, done for you"</td></tr>
        <tr><td>Growth</td><td>50 clips</td><td class="highlight">$497/month</td><td>"Post daily Shorts without lifting a finger"</td></tr>
        <tr><td>Agency</td><td>100 clips + thumbnails</td><td class="highlight">$997/month</td><td>"Full content team, AI-powered"</td></tr>
        <tr><td>Podcast per-episode</td><td>5-10 clips/episode</td><td class="highlight">$150-$350/episode</td><td>Auto-clip every new episode within 24hr</td></tr>
        <tr><td>Course Visual Kit</td><td>5 Manim + 3 titles + 5 thumbs</td><td class="highlight">$497/kit</td><td>AI-generated course visuals, 24hr turnaround</td></tr>
        <tr><td>Product Demo Video</td><td>60-90s SaaS/e-com demo</td><td class="highlight">$497-$997</td><td>Manim + Blender pipeline, 48hr delivery</td></tr>
      </tbody>
    </table>
    <p><span class="highlight">Volume moat:</span> Manual clippers offer 5-10 clips/month. Your AI does 50+ at zero marginal cost. Lead with that number in every pitch.</p>
    <p style="margin-top:8px"><span class="highlight">Monthly target (50 clients x $297):</span> $14,850/month recurring</p>
  </div>

  <div class="card">
    <h2>Day 1 Today, April 3 — Target: $1,000</h2>
    <span class="day-badge day1">Day 1</span>
    <h3>Core Move: DM 30 creators with a free demo clip of their own content</h3>
    <ol>
      <li><strong>Find 30 targets (1 hr)</strong> — Open <a href="/admin/prospect-finder">Prospect Finder</a>. YouTube: finance / crypto / fitness / podcast, subscribers 10k-200k. No consistent short-form presence.</li>
      <li><strong>Generate demo clips for top 10 (2 hrs)</strong> — Use the Clip Generator tab. 3 clips, 45-90s. Create delivery, copy shareable link.</li>
      <li><strong>Outreach on all channels simultaneously</strong></li>
    </ol>
    <h3>Outreach Channels</h3>
    <p><span class="platform-badge pb-up">YTjobs.co</span> <a href="https://ytjobs.co/" target="_blank">ytjobs.co</a> — Active clipper job board. Apply to every open listing today. Pitch: "AI pipeline, any URL to clips in 10 min. Sample: [link]"</p>
    <p><span class="platform-badge pb-up">Upwork</span> <a href="https://www.upwork.com/freelance-jobs/ai-generated-video/" target="_blank">Upwork AI Video Jobs</a> — Apply to $300-$800 AI video jobs. $500 and $720 fixed-price jobs confirmed in this category.</p>
    <p><span class="platform-badge pb-dc">Discord</span> Video Editors 115K: <a href="https://discord.com/invite/videoeditors" target="_blank">discord.com/invite/videoeditors</a> — Post in #for-hire with demo link.</p>
    <p><span class="platform-badge pb-dc">Discord</span> Video Editing Hub 55K: <a href="https://discord.com/invite/video-editing-hub-732343015711965204" target="_blank">invite link</a> — Post in hire channel.</p>
    <div class="total-box" style="text-align:left;padding:12px 16px">
      <p><span class="highlight">Day 1 math:</span> 1 retainer ($497) + 1 Upwork/YTjobs project ($500) = ~$1,000</p>
      <p style="margin-top:4px"><span class="warning">Fallback:</span> 3 Discord orders x $97 demo pack + 1 retainer = $788</p>
    </div>
  </div>

  <div class="card">
    <h2>Day 2 April 4 — Target: $1,000</h2>
    <span class="day-badge day2">Day 2</span>
    <h3>Revenue Stream A: Podcast-to-clips retainer</h3>
    <p>Target podcasters with 5K-50K downloads/episode who record video but don't post Shorts.</p>
    <ul>
      <li>Pricing: <span class="highlight">$150-$350/episode</span> or $497/month unlimited</li>
      <li>Find them: Twitter/X search "#podcast", <a href="https://podmatch.com/" target="_blank">Podmatch.com</a>, r/podcast [FOR HIRE]</li>
      <li>Demo move: Clip one of their episodes, send 3 clips before asking</li>
    </ul>
    <h3>Revenue Stream B: Course Creator Visual Kit</h3>
    <p>Target STEM YouTubers, online educators, corporate trainers.</p>
    <ul>
      <li>What you deliver: 5 Manim animations (30s each) + 3 Blender title cards + 5 AI thumbnails</li>
      <li>Price: <span class="highlight">$497/kit</span> (market rate individually is $150-$500 per piece)</li>
      <li>Find them: LinkedIn, r/elearning, r/OnlineCourses, Twitter/X</li>
    </ul>
    <div class="total-box" style="text-align:left;padding:12px 16px">
      <p><span class="highlight">Day 2 math:</span> 2 podcast clients x $200/episode + 1 visual kit = ~$900-$1,300</p>
    </div>
  </div>

  <div class="card">
    <h2>Day 3 April 5 — Close Pipeline + New Channels</h2>
    <span class="day-badge day3">Day 3</span>
    <h3>A: Follow up on all Day 1 DMs (48hr follow-up)</h3>
    <h3>B: Reddit posts</h3>
    <ul>
      <li>r/forhire — [FOR HIRE] with demo and pricing (budget required in post)</li>
      <li>r/NewTubers — offer free demo clips to small creators (builds testimonials)</li>
      <li>r/podcast — [FOR HIRE] targeting podcasters</li>
    </ul>
    <h3>C: Passive income setup (weeks 2-4, not for 72hr target)</h3>
    <ul>
      <li><a href="https://reach.cat/" target="_blank">Reach.cat</a> — pay-per-view clipping, $1-6 CPM, weekly payouts</li>
      <li><a href="https://whop.com/discover/explore/content-rewards/" target="_blank">Whop Content Rewards</a> — brand campaigns, $1-2 CPM</li>
      <li><a href="https://insolvo.com/audio-video/video-editing/yt-video-clipper" target="_blank">Insolvo.com</a> — marketplace listing</li>
    </ul>
  </div>

  <div class="card">
    <h2>Product Demo Video Vertical</h2>
    <p>Your Manim + Blender pipeline can produce professional product demos for businesses. High-ticket market.</p>
    <table>
      <thead><tr><th>Client Type</th><th>What You Deliver</th><th>Price Range</th></tr></thead>
      <tbody>
        <tr><td>SaaS companies</td><td>60-90s animated product walkthrough</td><td class="highlight">$2,990-$7,600</td></tr>
        <tr><td>E-commerce brands</td><td>30-60s product showcase video</td><td class="highlight">$497-$1,500</td></tr>
        <tr><td>Real estate agencies</td><td>Property tour + data viz</td><td class="highlight">$300-$2,500/property</td></tr>
        <tr><td>Corporate training</td><td>5-10 min animated course module</td><td class="highlight">$2,000-$25,000</td></tr>
        <tr><td>Marketing agencies</td><td>White-label production (resell)</td><td class="highlight">$500-$3,000/video</td></tr>
      </tbody>
    </table>
    <h3>Where to Find Business Clients</h3>
    <ul>
      <li>Prospect Finder: Business/Brand type — searches YouTube for company/brand channels</li>
      <li>LinkedIn: search "SaaS founder", "marketing director", "product manager"</li>
      <li>Upwork: search "product demo video", "explainer video", "SaaS demo"</li>
      <li>Cold email: find SaaS companies on ProductHunt, contact founders directly</li>
    </ul>
  </div>

  <div class="card">
    <h2>DM Script Templates</h2>
    <h3>Creator Clips DM</h3>
    <div class="dm-box" id="dm1">Hey [Name] — love your [specific recent video topic].

I noticed you're not turning your long-form into Shorts consistently.
I already clipped your latest video as a free demo: [delivery link]

I can deliver 30-50 clips every month — fully automated, zero extra work from you.
Most clippers offer 5-10 clips. I do 10x that at the same price.

$297/month for 30 clips or $497/month for 50 clips.
First week free. Interested?</div>
    <button class="btn-copy" onclick="navigator.clipboard.writeText(document.getElementById('dm1').textContent).then(()=>this.textContent='Copied!').then(()=>setTimeout(()=>this.textContent='Copy',1500))">Copy</button>

    <h3>Podcast DM</h3>
    <div class="dm-box" id="dm2">Hey [Name] — I listen to your show. Noticed you're not posting episode clips to Shorts.

I already pulled 3 clips from your last episode: [delivery link]

I do this automatically for every episode you publish — 5-10 clips per episode,
delivered within 24 hours of publish. $200/episode or $497/month unlimited.

Want the rest of last week's clips free?</div>
    <button class="btn-copy" onclick="navigator.clipboard.writeText(document.getElementById('dm2').textContent).then(()=>this.textContent='Copied!').then(()=>setTimeout(()=>this.textContent='Copy',1500))">Copy</button>

    <h3>Educator / Manim Visual DM</h3>
    <div class="dm-box" id="dm3">Hey [Name] — I watched your [topic] video. The concepts would hit way harder with animated visuals.

I generate Manim-style math/data animations and Blender title cards using AI.

$497 for a full kit: 5 animations + 3 title cards + 5 thumbnails, delivered in 24 hours.
Here's a sample: [portfolio link]

Worth a quick chat?</div>
    <button class="btn-copy" onclick="navigator.clipboard.writeText(document.getElementById('dm3').textContent).then(()=>this.textContent='Copied!').then(()=>setTimeout(()=>this.textContent='Copy',1500))">Copy</button>

    <h3>Product Demo (Business) DM</h3>
    <div class="dm-box" id="dm4">Hey [Name] — I noticed [Company] doesn't have a product demo video on their site.

I build AI-generated animated demos using Manim + Blender — same quality as $10k agency work.
I can deliver a 60-second walkthrough in 48 hours for $997.

Here's a sample: [portfolio link]
Interested?</div>
    <button class="btn-copy" onclick="navigator.clipboard.writeText(document.getElementById('dm4').textContent).then(()=>this.textContent='Copied!').then(()=>setTimeout(()=>this.textContent='Copy',1500))">Copy</button>

    <h3>Discord / Reddit For-Hire Post</h3>
    <div class="dm-box" id="dm5">[FOR HIRE] AI Video Clipper — 10-min turnaround, any YouTube/Twitch URL

I run an AI clipping pipeline that turns long-form content into
30-50 short clips per month. I can demo your content right now.

Intro offer: $97 for 10 clips (demo pack)
Monthly: $297-$497/month

Sample: [your portfolio test output link]
DM me with any URL and I'll clip it as a free sample.</div>
    <button class="btn-copy" onclick="navigator.clipboard.writeText(document.getElementById('dm5').textContent).then(()=>this.textContent='Copied!').then(()=>setTimeout(()=>this.textContent='Copy',1500))">Copy</button>
  </div>

  <div class="card">
    <h2>Revenue Projection</h2>
    <table>
      <thead><tr><th>Day</th><th>Source</th><th>Amount</th></tr></thead>
      <tbody>
        <tr><td>Day 1</td><td>1 Upwork/YTjobs project</td><td class="highlight">$500</td></tr>
        <tr><td>Day 1</td><td>1 creator retainer</td><td class="highlight">$497</td></tr>
        <tr><td>Day 2</td><td>2 podcast clients x $200</td><td class="highlight">$400-$800</td></tr>
        <tr><td>Day 2</td><td>1 course creator visual kit</td><td class="highlight">$497</td></tr>
        <tr><td>Day 3</td><td>Follow-up conversions</td><td class="highlight">$300-$600</td></tr>
        <tr style="font-weight:600"><td colspan="2">Total</td><td class="highlight">$2,200-$2,900</td></tr>
      </tbody>
    </table>
    <p><span class="warning">To hit $3,000:</span> One larger close — a brand, agency, or podcast production company at $500-$1,000.</p>
    <h3>Long-term Monthly Recurring</h3>
    <ul>
      <li>50 creators x $297/month = <span class="highlight">$14,850/month</span></li>
      <li>20 podcast clients x $200/episode x 4 episodes = <span class="highlight">$16,000/month</span></li>
      <li>5 business clients x $997/month = <span class="highlight">$4,985/month</span></li>
    </ul>
  </div>

  <div class="card">
    <h2>New High-Value Strategies (Added Apr 5)</h2>
    <table>
      <thead><tr><th>Strategy</th><th>Pricing</th><th>Speed to Cash</th><th>Notes</th></tr></thead>
      <tbody>
        <tr>
          <td><strong>White-label agency model</strong></td>
          <td class="highlight">$500-1,000/month per partner</td>
          <td>1-2 weeks</td>
          <td>Partner with existing video agencies who have clients — they resell your service at a markup. FASTEST path to recurring revenue without direct outreach.</td>
        </tr>
        <tr>
          <td><strong>PeoplePerHour gigs</strong></td>
          <td class="highlight">£50-150/gig</td>
          <td>72 hours</td>
          <td>Less saturated than Fiverr, 0% commission on first £500. List 3-5 fixed-price packages. Gig approval is fast (24-48hrs). URL: <a href="https://www.peopleperhour.com/category/video-animation/video-editing" target="_blank">PeoplePerHour</a></td>
        </tr>
        <tr>
          <td><strong>AI Thumbnail upsell</strong></td>
          <td class="highlight">$50-150/set (5 thumbnails)</td>
          <td>Same day</td>
          <td>Upsell to ANY existing clip client. Platform already generates these. Near-zero marginal cost. Add to every retainer pitch.</td>
        </tr>
        <tr>
          <td><strong>Sports highlights</strong></td>
          <td class="highlight">$400-1,099/package</td>
          <td>1-2 weeks</td>
          <td>Twitch eSports streamers, college sports coaches, local teams. Use Prospect Finder (Twitch, sports niche). Sample: Clip 3 moments from a recent match before the first DM.</td>
        </tr>
        <tr>
          <td><strong>Real estate listing videos</strong></td>
          <td class="highlight">$25-50/property</td>
          <td>48 hours</td>
          <td>Real estate agents need quick listing videos — high volume, low touch. Target agents with 10+ active listings on Zillow/Realtor. $25 entry price gets the relationship; upsell to monthly package.</td>
        </tr>
      </tbody>
    </table>

    <h3>White-label Agency Pitch</h3>
    <div class="dm-box" id="dm6">Hey [Agency Name] — I run an AI video clipping pipeline that turns any YouTube/Twitch URL into 30-50 clips in under 10 minutes.

I'm looking to white-label this for agencies like yours. You bring the clients, I handle production invisibly.

Your margin: charge clients $500-1,500/month, pay me $297-497/month. Pure profit on volume.

No contracts. Cancel anytime. Want a free 10-clip demo on one of your client's channels?</div>
    <button class="btn-copy" onclick="navigator.clipboard.writeText(document.getElementById('dm6').textContent).then(()=>this.textContent='Copied!').then(()=>setTimeout(()=>this.textContent='Copy',1500))">Copy</button>

    <h3>Sports Highlights DM</h3>
    <div class="dm-box" id="dm7">Hey [Name] — caught your [game/stream] on Twitch. Those [specific moments] would go crazy on TikTok/Shorts.

I clipped 3 highlights from your last session: [delivery link]

I can do this for every stream automatically — 10-20 clips per session, same-day delivery.
$400/month for weekly streams or $99 per one-off session.

Interested?</div>
    <button class="btn-copy" onclick="navigator.clipboard.writeText(document.getElementById('dm7').textContent).then(()=>this.textContent='Copied!').then(()=>setTimeout(()=>this.textContent='Copy',1500))">Copy</button>
  </div>

  <div class="card">
    <h2>Quick Actions</h2>
    <div style="display:flex;flex-wrap:wrap;gap:10px">
      <a href="/admin/prospect-finder" style="padding:10px 18px;background:#5c5470;color:#fff;border-radius:8px;text-decoration:none;font-size:0.9rem">Prospect Finder</a>
      <a href="/manual-clipping" style="padding:10px 18px;background:#0f3460;color:#dbd8e3;border:1px solid #1e3a5f;border-radius:8px;text-decoration:none;font-size:0.9rem">Manual Clipping</a>
      <a href="/admin/deliveries" style="padding:10px 18px;background:#0f3460;color:#dbd8e3;border:1px solid #1e3a5f;border-radius:8px;text-decoration:none;font-size:0.9rem">Deliveries</a>
      <a href="/admin/test-runs" style="padding:10px 18px;background:#0f3460;color:#dbd8e3;border:1px solid #1e3a5f;border-radius:8px;text-decoration:none;font-size:0.9rem">Portfolio Tests</a>
      <a href="https://ytjobs.co/" target="_blank" style="padding:10px 18px;background:#065f46;color:#6ee7b7;border-radius:8px;text-decoration:none;font-size:0.9rem">YTjobs.co</a>
      <a href="https://www.upwork.com/freelance-jobs/ai-generated-video/" target="_blank" style="padding:10px 18px;background:#1e3a5f;color:#93c5fd;border-radius:8px;text-decoration:none;font-size:0.9rem">Upwork AI Video</a>
      <a href="https://www.peopleperhour.com/category/video-animation/video-editing" target="_blank" style="padding:10px 18px;background:#1e3a5f;color:#93c5fd;border-radius:8px;text-decoration:none;font-size:0.9rem">PeoplePerHour</a>
    </div>
  </div>

</div>
</body>
</html>"###;

// ============================================================================
// Revenue Ledger — per-whitelisted-user attribution for revenue sharing
// ============================================================================
//
// Aggregates the full funnel per user:
//   leads sourced → leads contacted → leads converted → deliveries created
//   → USDC paid on HD unlocks → 50% payout owed.
//
// The user-visible SSR page just lists the aggregates; the admin can
// settle payouts off-ledger (crypto transfer on Base) using this view.

pub async fn api_revenue_ledger(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Per-user rollup. Nested CTEs would be cleaner with proper aggregation
    // but this keeps the SQL readable. Payout split is 50% — surfaced as a
    // decimal so the admin can override per-user later.
    let rows = sqlx::query(
        r#"
        SELECT
            u.id                AS user_id,
            u.username,
            u.email,
            -- Lead funnel counts
            COALESCE(lead_counts.total_leads, 0)      AS leads_sourced,
            COALESCE(lead_counts.contacted_leads, 0)  AS leads_contacted,
            COALESCE(lead_counts.converted_leads, 0)  AS leads_converted,
            -- Delivery + payment rollup
            COALESCE(delivery_stats.deliveries_created, 0)       AS deliveries_created,
            COALESCE(delivery_stats.deliveries_unlocked, 0)      AS deliveries_unlocked,
            COALESCE(delivery_stats.total_usdc_paid, 0)::numeric AS total_usdc_paid,
            (COALESCE(delivery_stats.total_usdc_paid, 0) * 0.5)::numeric AS pending_payout_usdc
        FROM users u
        LEFT JOIN (
            SELECT user_id,
                   COUNT(*)                                                  AS total_leads,
                   COUNT(*) FILTER (WHERE first_contacted_at IS NOT NULL)    AS contacted_leads,
                   COUNT(*) FILTER (WHERE converted_at IS NOT NULL)          AS converted_leads
            FROM instagram_leads
            WHERE user_id IS NOT NULL
            GROUP BY user_id
        ) lead_counts ON lead_counts.user_id = u.id
        LEFT JOIN (
            SELECT il.user_id,
                   COUNT(d.id)                                               AS deliveries_created,
                   COUNT(*) FILTER (WHERE d.unlocked_until IS NOT NULL)      AS deliveries_unlocked,
                   COALESCE(SUM(d.unlock_price_usdc) FILTER (WHERE d.unlocked_until IS NOT NULL), 0)
                                                                             AS total_usdc_paid
            FROM deliveries d
            JOIN instagram_leads il ON il.id = d.sourced_from_lead_id
            GROUP BY il.user_id
        ) delivery_stats ON delivery_stats.user_id = u.id
        WHERE COALESCE(lead_counts.total_leads, 0) > 0
           OR COALESCE(delivery_stats.deliveries_created, 0) > 0
        ORDER BY COALESCE(delivery_stats.total_usdc_paid, 0) DESC,
                 COALESCE(lead_counts.total_leads, 0) DESC
        "#,
    )
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| {
        tracing::error!("revenue_ledger query failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let items: Vec<serde_json::Value> = rows.iter().map(|r| {
        let dec_to_f64 = |d: Option<sqlx::types::Decimal>| -> f64 {
            d.and_then(|d| d.to_string().parse::<f64>().ok()).unwrap_or(0.0)
        };
        json!({
            "user_id":             r.get::<i32, _>("user_id"),
            "username":            r.get::<String, _>("username"),
            "email":               r.get::<String, _>("email"),
            "leads_sourced":       r.get::<i64, _>("leads_sourced"),
            "leads_contacted":     r.get::<i64, _>("leads_contacted"),
            "leads_converted":     r.get::<i64, _>("leads_converted"),
            "deliveries_created":  r.get::<i64, _>("deliveries_created"),
            "deliveries_unlocked": r.get::<i64, _>("deliveries_unlocked"),
            "total_usdc_paid":     dec_to_f64(r.try_get("total_usdc_paid").ok().flatten()),
            "pending_payout_usdc": dec_to_f64(r.try_get("pending_payout_usdc").ok().flatten()),
        })
    }).collect();

    Ok(Json(json!({"success": true, "ledger": items, "count": items.len()})))
}

pub async fn admin_revenue_ledger_page() -> Html<&'static str> {
    Html(REVENUE_LEDGER_HTML)
}

const REVENUE_LEDGER_HTML: &str = r###"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Revenue Ledger — VideoSync Admin</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700;800&display=swap" rel="stylesheet">
<style>
  :root{--bg:#2a2438;--surface:#352f44;--border:#5c5470;--accent:#dbd8e3;--purple:#7a4cff;--muted:#b8b3c8;--dim:#9999bb;--success:#4ade80;--warn:#facc15;--danger:#f87171}
  *{box-sizing:border-box;margin:0;padding:0}
  body{font-family:'Inter',system-ui,sans-serif;background:linear-gradient(135deg,#2a2438 0%,#1f1a2a 100%);color:var(--accent);min-height:100vh}
  .header{background:rgba(53,47,68,0.6);backdrop-filter:blur(18px);border-bottom:1px solid rgba(92,84,112,0.4);padding:16px 28px;display:flex;align-items:center;gap:16px;position:sticky;top:0;z-index:10}
  .header h1{font-size:1.2rem;font-weight:600;background:linear-gradient(135deg,var(--accent) 0%,#fff 100%);-webkit-background-clip:text;background-clip:text;-webkit-text-fill-color:transparent}
  .back{color:var(--dim);text-decoration:none;font-size:0.9rem}
  .back:hover{color:var(--accent)}
  .container{max-width:1320px;margin:0 auto;padding:28px}
  .intro{background:rgba(53,47,68,0.7);border:1px solid rgba(92,84,112,0.4);border-radius:14px;padding:22px;margin-bottom:22px}
  .intro h2{font-size:1rem;color:var(--accent);margin-bottom:8px}
  .intro p{font-size:0.9rem;color:var(--muted);line-height:1.6}
  .intro code{background:rgba(42,36,56,0.6);padding:2px 8px;border-radius:4px;font-size:0.85em;color:var(--accent)}
  .table-wrap{background:rgba(53,47,68,0.7);border:1px solid rgba(92,84,112,0.5);border-radius:14px;overflow:auto;box-shadow:0 4px 24px rgba(0,0,0,0.25)}
  table{width:100%;border-collapse:collapse;font-size:0.88rem;min-width:900px}
  th{text-align:left;padding:14px 16px;color:var(--dim);font-weight:500;font-size:0.72rem;text-transform:uppercase;letter-spacing:0.05em;border-bottom:1px solid rgba(92,84,112,0.4);background:rgba(42,36,56,0.4)}
  th.numeric,td.numeric{text-align:right}
  td{padding:14px 16px;border-bottom:1px solid rgba(92,84,112,0.2);color:var(--accent);vertical-align:middle}
  tr:hover td{background:rgba(92,84,112,0.12)}
  .user-cell strong{color:#fff;font-weight:600}
  .user-cell small{display:block;color:var(--dim);font-size:0.78rem;margin-top:2px}
  .num{font-variant-numeric:tabular-nums;font-weight:500}
  .num.big{font-weight:700;color:#fff;font-size:1rem}
  .num.money{color:var(--success)}
  .num.pending{color:var(--warn)}
  .empty{padding:60px 20px;text-align:center;color:var(--dim)}
  .empty-icon{font-size:3rem;margin-bottom:12px;opacity:0.5}
  .loading{padding:40px;text-align:center;color:var(--dim)}
  .totals{display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:12px;margin-bottom:22px}
  .stat-card{background:rgba(53,47,68,0.6);border:1px solid rgba(92,84,112,0.3);border-radius:12px;padding:14px 16px}
  .stat-val{font-size:1.4rem;font-weight:800;color:#fff}
  .stat-val.money{color:var(--success)}
  .stat-val.pending{color:var(--warn)}
  .stat-label{font-size:0.72rem;text-transform:uppercase;color:var(--dim);letter-spacing:0.05em;margin-top:2px}
</style>
</head>
<body>
<div class="header">
  <a href="/admin/dashboard" class="back">← Admin</a>
  <h1>💰 Revenue Ledger</h1>
</div>
<div class="container">
  <div class="intro">
    <h2>Per-user lead attribution + revenue share</h2>
    <p>Each row aggregates the full lead funnel for a whitelisted team member: leads they sourced on Instagram → leads they DMed → leads that converted to paid clients → deliveries created → USDC paid on HD unlocks. <strong>Pending payout</strong> shows the 50% cut owed — settle by sending USDC on Base to the user's wallet (you collect email/wallet off-ledger for now). A user only appears here when they've either sourced a lead or had a delivery created from their lead.</p>
  </div>
  <div id="totals" class="totals"></div>
  <div class="table-wrap">
    <div id="ledger-content"><div class="loading">Loading ledger…</div></div>
  </div>
</div>
<script>
const token = localStorage.getItem('auth_token') || localStorage.getItem('admin_token') || localStorage.getItem('authToken');
if (!token) window.location.href = '/admin';

async function loadLedger() {
  const res = await fetch('/api/admin/revenue-ledger', {headers:{'Authorization':'Bearer '+token}});
  const data = await res.json();
  const root = document.getElementById('ledger-content');
  if (!data.success) { root.innerHTML = '<div class="empty">Failed to load ledger</div>'; return; }
  const rows = data.ledger || [];
  if (rows.length === 0) {
    root.innerHTML = '<div class="empty"><div class="empty-icon">🪙</div>No revenue attribution yet.<div style="font-size:0.85rem;margin-top:8px;color:var(--dim)">As whitelisted users source IG leads and those leads pay for HD unlocks, rows will appear here.</div></div>';
    document.getElementById('totals').innerHTML = '';
    return;
  }
  const totals = rows.reduce((acc, r) => ({
    leads:    acc.leads    + (r.leads_sourced || 0),
    convs:    acc.convs    + (r.leads_converted || 0),
    unlocks:  acc.unlocks  + (r.deliveries_unlocked || 0),
    paid:     acc.paid     + (r.total_usdc_paid || 0),
    pending:  acc.pending  + (r.pending_payout_usdc || 0),
  }), {leads:0, convs:0, unlocks:0, paid:0, pending:0});
  document.getElementById('totals').innerHTML = `
    <div class="stat-card"><div class="stat-val">${totals.leads}</div><div class="stat-label">Total leads sourced</div></div>
    <div class="stat-card"><div class="stat-val">${totals.convs}</div><div class="stat-label">Conversions</div></div>
    <div class="stat-card"><div class="stat-val">${totals.unlocks}</div><div class="stat-label">HD unlocks paid</div></div>
    <div class="stat-card"><div class="stat-val money">$${totals.paid.toFixed(2)}</div><div class="stat-label">Gross USDC</div></div>
    <div class="stat-card"><div class="stat-val pending">$${totals.pending.toFixed(2)}</div><div class="stat-label">Pending payout (50%)</div></div>
  `;
  root.innerHTML = '<table><thead><tr><th>User</th><th class="numeric">Leads sourced</th><th class="numeric">Contacted</th><th class="numeric">Converted</th><th class="numeric">Deliveries created</th><th class="numeric">HD unlocks paid</th><th class="numeric">Gross USDC</th><th class="numeric">Payout owed (50%)</th></tr></thead><tbody>' +
    rows.map(r => `
      <tr>
        <td class="user-cell"><strong>@${r.username}</strong><small>${r.email}</small></td>
        <td class="numeric num big">${r.leads_sourced}</td>
        <td class="numeric num">${r.leads_contacted}</td>
        <td class="numeric num">${r.leads_converted}</td>
        <td class="numeric num">${r.deliveries_created}</td>
        <td class="numeric num">${r.deliveries_unlocked}</td>
        <td class="numeric num money">$${(r.total_usdc_paid || 0).toFixed(2)}</td>
        <td class="numeric num pending">$${(r.pending_payout_usdc || 0).toFixed(2)}</td>
      </tr>
    `).join('') +
    '</tbody></table>';
}
loadLedger();
</script>
</body>
</html>"###;

// ============================================================================
// Admin "How We Work" guide — the operating manual for the founder
// ============================================================================
//
// Covers every lead source, every service type, payment mechanics,
// attribution, QA review, and the daily workflow. Complements the
// whitelisted-team How It Works tab that lives inside content_machine.

pub async fn admin_how_it_works_page() -> Html<&'static str> {
    Html(HOW_WE_WORK_HTML)
}

const HOW_WE_WORK_HTML: &str = r###"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>How We Work — VideoSync Admin</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700;800&display=swap" rel="stylesheet">
<style>
  :root{--bg:#2a2438;--surface:#352f44;--border:#5c5470;--accent:#dbd8e3;--purple:#7a4cff;--muted:#b8b3c8;--dim:#9999bb;--success:#4ade80;--warn:#facc15;--danger:#f87171}
  *{box-sizing:border-box;margin:0;padding:0}
  body{font-family:'Inter',system-ui,sans-serif;background:linear-gradient(135deg,#2a2438 0%,#1f1a2a 100%);color:var(--accent);min-height:100vh;line-height:1.65}
  .header{background:rgba(53,47,68,0.6);backdrop-filter:blur(18px);border-bottom:1px solid rgba(92,84,112,0.4);padding:16px 28px;display:flex;align-items:center;gap:16px;position:sticky;top:0;z-index:10}
  .header h1{font-size:1.2rem;font-weight:600;background:linear-gradient(135deg,var(--accent) 0%,#fff 100%);-webkit-background-clip:text;background-clip:text;-webkit-text-fill-color:transparent}
  .back{color:var(--dim);text-decoration:none;font-size:0.9rem}
  .back:hover{color:var(--accent)}
  .container{max-width:960px;margin:0 auto;padding:32px 28px}
  .intro{background:linear-gradient(135deg,rgba(122,76,255,0.12),rgba(53,47,68,0.6));border:1px solid rgba(122,76,255,0.3);border-radius:14px;padding:22px;margin-bottom:28px}
  .intro h2{font-size:1.1rem;color:#fff;margin-bottom:8px}
  .intro p{font-size:0.9rem;color:var(--muted);line-height:1.7}
  section{background:rgba(53,47,68,0.7);border:1px solid rgba(92,84,112,0.4);border-radius:14px;padding:24px;margin-bottom:20px}
  section h2{font-size:1.05rem;font-weight:700;color:#fff;margin-bottom:12px;display:flex;align-items:center;gap:8px}
  section h3{font-size:0.95rem;font-weight:600;color:var(--accent);margin-top:18px;margin-bottom:10px}
  section p, section li{color:var(--muted);font-size:0.9rem;line-height:1.7}
  section code{background:rgba(42,36,56,0.8);padding:2px 7px;border-radius:4px;font-size:0.85em;color:var(--accent);border:1px solid rgba(92,84,112,0.3)}
  section a{color:var(--purple);text-decoration:none}
  section a:hover{text-decoration:underline}
  section ol,section ul{padding-left:24px;margin:10px 0}
  section li{margin-bottom:8px}
  section li strong{color:var(--accent)}
  .source-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));gap:14px;margin-top:14px}
  .source-card{background:rgba(42,36,56,0.5);border:1px solid rgba(92,84,112,0.3);border-radius:10px;padding:16px}
  .source-card h4{color:#fff;font-size:0.95rem;font-weight:700;margin-bottom:6px}
  .source-card .who{font-size:0.72rem;text-transform:uppercase;letter-spacing:0.08em;color:var(--purple);font-weight:600;margin-bottom:8px}
  .source-card p{font-size:0.82rem;line-height:1.55}
  .service-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(240px,1fr));gap:12px;margin-top:12px}
  .service-card{background:rgba(42,36,56,0.5);border:1px solid rgba(92,84,112,0.3);border-radius:10px;padding:14px}
  .service-card .name{font-weight:700;color:#fff;margin-bottom:4px;font-size:0.92rem}
  .service-card .price{color:var(--success);font-size:0.8rem;font-weight:600;margin-bottom:6px;display:block}
  .service-card .desc{font-size:0.82rem;line-height:1.5;color:var(--muted)}
  .step{display:flex;gap:14px;margin-bottom:16px;align-items:flex-start}
  .step-num{flex-shrink:0;width:28px;height:28px;border-radius:50%;background:var(--purple);color:#fff;font-weight:700;font-size:0.85rem;display:flex;align-items:center;justify-content:center}
  .step-body{flex:1}
  .step-body strong{color:var(--accent);display:block;margin-bottom:3px}
  .callout{background:rgba(250,204,21,0.08);border:1px solid rgba(250,204,21,0.3);border-radius:8px;padding:12px 14px;margin:14px 0;font-size:0.85rem;color:var(--warn)}
  .callout.ok{background:rgba(74,222,128,0.08);border-color:rgba(74,222,128,0.3);color:var(--success)}
  .toc{background:rgba(53,47,68,0.5);border:1px solid rgba(92,84,112,0.3);border-radius:10px;padding:16px 20px;margin-bottom:24px}
  .toc h3{color:var(--accent);font-size:0.85rem;text-transform:uppercase;letter-spacing:0.08em;margin-bottom:10px;font-weight:600}
  .toc ol{padding-left:20px}
  .toc li{margin-bottom:4px;font-size:0.87rem}
  .toc a{color:var(--muted)}
  .toc a:hover{color:var(--accent)}
</style>
</head>
<body>
<div class="header">
  <a href="/admin/dashboard" class="back">← Admin</a>
  <h1>📘 How We Work — The Operating Manual</h1>
</div>
<div class="container">

<div class="intro">
  <h2>The platform in one paragraph</h2>
  <p>VideoSync is an AI video production studio monetised 4 ways: (1) <b>service sales</b> to creators / founders you source via Instagram, LinkedIn, Telegram, YouTube, and Twitch; (2) <b>per-delivery x402 USDC unlocks</b> on <code>/delivery/:id</code> preview pages; (3) <b>monthly subscriptions</b> — $15/mo creator tier + $99/$199 agency tier; (4) <b>clipping-payout platforms</b> (Whop, Reach.cat, etc.) where we post authorised clips and get paid per 1,000 views. Everything flows to USDC on Base — no Stripe, no contracts.</p>
</div>

<div class="toc">
  <h3>In this guide</h3>
  <ol>
    <li><a href="#lead-sources">The 5 lead sources + who runs them</a></li>
    <li><a href="#services">The 7 services + pricing</a></li>
    <li><a href="#daily">Your daily workflow (hour-by-hour)</a></li>
    <li><a href="#team">How the whitelisted clipping team works</a></li>
    <li><a href="#payments">Payment rails + attribution + revenue share</a></li>
    <li><a href="#qa">LLM QA review on renders</a></li>
    <li><a href="#paywall">The $15/mo creator paywall</a></li>
    <li><a href="#faq">FAQ</a></li>
  </ol>
</div>

<!-- ═══════ LEAD SOURCES ═══════ -->
<section id="lead-sources">
  <h2>1. The 5 lead sources — where leads come from + who runs each</h2>
  <p>Each source surfaces prospects you can pitch. Some run automatically, some need your attention daily.</p>
  <div class="source-grid">
    <div class="source-card">
      <div class="who">You only</div>
      <h4>🎯 YouTube / Twitch</h4>
      <p>AI agent searches for creators by niche + audience size + content type. Runs when you fire it from <a href="/admin/prospect-finder">prospect-finder → YouTube/Twitch</a>. Leads land in the <code>prospects</code> table.</p>
    </div>
    <div class="source-card">
      <div class="who">You only</div>
      <h4>💼 LinkedIn</h4>
      <p>Plain-English description → AI builds filters → PhantomBuster scrapes Sales Navigator OR regular LinkedIn Search depending on which phantom you have installed. Results import via <a href="/admin/prospect-finder">prospect-finder → LinkedIn tab</a>.</p>
    </div>
    <div class="source-card">
      <div class="who">Whitelisted team (primary) + you</div>
      <h4>📸 Instagram</h4>
      <p>PhantomBuster hashtag search + per-user scoping. Team uses <a href="https://cmachine.devthuku.io/instagram-leads">cmachine → Instagram Leads</a>. You can also run it from the admin prospect-finder. Attribution per whitelisted user flows to the <a href="/admin/revenue-ledger">Revenue Ledger</a>.</p>
    </div>
    <div class="source-card">
      <div class="who">You only (automated)</div>
      <h4>✈️ Telegram (MTProto watcher)</h4>
      <p>Once you finish phone-code login on the Telegram tab, the userbot polls configured channels (cryptojobslist, web3_jobs, etc.) for "need editor / hiring / paying" posts, scores them with AI, and pings <code>@videosync_sales_bot</code> with a pre-written custom DM. You copy + send.</p>
    </div>
    <div class="source-card">
      <div class="who">You only (passive income)</div>
      <h4>💰 Whop / Reach.cat / content-reward platforms</h4>
      <p>Not outreach — you post authorised clips from creator campaigns to TikTok/Reels/Shorts, get paid per 1,000 views. Highest-CPM niches: crypto/finance ($4-6/1k), creator campaigns ($1-3/1k). <a href="/admin/monetization-guide">Monetization Guide</a> has the current-picks list.</p>
    </div>
  </div>
</section>

<!-- ═══════ SERVICES ═══════ -->
<section id="services">
  <h2>2. The 7 services — what we sell + what we charge</h2>
  <p>Every scored lead gets a service pick from the AI. You (or team members) can override it via the dropdown in the DM dialog.</p>
  <div class="service-grid">
    <div class="service-card">
      <div class="name">🎬 Clipping</div>
      <span class="price">$297 – $899/mo</span>
      <p class="desc">Long-form videos → 20–40 vertical Shorts/Reels/TikToks per month. Team-only service (whitelisted team fulfills).</p>
    </div>
    <div class="service-card">
      <div class="name">🎞️ Animations</div>
      <span class="price">$50 – $150 each</span>
      <p class="desc">Blender explainer scenes, data visualisations, LaTeX equations, lower-thirds, title cards. Best for educators + crypto / finance.</p>
    </div>
    <div class="service-card">
      <div class="name">🖼️ AI Thumbnails</div>
      <span class="price">$25 – $50 each</span>
      <p class="desc">Gemini picks best frame → title overlay → CTR-tested. Best for growing YouTubers.</p>
    </div>
    <div class="service-card">
      <div class="name">📱 UGC / Product-demo Videos</div>
      <span class="price">$200 – $500 each</span>
      <p class="desc">Vertical-first ad-style demo videos for Shopify / DTC / SaaS founders.</p>
    </div>
    <div class="service-card">
      <div class="name">📦 Product Mockups</div>
      <span class="price">$100 – $300 each</span>
      <p class="desc">3D Blender product renders on device or lifestyle scenes. Gemini synthesises the product shot.</p>
    </div>
    <div class="service-card">
      <div class="name">🚀 Landing-Page Animations</div>
      <span class="price">$200 – $600 each</span>
      <p class="desc">Animated SaaS hero mockups. Paste the client's live URL — we scrape their hero image and animate it. Great for Product Hunt launches.</p>
    </div>
    <div class="service-card">
      <div class="name">⭐ Full Stack</div>
      <span class="price">$1,500 – $3,000/mo</span>
      <p class="desc">Bundle of all of the above. Best for 100k+ creators + established brands.</p>
    </div>
  </div>
</section>

<!-- ═══════ DAILY WORKFLOW ═══════ -->
<section id="daily">
  <h2>3. Your daily workflow</h2>
  <p>You don't need to run every step every day — automation handles the watching. Your high-leverage time is DMing leads + closing + delivering.</p>

  <h3>Morning (~30 min)</h3>
  <div class="step">
    <div class="step-num">1</div>
    <div class="step-body">
      <strong>Check Telegram notifications from <code>@videosync_sales_bot</code></strong>
      Overnight payment pings + new opportunities from the MTProto watcher. Each opportunity already has a pre-written custom DM ready to copy.
    </div>
  </div>
  <div class="step">
    <div class="step-num">2</div>
    <div class="step-body">
      <strong>Open <a href="/admin/dashboard">/admin/dashboard</a></strong>
      Scan Recent Users (new signups overnight, trials about to expire), check health stats.
    </div>
  </div>
  <div class="step">
    <div class="step-num">3</div>
    <div class="step-body">
      <strong>Open <a href="/admin/prospect-finder">prospect-finder → Telegram tab</a></strong>
      Review AI-scored opportunities. Click <em>Open in Telegram</em> → paste the pre-written DM → mark as <em>Contacted</em>.
    </div>
  </div>

  <h3>Mid-morning (~2h) — revenue work</h3>
  <div class="step">
    <div class="step-num">4</div>
    <div class="step-body">
      <strong>Run 2–3 LinkedIn smart-searches</strong>
      Focus on niches you can close fast. Describe your ideal client → AI builds filters → PB scrapes → leads import with AI scoring.
    </div>
  </div>
  <div class="step">
    <div class="step-num">5</div>
    <div class="step-body">
      <strong>Post Whop / Reach.cat clips</strong>
      Take yesterday's authorised source videos → auto-clipper produces ~20 Shorts per source → post to burner TikTok / Reels / Shorts accounts with tracking links.
    </div>
  </div>

  <h3>Afternoon (~3h) — fulfilment</h3>
  <div class="step">
    <div class="step-num">6</div>
    <div class="step-body">
      <strong>DM top-scored IG / LinkedIn leads</strong>
      Click each lead → generate DM → attach sample (Blender render or URL-driven landing-page mockup) → the DM body automatically includes the <code>/delivery/:id</code> link. One-tap "Copy &amp; Open Instagram" handles the rest.
    </div>
  </div>
  <div class="step">
    <div class="step-num">7</div>
    <div class="step-body">
      <strong>Close via DM or delivery unlock</strong>
      Big deals: invoice direct in USDC (your Base address). Small one-offs: client pays $5 to unlock HD on the delivery page. Both attribute correctly on the Revenue Ledger.
    </div>
  </div>

  <h3>Evening (~30 min) — triage</h3>
  <div class="step">
    <div class="step-num">8</div>
    <div class="step-body">
      <strong>Mark leads: replied / won / lost</strong>
      Keeps the funnel honest + updates the revenue ledger's conversion numbers.
    </div>
  </div>
  <div class="step">
    <div class="step-num">9</div>
    <div class="step-body">
      <strong>Reply to any DMs @videosync_sales_bot flagged for human review</strong>
      The bot auto-answers simple questions; it forwards complex ones to you.
    </div>
  </div>
</section>

<!-- ═══════ TEAM ═══════ -->
<section id="team">
  <h2>4. How the whitelisted clipping team works</h2>
  <p>Your 15-person team operates through <a href="https://cmachine.devthuku.io">content_machine</a>. They have their own scoped view — each user only sees the leads they sourced.</p>
  <ol>
    <li>Log in → <strong>Instagram Leads</strong> page.</li>
    <li>Run Auto-Discover (AI picks hashtags for their niche) OR Manual Search.</li>
    <li>Leads come in scored with an AI-picked service type. Team pitches <strong>mostly clipping</strong> but can sell any service — dropdown in the DM dialog overrides the pick.</li>
    <li>Click lead → Attach sample → Copy DM &amp; Open Instagram → paste, send.</li>
    <li>Status moves: <em>new</em> → <em>contacted</em> → <em>replied</em> → <em>converted</em>.</li>
    <li>When converted, fulfilment happens inside VideoSync's main app.</li>
  </ol>
  <div class="callout ok">Team members pay NO subscription — they're grandfathered. Their work generates revenue → admin splits 50% on delivery unlocks via the <a href="/admin/revenue-ledger">Revenue Ledger</a>.</div>
  <p>Team members can see the <strong>How It Works</strong> tab inside their own Instagram Leads page — covers the same workflow from their side.</p>
</section>

<!-- ═══════ PAYMENTS ═══════ -->
<section id="payments">
  <h2>5. Payment rails + attribution + revenue share</h2>

  <h3>Payment rails</h3>
  <ul>
    <li><strong>x402 USDC on Base</strong> — every /subscribe, /api-access, /delivery/:id unlock uses the same EIP-3009 transferWithAuthorization flow. Wallet signs once, Coinbase facilitator settles on-chain in ~1s.</li>
    <li><strong>Direct wallet-to-wallet USDC</strong> — for big retainer deals closed via DM. Give the client your Base address, they send USDC, you invoice outside the app.</li>
  </ul>

  <h3>Attribution (who sourced who)</h3>
  <ul>
    <li>Every Instagram lead has a <code>user_id</code> — the whitelisted user who sourced it. Leads are private to that user.</li>
    <li>When a team member creates a sample via "+ Attach sample", the generated delivery row gets <code>sourced_from_lead_id</code> + <code>unlock_price_usdc = 5</code>.</li>
    <li>When a client pays $5 to unlock that delivery, x402 settles on your wallet — but the <a href="/admin/revenue-ledger">Revenue Ledger</a> records which team member's lead drove it.</li>
  </ul>

  <h3>Revenue share</h3>
  <ul>
    <li>Default split: <strong>50% to the whitelisted user who sourced the lead, 50% to you</strong>.</li>
    <li>Payout schedule: weekly or monthly, on-chain USDC to each team member's Base address.</li>
    <li>See per-user owed amount on the <a href="/admin/revenue-ledger">Revenue Ledger</a>.</li>
  </ul>
</section>

<!-- ═══════ QA ═══════ -->
<section id="qa">
  <h2>6. LLM QA review on every render</h2>
  <p>Before any rendered output (Blender thumbnail, animation, landing-page scene, or <code>auto_generate_video</code> pipeline) is returned to a client, Gemini reviews it against the original prompt.</p>
  <ul>
    <li>Review scores <strong>1–10</strong>. ≥6 = ships cleanly. ≤5 = flags a QA warning on the delivery row.</li>
    <li>Low-score renders STILL return the URL (fail-open) — they just get stamped with a warning in <code>deliveries.error_message</code> so you can spot them in <a href="/admin/deliveries">/admin/deliveries</a>.</li>
    <li>Every review is logged to <code>blender_render_reviews</code>. Query it if you want to see pass/fail rate per tool over time.</li>
  </ul>
</section>

<!-- ═══════ PAYWALL ═══════ -->
<section id="paywall">
  <h2>7. The $15/mo creator paywall</h2>
  <p>New signups (from Twitter ads or direct) get a <strong>7-day free trial</strong>. After that, paywalled endpoints return HTTP 402 with <code>upgrade_url: /subscribe</code> until they pay.</p>

  <h3>What's paywalled (for regular users)</h3>
  <ul>
    <li>AI thumbnails (<code>/api/tools/*</code>)</li>
    <li>Blender animations (title cards, data viz, lower thirds, LaTeX, UI mockups)</li>
    <li>Full <code>auto_generate_video</code> pipeline (<code>/ws</code> WebSocket chat)</li>
    <li>FFmpeg tool API</li>
    <li>Delivery page viewing + generation</li>
  </ul>

  <h3>What's NOT paywalled</h3>
  <ul>
    <li><strong>Clipping + manual clipping</strong> — whitelist-only, team fulfills.</li>
    <li>Signup, login, basic chat without video generation.</li>
    <li>Prospect finder + Instagram leads — these are admin/team-only tools; regular users don't see them.</li>
  </ul>

  <div class="callout">Existing users (signed up before April 16, 2026) are <code>subscription_status = 'grandfathered'</code> — they stay free forever. Only NEW signups hit the trial + paywall flow.</div>
</section>

<!-- ═══════ FAQ ═══════ -->
<section id="faq">
  <h2>8. FAQ</h2>
  <h3>What if Gemini is down — does the platform stop working?</h3>
  <p>No. LLM calls are fail-open: scoring returns a neutral score, QA review returns "pass with score 0", DM generation returns a default template. The pipeline never blocks on LLM infra.</p>

  <h3>What if Telegram revokes my MTProto session?</h3>
  <p>The watcher logs the error to <code>telegram_sessions.last_error</code> and stops. Visit <a href="/admin/prospect-finder">prospect-finder → Telegram tab</a> → status panel shows the error → re-run phone-code login.</p>

  <h3>How do I add new watched Telegram channels?</h3>
  <p>POST to <code>/api/admin/telegram/channels</code> or edit the <code>telegram_watch_channels</code> table directly. Default list: cryptojobslist, cryptojobs, web3_jobs, SaaSFounders, directoryofmarketers, contentcreators_hub.</p>

  <h3>How do I price custom deliveries?</h3>
  <p>On <a href="/admin/deliveries">/admin/deliveries</a> set <code>unlock_price_usdc</code> per delivery. Default is $5. For bigger work (e.g. $200 product mockup) set it to 200.</p>

  <h3>A team member sourced a lead that paid $500. What do I owe them?</h3>
  <p>$250 (50%). The <a href="/admin/revenue-ledger">Revenue Ledger</a> shows this as "Pending payout".</p>

  <h3>Where do I see payment notifications?</h3>
  <p>Telegram DMs from <code>@videosync_sales_bot</code>. Every successful x402 settlement (subscribe, api-access, delivery unlock) pings you with the amount + tx hash.</p>
</section>

</div>
</body>
</html>"###;
