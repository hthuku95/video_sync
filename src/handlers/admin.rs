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
        .route("/admin/users/:id", get(admin_user_detail));
    
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

// Placeholder implementations for other admin functions
pub async fn admin_users_list() -> Html<String> {
    Html("<h1>Users Management - Coming Soon</h1>".to_string())
}

pub async fn admin_user_detail(Path(_id): Path<i32>) -> Html<String> {
    Html("<h1>User Detail - Coming Soon</h1>".to_string())
}

pub async fn admin_create_user() -> Result<(), StatusCode> {
    Err(StatusCode::NOT_IMPLEMENTED)
}

pub async fn admin_update_user(Path(_id): Path<i32>) -> Result<(), StatusCode> {
    Err(StatusCode::NOT_IMPLEMENTED)
}

pub async fn admin_delete_user(Path(_id): Path<i32>) -> Result<(), StatusCode> {
    Err(StatusCode::NOT_IMPLEMENTED)
}

pub async fn admin_user_api(Path(_id): Path<i32>) -> Result<(), StatusCode> {
    Err(StatusCode::NOT_IMPLEMENTED)
}

pub async fn admin_update_user_api(Path(_id): Path<i32>) -> Result<(), StatusCode> {
    Err(StatusCode::NOT_IMPLEMENTED)
}

pub async fn admin_toggle_user_active(Path(_id): Path<i32>) -> Result<(), StatusCode> {
    Err(StatusCode::NOT_IMPLEMENTED)
}

pub async fn admin_make_staff(Path(_id): Path<i32>) -> Result<(), StatusCode> {
    Err(StatusCode::NOT_IMPLEMENTED)
}

pub async fn admin_remove_staff(Path(_id): Path<i32>) -> Result<(), StatusCode> {
    Err(StatusCode::NOT_IMPLEMENTED)
}

pub async fn admin_make_superuser(Path(_id): Path<i32>) -> Result<(), StatusCode> {
    Err(StatusCode::NOT_IMPLEMENTED)
}

pub async fn admin_remove_superuser(Path(_id): Path<i32>) -> Result<(), StatusCode> {
    Err(StatusCode::NOT_IMPLEMENTED)
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