use axum::{
    response::Html,
    routing::get,
    Router,
};

pub fn ui_routes() -> Router {
    Router::new()
        .route("/", get(landing_page))
        .route("/login", get(login_page))
        .route("/signup", get(signup_page))
        .route("/dashboard", get(dashboard_page))
        .route("/analytics", get(analytics_dashboard_page))
        .route("/help", get(help_guide_page))
        .route("/privacy", get(privacy_policy_page))
        .route("/terms", get(terms_of_service_page))
        .route("/chat", get(chat_interface))
        .route("/chat/:session_id", get(chat_interface_with_session))
        .route("/app", get(chat_interface)) // Alternative route
}

pub async fn landing_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>🎬 VideoSync - AI-Powered Video Editing</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        html {
            scroll-behavior: smooth;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            line-height: 1.6;
            color: #e8e8e8;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            background-size: cover;
            background-position: center;
            background-attachment: fixed;
            transition: background-image 1s ease-in-out;
        }

        .container {
            max-width: 1200px;
            margin: 0 auto;
            padding: 0 20px;
        }

        /* Header */
        .header {
            background: rgba(26, 26, 46, 0.9);
            backdrop-filter: blur(10px);
            border-bottom: 1px solid rgba(59, 130, 246, 0.3);
            padding: 1rem 0;
            position: fixed;
            width: 100%;
            top: 0;
            z-index: 1000;
        }

        .nav {
            display: flex;
            justify-content: space-between;
            align-items: center;
        }

        .logo {
            font-size: 1.5rem;
            font-weight: bold;
            color: white;
            text-decoration: none;
        }

        .nav-links {
            display: flex;
            gap: 2rem;
        }

        .nav-links a {
            color: white;
            text-decoration: none;
            padding: 0.5rem 1rem;
            border-radius: 20px;
            transition: background-color 0.3s;
        }

        .nav-links a:hover {
            background-color: rgba(59, 130, 246, 0.3);
        }

        .auth-buttons {
            display: flex;
            gap: 1rem;
        }

        .btn {
            padding: 0.75rem 1.5rem;
            border: none;
            border-radius: 25px;
            font-weight: 600;
            text-decoration: none;
            display: inline-block;
            transition: all 0.3s;
            cursor: pointer;
        }

        .btn-primary {
            background: linear-gradient(135deg, #3b82f6, #1d4ed8);
            color: white;
            border: 1px solid rgba(59, 130, 246, 0.3);
        }

        .btn-primary:hover {
            background: linear-gradient(135deg, #2563eb, #1e40af);
            transform: translateY(-2px);
            box-shadow: 0 4px 20px rgba(59, 130, 246, 0.4);
        }

        .btn-secondary {
            background: rgba(30, 30, 52, 0.8);
            color: #e8e8e8;
            border: 2px solid rgba(59, 130, 246, 0.3);
        }

        .btn-secondary:hover {
            background: rgba(59, 130, 246, 0.2);
            border-color: rgba(59, 130, 246, 0.6);
        }

        /* Hero Section */
        .hero {
            padding: 120px 0 80px;
            text-align: center;
            color: white;
        }

        .hero h1 {
            font-size: 3.5rem;
            margin-bottom: 1.5rem;
            font-weight: 700;
            opacity: 0;
            transform: translateY(30px);
            animation: fadeInUp 0.8s ease-out 0.2s forwards;
        }

        .hero p {
            font-size: 1.3rem;
            margin-bottom: 2.5rem;
            opacity: 0;
            max-width: 600px;
            margin-left: auto;
            margin-right: auto;
            transform: translateY(30px);
            animation: fadeInUp 0.8s ease-out 0.4s forwards;
        }

        .hero-buttons {
            display: flex;
            gap: 1.5rem;
            justify-content: center;
            flex-wrap: wrap;
            opacity: 0;
            transform: translateY(30px);
            animation: fadeInUp 0.8s ease-out 0.6s forwards;
        }

        .btn-large {
            padding: 1rem 2rem;
            font-size: 1.1rem;
        }

        /* Features Section */
        .features {
            padding: 80px 0;
            background: rgba(15, 20, 25, 0.95);
            backdrop-filter: blur(20px);
        }

        .features h2 {
            text-align: center;
            font-size: 2.5rem;
            margin-bottom: 3rem;
            color: #f8fafc;
        }

        .features-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 3rem;
            margin-top: 2rem;
        }

        .feature-card {
            text-align: center;
            padding: 2rem;
            border-radius: 15px;
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            backdrop-filter: blur(10px);
            box-shadow: 0 10px 30px rgba(0,0,0,0.3);
            transition: all 0.4s cubic-bezier(0.4, 0, 0.2, 1);
            transform: translateY(0);
            opacity: 0;
            animation: fadeInUp 0.6s ease-out forwards;
        }

        .feature-card:nth-child(1) { animation-delay: 0.1s; }
        .feature-card:nth-child(2) { animation-delay: 0.2s; }
        .feature-card:nth-child(3) { animation-delay: 0.3s; }
        .feature-card:nth-child(4) { animation-delay: 0.4s; }
        .feature-card:nth-child(5) { animation-delay: 0.5s; }
        .feature-card:nth-child(6) { animation-delay: 0.6s; }

        .feature-card:hover {
            transform: translateY(-8px) scale(1.02);
            box-shadow: 0 20px 40px rgba(59, 130, 246, 0.2);
            border-color: rgba(59, 130, 246, 0.4);
        }

        @keyframes fadeInUp {
            from {
                opacity: 0;
                transform: translateY(30px);
            }
            to {
                opacity: 1;
                transform: translateY(0);
            }
        }

        .feature-icon {
            font-size: 3rem;
            margin-bottom: 1rem;
            display: inline-block;
            transition: transform 0.3s ease;
        }

        .feature-card:hover .feature-icon {
            transform: scale(1.1) rotate(5deg);
        }

        .feature-card h3 {
            font-size: 1.5rem;
            margin-bottom: 1rem;
            color: #f8fafc;
        }

        .feature-card p {
            color: #cbd5e1;
            line-height: 1.6;
        }

        /* Tools Section */
        .tools {
            padding: 80px 0;
            background: rgba(26, 26, 46, 0.8);
            backdrop-filter: blur(20px);
        }

        .tools h2 {
            text-align: center;
            font-size: 2.5rem;
            margin-bottom: 3rem;
            color: #f8fafc;
        }

        .tools-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
            gap: 2rem;
            margin-top: 2rem;
        }

        .tool-category {
            background: rgba(30, 30, 52, 0.7);
            border: 1px solid rgba(59, 130, 246, 0.2);
            backdrop-filter: blur(10px);
            padding: 2rem;
            border-radius: 10px;
            box-shadow: 0 5px 15px rgba(0,0,0,0.3);
            transition: all 0.3s ease;
            opacity: 0;
            transform: translateY(20px);
            animation: slideInUp 0.6s ease-out forwards;
        }

        .tool-category:nth-child(1) { animation-delay: 0.1s; }
        .tool-category:nth-child(2) { animation-delay: 0.2s; }
        .tool-category:nth-child(3) { animation-delay: 0.3s; }
        .tool-category:nth-child(4) { animation-delay: 0.4s; }
        .tool-category:nth-child(5) { animation-delay: 0.5s; }
        .tool-category:nth-child(6) { animation-delay: 0.6s; }

        .tool-category:hover {
            transform: translateY(-5px);
            box-shadow: 0 15px 30px rgba(59, 130, 246, 0.2);
            border-color: rgba(59, 130, 246, 0.4);
        }

        @keyframes slideInUp {
            from {
                opacity: 0;
                transform: translateY(30px);
            }
            to {
                opacity: 1;
                transform: translateY(0);
            }
        }

        .tool-category h3 {
            font-size: 1.3rem;
            margin-bottom: 1rem;
            color: #3b82f6;
            display: flex;
            align-items: center;
            gap: 0.5rem;
        }

        .tool-list {
            list-style: none;
        }

        .tool-list li {
            padding: 0.3rem 0;
            color: #cbd5e1;
            position: relative;
            padding-left: 1rem;
        }

        .tool-list li::before {
            content: "✓";
            position: absolute;
            left: 0;
            color: #10b981;
            font-weight: bold;
        }

        /* About Section */
        .about {
            padding: 80px 0;
            background: rgba(15, 20, 25, 0.95);
            backdrop-filter: blur(20px);
        }

        .about h2 {
            text-align: center;
            font-size: 2.5rem;
            margin-bottom: 3rem;
            color: #f8fafc;
        }

        .about-content {
            display: grid;
            grid-template-columns: 2fr 1fr;
            gap: 4rem;
            align-items: start;
        }

        .about-text h3 {
            font-size: 1.5rem;
            margin-bottom: 1rem;
            color: #3b82f6;
        }

        .about-text p {
            margin-bottom: 2rem;
            line-height: 1.7;
            color: #cbd5e1;
        }

        .about-stats {
            display: flex;
            flex-direction: column;
            gap: 2rem;
        }

        .stat-item {
            text-align: center;
            padding: 1.5rem;
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            backdrop-filter: blur(10px);
            border-radius: 12px;
            transition: all 0.3s ease;
        }

        .stat-item:hover {
            transform: translateY(-3px);
            box-shadow: 0 8px 25px rgba(59, 130, 246, 0.2);
            border-color: rgba(59, 130, 246, 0.4);
        }

        .stat-number {
            font-size: 2.5rem;
            font-weight: bold;
            color: #3b82f6;
            margin-bottom: 0.5rem;
        }

        .stat-label {
            color: #cbd5e1;
            font-weight: 500;
        }

        /* CTA Section */
        .cta {
            padding: 80px 0;
            background: linear-gradient(135deg, #1a1a2e 0%, #3b82f6 50%, #1e40af 100%);
            text-align: center;
            color: white;
        }

        .cta h2 {
            font-size: 2.5rem;
            margin-bottom: 1rem;
        }

        .cta p {
            font-size: 1.2rem;
            margin-bottom: 2rem;
            opacity: 0.9;
        }

        /* Privacy Section */
        .privacy-section {
            padding: 80px 0;
            background: rgba(26, 26, 46, 0.9);
            backdrop-filter: blur(20px);
        }

        .privacy-section h2 {
            text-align: center;
            font-size: 2.5rem;
            margin-bottom: 1rem;
            color: #f8fafc;
        }

        .privacy-intro {
            text-align: center;
            font-size: 1.1rem;
            margin-bottom: 3rem;
            color: #cbd5e1;
            max-width: 700px;
            margin-left: auto;
            margin-right: auto;
        }

        .privacy-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
            gap: 2rem;
            margin-bottom: 2rem;
        }

        .privacy-item {
            display: flex;
            align-items: center;
            gap: 1rem;
            padding: 1.5rem;
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            border-radius: 10px;
            color: #cbd5e1;
            transition: all 0.3s ease;
        }

        .privacy-item:hover {
            transform: translateY(-3px);
            border-color: rgba(59, 130, 246, 0.4);
        }

        .privacy-icon {
            color: #10b981;
            font-size: 1.5rem;
            font-weight: bold;
        }

        .privacy-link {
            text-align: center;
            font-size: 1.1rem;
            color: #cbd5e1;
        }

        .link-blue {
            color: #3b82f6;
            text-decoration: none;
            border-bottom: 1px solid transparent;
            transition: border-color 0.2s;
        }

        .link-blue:hover {
            border-bottom-color: #3b82f6;
        }

        /* Footer */
        .footer {
            background: #0f1419;
            border-top: 1px solid rgba(59, 130, 246, 0.2);
            color: #cbd5e1;
            padding: 2rem 0;
        }

        .footer-content {
            display: flex;
            flex-direction: column;
            align-items: center;
            gap: 1rem;
        }

        .footer-links {
            display: flex;
            align-items: center;
            gap: 1rem;
        }

        .footer-links a {
            color: #3b82f6;
            text-decoration: none;
            transition: color 0.2s;
        }

        .footer-links a:hover {
            color: #60a5fa;
            text-decoration: underline;
        }

        .separator {
            color: rgba(59, 130, 246, 0.3);
        }

        /* Responsive */
        @media (max-width: 768px) {
            .hero h1 {
                font-size: 2.5rem;
            }
            
            .hero p {
                font-size: 1.1rem;
            }
            
            .hero-buttons {
                flex-direction: column;
                align-items: center;
            }
            
            .nav-links {
                display: none;
            }
            
            .auth-buttons {
                flex-direction: column;
                gap: 0.5rem;
            }

            .about-content {
                grid-template-columns: 1fr;
                gap: 2rem;
            }

            .about-stats {
                flex-direction: row;
                flex-wrap: wrap;
            }

            .stat-item {
                flex: 1;
                min-width: 150px;
            }

            .footer-content {
                text-align: center;
            }

            .footer-links {
                flex-wrap: wrap;
                justify-content: center;
            }
        }
    </style>
</head>
<body>
    <!-- Header -->
    <header class="header">
        <div class="container">
            <nav class="nav">
                <a href="/" class="logo">🎬 VideoSync</a>
                <div class="nav-links">
                    <a href="#features">Features</a>
                    <a href="#tools">Tools</a>
                    <a href="#about">About</a>
                </div>
                <div class="auth-buttons">
                    <a href="/login" class="btn btn-secondary">Login</a>
                    <a href="/signup" class="btn btn-primary">Sign Up</a>
                </div>
            </nav>
        </div>
    </header>

    <!-- Hero Section -->
    <section class="hero">
        <div class="container">
            <h1>AI-Powered Video Editing Made Simple</h1>
            <p>Transform your videos with natural language commands and publish directly to YouTube. No complex software, no steep learning curves - just tell our AI what you want, and watch the magic happen.</p>
            <div class="hero-buttons">
                <a href="/signup" class="btn btn-primary btn-large">Get Started Free</a>
                <a href="/login" class="btn btn-secondary btn-large">Sign In</a>
            </div>
        </div>
    </section>

    <!-- Features Section -->
    <section class="features" id="features">
        <div class="container">
            <h2>Revolutionary Video Editing Experience</h2>
            <div class="features-grid">
                <div class="feature-card">
                    <div class="feature-icon">🤖</div>
                    <h3>AI-Powered Assistant</h3>
                    <p>Chat with our intelligent AI to edit videos using natural language. No need to learn complex tools or techniques.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">⚡</div>
                    <h3>Lightning Fast</h3>
                    <p>Process videos in seconds, not minutes. Our optimized backend handles complex operations efficiently.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">🎯</div>
                    <h3>Professional Quality</h3>
                    <p>Get broadcast-quality results with advanced algorithms and professional-grade video processing.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">🎥</div>
                    <h3>YouTube Integration</h3>
                    <p>Upload directly to YouTube, manage videos, track analytics, optimize metadata, and moderate comments—all from one place.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">🔒</div>
                    <h3>Secure & Private</h3>
                    <p>Your videos are processed securely with enterprise-grade encryption and privacy protection.</p>
                </div>
                <div class="feature-card">
                    <div class="feature-icon">💾</div>
                    <h3>Smart Memory</h3>
                    <p>Our AI remembers your preferences and past projects to provide better, more personalized results.</p>
                </div>
            </div>
        </div>
    </section>

    <!-- Tools Section -->
    <section class="tools" id="tools">
        <div class="container">
            <h2>Comprehensive Video Editing Toolkit</h2>
            <div class="tools-grid">
                <div class="tool-category">
                    <h3>🎬 Core Editing</h3>
                    <ul class="tool-list">
                        <li>Trim & Cut Videos</li>
                        <li>Merge Multiple Videos</li>
                        <li>Split Videos</li>
                        <li>Video Analysis</li>
                    </ul>
                </div>
                <div class="tool-category">
                    <h3>🔧 Transform</h3>
                    <ul class="tool-list">
                        <li>Resize & Scale</li>
                        <li>Crop Videos</li>
                        <li>Rotate & Flip</li>
                        <li>Speed Adjustment</li>
                        <li>Video Stabilization</li>
                    </ul>
                </div>
                <div class="tool-category">
                    <h3>🎨 Visual Effects</h3>
                    <ul class="tool-list">
                        <li>Text Overlays</li>
                        <li>Image Overlays</li>
                        <li>Color Adjustment</li>
                        <li>Filters & Effects</li>
                        <li>Subtitles</li>
                    </ul>
                </div>
                <div class="tool-category">
                    <h3>🔊 Audio</h3>
                    <ul class="tool-list">
                        <li>Extract Audio</li>
                        <li>Add Background Music</li>
                        <li>Volume Control</li>
                        <li>Audio Fade Effects</li>
                    </ul>
                </div>
                <div class="tool-category">
                    <h3>📤 Export</h3>
                    <ul class="tool-list">
                        <li>Format Conversion</li>
                        <li>Platform Optimization</li>
                        <li>Compression</li>
                        <li>Thumbnail Creation</li>
                        <li>Frame Extraction</li>
                    </ul>
                </div>
                <div class="tool-category">
                    <h3>🚀 Advanced</h3>
                    <ul class="tool-list">
                        <li>Picture-in-Picture</li>
                        <li>Green Screen (Chroma Key)</li>
                        <li>Split Screen</li>
                        <li>Advanced Transitions</li>
                    </ul>
                </div>
            </div>
        </div>
    </section>

    <!-- About Section -->
    <section class="about" id="about">
        <div class="container">
            <h2>About VideoSync</h2>
            <div class="about-content">
                <div class="about-text">
                    <h3>Revolutionizing Video Editing with AI</h3>
                    <p>Our AI-powered video editor transforms the way creators work with video content. Instead of learning complex software interfaces, simply describe what you want in natural language, and our intelligent system handles the technical details.</p>
                    
                    <h3>Built for Modern Creators</h3>
                    <p>Whether you're a content creator, marketer, educator, or filmmaker, our platform adapts to your workflow. From simple cuts and transitions to advanced effects and color grading, experience professional-quality results without the learning curve.</p>
                    
                    <h3>Secure & Reliable</h3>
                    <p>Your content is processed with enterprise-grade security and privacy protection. All video processing happens in our secure cloud infrastructure, ensuring your creative work remains safe and confidential.</p>
                </div>
                <div class="about-stats">
                    <div class="stat-item">
                        <div class="stat-number">10,000+</div>
                        <div class="stat-label">Videos Processed</div>
                    </div>
                    <div class="stat-item">
                        <div class="stat-number">2,500+</div>
                        <div class="stat-label">Active Users</div>
                    </div>
                    <div class="stat-item">
                        <div class="stat-number">99.9%</div>
                        <div class="stat-label">Uptime</div>
                    </div>
                </div>
            </div>
        </div>
    </section>

    <!-- Privacy & Security Section -->
    <section class="privacy-section" id="privacy">
        <div class="container">
            <h2>🔒 Your Data, Your Control</h2>
            <p class="privacy-intro">We prioritize your privacy and security. Our platform is built with transparency and compliance at its core.</p>
            <div class="privacy-grid">
                <div class="privacy-item">
                    <div class="privacy-icon">✓</div>
                    <div>Encrypted OAuth tokens</div>
                </div>
                <div class="privacy-item">
                    <div class="privacy-icon">✓</div>
                    <div>GDPR & CCPA compliant</div>
                </div>
                <div class="privacy-item">
                    <div class="privacy-icon">✓</div>
                    <div>YouTube API Terms compliant</div>
                </div>
                <div class="privacy-item">
                    <div class="privacy-icon">✓</div>
                    <div>Revoke access anytime</div>
                </div>
            </div>
            <p class="privacy-link">Learn more in our <a href="/privacy" class="link-blue">Privacy Policy</a> and <a href="/terms" class="link-blue">Terms of Service</a>.</p>
        </div>
    </section>

    <!-- CTA Section -->
    <section class="cta">
        <div class="container">
            <h2>Ready to Transform Your Videos?</h2>
            <p>Join thousands of creators who are already using AI to create amazing videos effortlessly.</p>
            <a href="/signup" class="btn btn-primary btn-large">Start Creating Now</a>
        </div>
    </section>

    <!-- Footer -->
    <footer class="footer">
        <div class="container">
            <div class="footer-content">
                <p>&copy; 2025 VideoSync. Professional AI-powered video editing solutions.</p>
                <div class="footer-links">
                    <a href="/privacy">Privacy Policy</a>
                    <span class="separator">|</span>
                    <a href="/terms">Terms of Service</a>
                    <span class="separator">|</span>
                    <a href="/help">Help & Support</a>
                </div>
            </div>
        </div>
    </footer>

    <script>
        class DynamicBackgroundManager {
            constructor() {
                this.lastBackgroundUpdate = Date.now();
                this.updateInterval = 5 * 60 * 1000; // 5 minutes
                this.retryDelay = 30 * 1000; // 30 seconds on error
                this.isUpdating = false;
                
                this.init();
            }

            async init() {
                // Load initial background
                await this.updateBackground();
                
                // Set up periodic updates
                setInterval(() => {
                    this.checkAndUpdateBackground();
                }, 60 * 1000); // Check every minute
            }

            async checkAndUpdateBackground() {
                if (this.isUpdating) return;
                
                const timeSinceLastUpdate = Date.now() - this.lastBackgroundUpdate;
                if (timeSinceLastUpdate >= this.updateInterval) {
                    await this.updateBackground();
                }
            }

            async updateBackground() {
                if (this.isUpdating) return;
                
                this.isUpdating = true;
                
                try {
                    console.log('🎨 Fetching new dynamic background...');
                    
                    const response = await fetch('/api/background/image');
                    
                    if (response.ok) {
                        const contentType = response.headers.get('content-type');
                        
                        if (contentType && contentType.includes('application/json')) {
                            // Fallback gradient
                            const data = await response.json();
                            if (data.fallback && data.gradient) {
                                document.body.style.background = data.gradient;
                                console.log('🎨 Applied fallback gradient background');
                            }
                        } else {
                            // Image response
                            const blob = await response.blob();
                            const imageUrl = URL.createObjectURL(blob);
                            
                            // Create overlay for smooth transition
                            const overlay = document.createElement('div');
                            overlay.style.cssText = `
                                position: fixed;
                                top: 0;
                                left: 0;
                                width: 100%;
                                height: 100%;
                                background-image: url(${imageUrl});
                                background-size: cover;
                                background-position: center;
                                background-attachment: fixed;
                                opacity: 0;
                                transition: opacity 1s ease-in-out;
                                z-index: -1;
                                pointer-events: none;
                            `;
                            
                            document.body.appendChild(overlay);
                            
                            // Trigger fade in
                            setTimeout(() => {
                                overlay.style.opacity = '0.3'; // Semi-transparent overlay
                            }, 100);
                            
                            // Clean up old overlays after transition
                            setTimeout(() => {
                                const oldOverlays = document.querySelectorAll('div[style*="background-image"]');
                                oldOverlays.forEach((old, index) => {
                                    if (index < oldOverlays.length - 1) {
                                        old.remove();
                                    }
                                });
                            }, 1100);
                            
                            console.log('🎨 Applied new AI-generated background');
                        }
                        
                        this.lastBackgroundUpdate = Date.now();
                    } else {
                        console.warn('Failed to fetch background image:', response.status);
                    }
                } catch (error) {
                    console.error('Error updating background:', error);
                    // Retry with shorter delay on error
                    setTimeout(() => {
                        this.lastBackgroundUpdate = Date.now() - this.updateInterval + this.retryDelay;
                    }, this.retryDelay);
                } finally {
                    this.isUpdating = false;
                }
            }
        }

        // Initialize dynamic background manager
        document.addEventListener('DOMContentLoaded', () => {
            new DynamicBackgroundManager();

            // Hide login/signup buttons if user is authenticated
            const authToken = localStorage.getItem('authToken');
            if (authToken) {
                const authButtons = document.querySelectorAll('.auth-buttons, .hero-buttons');
                authButtons.forEach(container => {
                    container.style.display = 'none';
                });

                // Show logged-in state
                const nav = document.querySelector('nav .auth-buttons');
                if (nav) {
                    nav.innerHTML = '<a href="/dashboard" class="btn btn-primary">Go to Dashboard</a>';
                    nav.style.display = 'flex';
                }
            }
        });

        // Add subtle loading indicator for background updates
        let backgroundLoadingIndicator = null;

        function showBackgroundLoading() {
            if (backgroundLoadingIndicator) return;
            
            backgroundLoadingIndicator = document.createElement('div');
            backgroundLoadingIndicator.innerHTML = '🎨 Refreshing background...';
            backgroundLoadingIndicator.style.cssText = `
                position: fixed;
                top: 20px;
                right: 20px;
                background: rgba(0, 0, 0, 0.8);
                color: white;
                padding: 8px 16px;
                border-radius: 20px;
                font-size: 12px;
                z-index: 1000;
                opacity: 0;
                transition: opacity 0.3s ease;
            `;
            
            document.body.appendChild(backgroundLoadingIndicator);
            setTimeout(() => {
                backgroundLoadingIndicator.style.opacity = '1';
            }, 100);
            
            setTimeout(() => {
                if (backgroundLoadingIndicator) {
                    backgroundLoadingIndicator.style.opacity = '0';
                    setTimeout(() => {
                        if (backgroundLoadingIndicator) {
                            backgroundLoadingIndicator.remove();
                            backgroundLoadingIndicator = null;
                        }
                    }, 300);
                }
            }, 3000);
        }
    </script>
</body>
</html>
    "###;
    
    Html(html.to_string())
}

pub async fn login_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Login - VideoSync</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        html {
            scroll-behavior: smooth;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            background-size: cover;
            background-position: center;
            background-attachment: fixed;
            transition: background-image 1s ease-in-out;
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            color: #e8e8e8;
        }

        .auth-container {
            background: rgba(30, 30, 52, 0.8);
            backdrop-filter: blur(20px);
            border: 1px solid rgba(59, 130, 246, 0.3);
            padding: 3rem;
            border-radius: 20px;
            box-shadow: 0 20px 40px rgba(0,0,0,0.3);
            width: 100%;
            max-width: 400px;
            color: #e8e8e8;
            opacity: 0;
            transform: translateY(30px) scale(0.95);
            animation: authFormSlideIn 0.6s ease-out forwards;
        }

        @keyframes authFormSlideIn {
            to {
                opacity: 1;
                transform: translateY(0) scale(1);
            }
        }

        .auth-header {
            text-align: center;
            margin-bottom: 2rem;
        }

        .auth-header h1 {
            font-size: 2rem;
            color: #f8fafc;
            margin-bottom: 0.5rem;
        }

        .auth-header p {
            color: #cbd5e1;
        }

        .form-group {
            margin-bottom: 1.5rem;
        }

        .form-group label {
            display: block;
            margin-bottom: 0.5rem;
            color: #f8fafc;
            font-weight: 500;
        }

        .form-group input {
            width: 100%;
            padding: 0.75rem 1rem;
            border: 2px solid rgba(59, 130, 246, 0.3);
            border-radius: 10px;
            font-size: 1rem;
            background: rgba(15, 20, 25, 0.6);
            color: #e8e8e8;
            transition: all 0.3s ease;
        }

        .form-group input:focus {
            outline: none;
            border-color: #3b82f6;
            background: rgba(15, 20, 25, 0.8);
            transform: translateY(-2px);
            box-shadow: 0 4px 12px rgba(59, 130, 246, 0.3);
        }

        .form-group input::placeholder {
            color: #9ca3af;
        }

        .btn {
            width: 100%;
            padding: 0.75rem;
            background: linear-gradient(135deg, #3b82f6, #1d4ed8);
            color: white;
            border: 1px solid rgba(59, 130, 246, 0.3);
            border-radius: 10px;
            font-size: 1rem;
            font-weight: 600;
            cursor: pointer;
            transition: all 0.3s ease;
        }

        .btn:hover {
            background: linear-gradient(135deg, #2563eb, #1e40af);
            transform: translateY(-2px);
            box-shadow: 0 4px 12px rgba(59, 130, 246, 0.4);
        }

        .auth-links {
            text-align: center;
            margin-top: 1.5rem;
        }

        .auth-links a {
            color: #3b82f6;
            text-decoration: none;
        }

        .auth-links a:hover {
            text-decoration: underline;
        }

        .error-message {
            background: #f8d7da;
            color: #721c24;
            padding: 0.75rem;
            border-radius: 8px;
            margin-bottom: 1rem;
            display: none;
        }

        .success-message {
            background: #d4edda;
            color: #155724;
            padding: 0.75rem;
            border-radius: 8px;
            margin-bottom: 1rem;
            display: none;
        }

        .back-link {
            position: absolute;
            top: 2rem;
            left: 2rem;
            color: #cbd5e1;
            text-decoration: none;
            font-weight: 500;
            transition: color 0.3s ease;
        }

        .back-link:hover {
            color: #3b82f6;
            text-decoration: underline;
        }

        .divider {
            text-align: center;
            margin: 1.5rem 0;
            position: relative;
        }

        .divider::before {
            content: '';
            position: absolute;
            left: 0;
            top: 50%;
            width: 100%;
            height: 1px;
            background: rgba(59, 130, 246, 0.3);
        }

        .divider span {
            background: rgba(30, 30, 52, 0.8);
            padding: 0 1rem;
            position: relative;
            color: #cbd5e1;
        }

        .btn-google {
            width: 100%;
            padding: 0.75rem;
            background: white;
            color: #444;
            border: 1px solid #ddd;
            border-radius: 10px;
            font-size: 1rem;
            font-weight: 600;
            cursor: pointer;
            transition: all 0.3s ease;
            display: flex;
            align-items: center;
            justify-content: center;
            gap: 0.75rem;
            margin-bottom: 1rem;
        }

        .btn-google:hover {
            background: #f8f9fa;
            border-color: #3b82f6;
            transform: translateY(-2px);
            box-shadow: 0 4px 12px rgba(59, 130, 246, 0.2);
        }

        .google-icon {
            width: 20px;
            height: 20px;
        }
    </style>
</head>
<body>
    <a href="/" class="back-link">← Back to Home</a>

    <div class="auth-container">
        <div class="auth-header">
            <h1>🎬 Welcome Back</h1>
            <p>Sign in to your account</p>
        </div>

        <div id="errorMessage" class="error-message"></div>
        <div id="successMessage" class="success-message"></div>

        <button onclick="signInWithGoogle()" class="btn-google">
            <svg class="google-icon" viewBox="0 0 48 48"><path fill="#EA4335" d="M24 9.5c3.54 0 6.71 1.22 9.21 3.6l6.85-6.85C35.9 2.38 30.47 0 24 0 14.62 0 6.51 5.38 2.56 13.22l7.98 6.19C12.43 13.72 17.74 9.5 24 9.5z"/><path fill="#4285F4" d="M46.98 24.55c0-1.57-.15-3.09-.38-4.55H24v9.02h12.94c-.58 2.96-2.26 5.48-4.78 7.18l7.73 6c4.51-4.18 7.09-10.36 7.09-17.65z"/><path fill="#FBBC05" d="M10.53 28.59c-.48-1.45-.76-2.99-.76-4.59s.27-3.14.76-4.59l-7.98-6.19C.92 16.46 0 20.12 0 24c0 3.88.92 7.54 2.56 10.78l7.97-6.19z"/><path fill="#34A853" d="M24 48c6.48 0 11.93-2.13 15.89-5.81l-7.73-6c-2.15 1.45-4.92 2.3-8.16 2.3-6.26 0-11.57-4.22-13.47-9.91l-7.98 6.19C6.51 42.62 14.62 48 24 48z"/></svg>
            Sign in with Google
        </button>

        <div class="divider"><span>OR</span></div>

        <form id="loginForm">
            <div class="form-group">
                <label for="email">Email Address</label>
                <input type="email" id="email" name="email" required>
            </div>

            <div class="form-group">
                <label for="password">Password</label>
                <input type="password" id="password" name="password" required>
            </div>

            <button type="submit" class="btn">Sign In</button>
        </form>

        <div class="auth-links">
            <p>Don't have an account? <a href="/signup">Sign up here</a></p>
        </div>
    </div>

    <script>
        document.getElementById('loginForm').addEventListener('submit', async (e) => {
            e.preventDefault();
            
            const email = document.getElementById('email').value;
            const password = document.getElementById('password').value;
            
            try {
                const response = await fetch('/api/auth/login', {
                    method: 'POST',
                    headers: {
                        'Content-Type': 'application/json',
                    },
                    body: JSON.stringify({ email, password }),
                });
                
                const data = await response.json();
                
                if (data.success) {
                    localStorage.setItem('authToken', data.token);
                    localStorage.setItem('user', JSON.stringify(data.user));
                    
                    document.getElementById('successMessage').textContent = 'Login successful! Redirecting...';
                    document.getElementById('successMessage').style.display = 'block';
                    document.getElementById('errorMessage').style.display = 'none';
                    
                    setTimeout(() => {
                        window.location.href = '/dashboard';
                    }, 1000);
                } else {
                    document.getElementById('errorMessage').textContent = data.message;
                    document.getElementById('errorMessage').style.display = 'block';
                    document.getElementById('successMessage').style.display = 'none';
                }
            } catch (error) {
                document.getElementById('errorMessage').textContent = 'Network error. Please try again.';
                document.getElementById('errorMessage').style.display = 'block';
                document.getElementById('successMessage').style.display = 'none';
            }
        });

        function signInWithGoogle() {
            window.location.href = '/api/auth/google?redirect_to=' + encodeURIComponent('/dashboard');
        }

        // Dynamic Background Management for Login Page
        class LoginDynamicBackgroundManager {
            constructor() {
                this.lastBackgroundUpdate = Date.now();
                this.updateInterval = 5 * 60 * 1000; // 5 minutes
                this.retryDelay = 30 * 1000; // 30 seconds on error
                this.isUpdating = false;
                
                this.init();
            }

            async init() {
                // Load initial background
                await this.updateBackground();
                
                // Set up periodic updates
                setInterval(() => {
                    this.checkAndUpdateBackground();
                }, 60 * 1000); // Check every minute
            }

            async checkAndUpdateBackground() {
                if (this.isUpdating) return;
                
                const timeSinceLastUpdate = Date.now() - this.lastBackgroundUpdate;
                if (timeSinceLastUpdate >= this.updateInterval) {
                    await this.updateBackground();
                }
            }

            async updateBackground() {
                if (this.isUpdating) return;
                
                this.isUpdating = true;
                
                try {
                    const response = await fetch('/api/background/image');
                    
                    if (response.ok) {
                        const contentType = response.headers.get('content-type');
                        
                        if (contentType && contentType.includes('application/json')) {
                            // Fallback gradient
                            const data = await response.json();
                            if (data.fallback && data.gradient) {
                                document.body.style.background = data.gradient;
                            }
                        } else {
                            // Image response
                            const blob = await response.blob();
                            const imageUrl = URL.createObjectURL(blob);
                            
                            // Create overlay for smooth transition
                            const overlay = document.createElement('div');
                            overlay.style.cssText = `
                                position: fixed;
                                top: 0;
                                left: 0;
                                width: 100%;
                                height: 100%;
                                background-image: url(${imageUrl});
                                background-size: cover;
                                background-position: center;
                                background-attachment: fixed;
                                opacity: 0;
                                transition: opacity 1s ease-in-out;
                                z-index: -1;
                                pointer-events: none;
                            `;
                            
                            document.body.appendChild(overlay);
                            
                            // Trigger fade in with subtle opacity for auth page
                            setTimeout(() => {
                                overlay.style.opacity = '0.15'; // Very subtle for auth pages
                            }, 100);
                            
                            // Clean up old overlays after transition
                            setTimeout(() => {
                                const oldOverlays = document.querySelectorAll('div[style*="background-image"]');
                                oldOverlays.forEach((old, index) => {
                                    if (index < oldOverlays.length - 1) {
                                        old.remove();
                                    }
                                });
                            }, 1100);
                        }
                        
                        this.lastBackgroundUpdate = Date.now();
                    }
                } catch (error) {
                    console.error('Error updating login background:', error);
                    setTimeout(() => {
                        this.lastBackgroundUpdate = Date.now() - this.updateInterval + this.retryDelay;
                    }, this.retryDelay);
                } finally {
                    this.isUpdating = false;
                }
            }
        }

        // Initialize dynamic background manager for login
        document.addEventListener('DOMContentLoaded', () => {
            new LoginDynamicBackgroundManager();
        });
    </script>
</body>
</html>
    "###;
    
    Html(html.to_string())
}

pub async fn signup_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Sign Up - VideoSync</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        html {
            scroll-behavior: smooth;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segui UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            color: #e8e8e8;
        }

        .auth-container {
            background: rgba(22, 33, 62, 0.85);
            backdrop-filter: blur(25px);
            -webkit-backdrop-filter: blur(25px);
            border: 1px solid rgba(59, 130, 246, 0.2);
            padding: 4rem 3rem; /* Added extra top/bottom padding as requested */
            margin: 2rem; /* Extra margin for mobile spacing */
            border-radius: 24px;
            box-shadow: 0 25px 50px rgba(0,0,0,0.4), 
                        0 0 0 1px rgba(255,255,255,0.05) inset;
            width: 100%;
            max-width: 420px;
            color: #e8e8e8;
            opacity: 0;
            transform: translateY(30px) scale(0.95);
            animation: authFormSlideIn 0.6s ease-out forwards;
        }

        @keyframes authFormSlideIn {
            to {
                opacity: 1;
                transform: translateY(0) scale(1);
            }
        }

        .auth-header {
            text-align: center;
            margin-bottom: 2rem;
        }

        .auth-header h1 {
            font-size: 2rem;
            color: #f8fafc;
            margin-bottom: 0.5rem;
        }

        .auth-header p {
            color: #cbd5e1;
        }

        .form-group {
            margin-bottom: 1.5rem;
        }

        .form-group label {
            display: block;
            margin-bottom: 0.5rem;
            color: #f8fafc;
            font-weight: 500;
        }

        .form-group input {
            width: 100%;
            padding: 0.75rem 1rem;
            border: 2px solid rgba(59, 130, 246, 0.3);
            border-radius: 10px;
            font-size: 1rem;
            background: rgba(15, 20, 25, 0.6);
            color: #e8e8e8;
            transition: all 0.3s ease;
        }

        .form-group input:focus {
            outline: none;
            border-color: #3b82f6;
            background: rgba(15, 20, 25, 0.9);
            transform: translateY(-1px);
            box-shadow: 0 8px 25px rgba(59, 130, 246, 0.3),
                        0 0 0 1px rgba(59, 130, 246, 0.1) inset;
        }

        .form-group input::placeholder {
            color: #9ca3af;
        }

        .btn {
            width: 100%;
            padding: 0.875rem;
            background: linear-gradient(135deg, #3b82f6, #1d4ed8);
            color: white;
            border: 1px solid rgba(59, 130, 246, 0.3);
            border-radius: 12px;
            font-size: 1rem;
            font-weight: 600;
            cursor: pointer;
            transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1);
            backdrop-filter: blur(10px);
            -webkit-backdrop-filter: blur(10px);
        }

        .btn:hover {
            background: linear-gradient(135deg, #2563eb, #1e40af);
            transform: translateY(-2px);
            box-shadow: 0 8px 25px rgba(59, 130, 246, 0.4),
                        0 0 0 1px rgba(59, 130, 246, 0.2) inset;
        }

        .auth-links {
            text-align: center;
            margin-top: 1.5rem;
        }

        .auth-links a {
            color: #3b82f6;
            text-decoration: none;
            transition: all 0.3s ease;
        }

        .auth-links a:hover {
            color: #60a5fa;
            text-decoration: underline;
        }

        .error-message {
            background: rgba(248, 215, 218, 0.9);
            backdrop-filter: blur(10px);
            -webkit-backdrop-filter: blur(10px);
            color: #721c24;
            padding: 0.875rem;
            border-radius: 12px;
            margin-bottom: 1rem;
            border: 1px solid rgba(245, 198, 203, 0.3);
            display: none;
        }

        .success-message {
            background: rgba(212, 237, 218, 0.9);
            backdrop-filter: blur(10px);
            -webkit-backdrop-filter: blur(10px);
            color: #155724;
            padding: 0.875rem;
            border-radius: 12px;
            margin-bottom: 1rem;
            border: 1px solid rgba(195, 230, 203, 0.3);
            display: none;
        }

        .back-link {
            position: absolute;
            top: 2rem;
            left: 2rem;
            color: #cbd5e1;
            text-decoration: none;
            font-weight: 500;
            transition: color 0.3s ease;
            backdrop-filter: blur(10px);
            -webkit-backdrop-filter: blur(10px);
            padding: 0.5rem 1rem;
            border-radius: 8px;
            background: rgba(22, 33, 62, 0.3);
        }

        .back-link:hover {
            color: #3b82f6;
            background: rgba(22, 33, 62, 0.5);
            text-decoration: underline;
        }

        .password-requirements {
            font-size: 0.8rem;
            color: #94a3b8;
            margin-top: 0.25rem;
        }

        .divider {
            text-align: center;
            margin: 1.5rem 0;
            position: relative;
        }

        .divider::before {
            content: '';
            position: absolute;
            left: 0;
            top: 50%;
            width: 100%;
            height: 1px;
            background: rgba(59, 130, 246, 0.3);
        }

        .divider span {
            background: rgba(22, 33, 62, 0.85);
            padding: 0 1rem;
            position: relative;
            color: #cbd5e1;
        }

        .btn-google {
            width: 100%;
            padding: 0.875rem;
            background: white;
            color: #444;
            border: 1px solid #ddd;
            border-radius: 12px;
            font-size: 1rem;
            font-weight: 600;
            cursor: pointer;
            transition: all 0.3s ease;
            display: flex;
            align-items: center;
            justify-content: center;
            gap: 0.75rem;
            margin-bottom: 1rem;
        }

        .btn-google:hover {
            background: #f8f9fa;
            border-color: #3b82f6;
            transform: translateY(-2px);
            box-shadow: 0 8px 25px rgba(59, 130, 246, 0.2);
        }

        .google-icon {
            width: 20px;
            height: 20px;
        }
    </style>
</head>
<body>
    <a href="/" class="back-link">← Back to Home</a>

    <div class="auth-container">
        <div class="auth-header">
            <h1>🎬 Get Started</h1>
            <p>Create your account</p>
        </div>

        <div id="errorMessage" class="error-message"></div>
        <div id="successMessage" class="success-message"></div>

        <button onclick="signUpWithGoogle()" class="btn-google">
            <svg class="google-icon" viewBox="0 0 48 48"><path fill="#EA4335" d="M24 9.5c3.54 0 6.71 1.22 9.21 3.6l6.85-6.85C35.9 2.38 30.47 0 24 0 14.62 0 6.51 5.38 2.56 13.22l7.98 6.19C12.43 13.72 17.74 9.5 24 9.5z"/><path fill="#4285F4" d="M46.98 24.55c0-1.57-.15-3.09-.38-4.55H24v9.02h12.94c-.58 2.96-2.26 5.48-4.78 7.18l7.73 6c4.51-4.18 7.09-10.36 7.09-17.65z"/><path fill="#FBBC05" d="M10.53 28.59c-.48-1.45-.76-2.99-.76-4.59s.27-3.14.76-4.59l-7.98-6.19C.92 16.46 0 20.12 0 24c0 3.88.92 7.54 2.56 10.78l7.97-6.19z"/><path fill="#34A853" d="M24 48c6.48 0 11.93-2.13 15.89-5.81l-7.73-6c-2.15 1.45-4.92 2.3-8.16 2.3-6.26 0-11.57-4.22-13.47-9.91l-7.98 6.19C6.51 42.62 14.62 48 24 48z"/></svg>
            Sign up with Google
        </button>

        <div class="divider"><span>OR</span></div>

        <form id="signupForm">
            <div class="form-group">
                <label for="email">Email Address</label>
                <input type="email" id="email" name="email" required>
            </div>

            <div class="form-group">
                <label for="username">Username</label>
                <input type="text" id="username" name="username" required>
            </div>

            <div class="form-group">
                <label for="password">Password</label>
                <input type="password" id="password" name="password" required>
                <div class="password-requirements">
                    Must be at least 6 characters long
                </div>
            </div>

            <div class="form-group">
                <label for="confirmPassword">Confirm Password</label>
                <input type="password" id="confirmPassword" name="confirmPassword" required>
            </div>

            <button type="submit" class="btn">Create Account</button>
        </form>

        <div class="auth-links">
            <p>Already have an account? <a href="/login">Sign in here</a></p>
        </div>
    </div>

    <script>
        document.getElementById('signupForm').addEventListener('submit', async (e) => {
            e.preventDefault();
            
            const email = document.getElementById('email').value;
            const username = document.getElementById('username').value;
            const password = document.getElementById('password').value;
            const confirmPassword = document.getElementById('confirmPassword').value;
            
            if (password !== confirmPassword) {
                document.getElementById('errorMessage').textContent = 'Passwords do not match.';
                document.getElementById('errorMessage').style.display = 'block';
                document.getElementById('successMessage').style.display = 'none';
                return;
            }
            
            if (password.length < 6) {
                document.getElementById('errorMessage').textContent = 'Password must be at least 6 characters long.';
                document.getElementById('errorMessage').style.display = 'block';
                document.getElementById('successMessage').style.display = 'none';
                return;
            }
            
            try {
                const response = await fetch('/api/auth/register', {
                    method: 'POST',
                    headers: {
                        'Content-Type': 'application/json',
                    },
                    body: JSON.stringify({ email, username, password }),
                });
                
                const data = await response.json();
                
                if (data.success) {
                    localStorage.setItem('authToken', data.token);
                    localStorage.setItem('user', JSON.stringify(data.user));
                    
                    document.getElementById('successMessage').textContent = 'Account created successfully! Redirecting...';
                    document.getElementById('successMessage').style.display = 'block';
                    document.getElementById('errorMessage').style.display = 'none';
                    
                    setTimeout(() => {
                        window.location.href = '/dashboard';
                    }, 1000);
                } else {
                    document.getElementById('errorMessage').textContent = data.message;
                    document.getElementById('errorMessage').style.display = 'block';
                    document.getElementById('successMessage').style.display = 'none';
                }
            } catch (error) {
                document.getElementById('errorMessage').textContent = 'Network error. Please try again.';
                document.getElementById('errorMessage').style.display = 'block';
                document.getElementById('successMessage').style.display = 'none';
            }
        });

        function signUpWithGoogle() {
            window.location.href = '/api/auth/google?redirect_to=' + encodeURIComponent('/dashboard');
        }

        // Dynamic Background Manager Class for Signup
        class SignupDynamicBackgroundManager {
            constructor() {
                this.updateInterval = 5 * 60 * 1000; // 5 minutes
                this.retryDelay = 30 * 1000; // 30 seconds retry on error
                this.lastBackgroundUpdate = 0;
                this.isUpdating = false;
                
                // Initial background update
                setTimeout(() => this.updateBackground(), 1000);
                
                // Set up periodic updates
                setInterval(() => {
                    if (Date.now() - this.lastBackgroundUpdate >= this.updateInterval) {
                        this.updateBackground();
                    }
                }, 30000); // Check every 30 seconds
            }
            
            async updateBackground() {
                if (this.isUpdating) return;
                
                this.isUpdating = true;
                
                try {
                    const response = await fetch('/api/background/image');
                    
                    if (response.ok) {
                        const contentType = response.headers.get('content-type');
                        
                        if (contentType && contentType.includes('application/json')) {
                            // Fallback gradient
                            const data = await response.json();
                            if (data.fallback && data.gradient) {
                                document.body.style.background = data.gradient;
                            }
                        } else {
                            // Image response
                            const blob = await response.blob();
                            const imageUrl = URL.createObjectURL(blob);
                            
                            // Create overlay for smooth transition
                            const overlay = document.createElement('div');
                            overlay.style.cssText = `
                                position: fixed;
                                top: 0;
                                left: 0;
                                width: 100%;
                                height: 100%;
                                background-image: url(${imageUrl});
                                background-size: cover;
                                background-position: center;
                                background-attachment: fixed;
                                opacity: 0;
                                transition: opacity 1s ease-in-out;
                                z-index: -1;
                                pointer-events: none;
                            `;
                            
                            document.body.appendChild(overlay);
                            
                            // Trigger fade in with subtle opacity for auth page
                            setTimeout(() => {
                                overlay.style.opacity = '0.15'; // Very subtle for auth pages
                            }, 100);
                            
                            // Clean up old overlays after transition
                            setTimeout(() => {
                                const oldOverlays = document.querySelectorAll('div[style*="background-image"]');
                                oldOverlays.forEach((old, index) => {
                                    if (index < oldOverlays.length - 1) {
                                        old.remove();
                                    }
                                });
                            }, 1100);
                        }
                        
                        this.lastBackgroundUpdate = Date.now();
                    }
                } catch (error) {
                    console.error('Error updating signup background:', error);
                    setTimeout(() => {
                        this.lastBackgroundUpdate = Date.now() - this.updateInterval + this.retryDelay;
                    }, this.retryDelay);
                } finally {
                    this.isUpdating = false;
                }
            }
        }

        // Initialize dynamic background manager for signup
        document.addEventListener('DOMContentLoaded', () => {
            new SignupDynamicBackgroundManager();
        });
    </script>
</body>
</html>
    "###;
    
    Html(html.to_string())
}

pub async fn dashboard_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Dashboard - VideoSync</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        html {
            scroll-behavior: smooth;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            line-height: 1.6;
            color: #e8e8e8;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            background-size: cover;
            background-position: center;
            background-attachment: fixed;
            transition: background-image 1s ease-in-out;
            min-height: 100vh;
        }

        .header {
            background: rgba(26, 26, 46, 0.9);
            backdrop-filter: blur(10px);
            border-bottom: 1px solid rgba(59, 130, 246, 0.3);
            padding: 1rem 0;
        }

        .nav {
            max-width: 1200px;
            margin: 0 auto;
            padding: 0 20px;
            display: flex;
            justify-content: space-between;
            align-items: center;
        }

        .logo {
            font-size: 1.5rem;
            font-weight: bold;
            color: white;
            text-decoration: none;
        }

        .user-menu {
            display: flex;
            align-items: center;
            gap: 1rem;
        }

        .btn {
            padding: 0.5rem 1rem;
            border: none;
            border-radius: 8px;
            text-decoration: none;
            font-weight: 500;
            cursor: pointer;
            transition: all 0.3s;
        }

        .btn-primary {
            background: linear-gradient(135deg, #3b82f6, #1d4ed8);
            color: white;
            border: 1px solid rgba(59, 130, 246, 0.3);
        }

        .btn-primary:hover {
            background: linear-gradient(135deg, #2563eb, #1e40af);
            transform: translateY(-2px);
            box-shadow: 0 4px 20px rgba(59, 130, 246, 0.4);
        }

        .btn-secondary {
            background: rgba(30, 30, 52, 0.8);
            color: #e8e8e8;
            border: 2px solid rgba(59, 130, 246, 0.3);
        }

        .btn-secondary:hover {
            background: rgba(59, 130, 246, 0.2);
            border-color: rgba(59, 130, 246, 0.6);
        }

        .container {
            max-width: 1200px;
            margin: 0 auto;
            padding: 2rem 20px;
        }

        .dashboard-header {
            margin-bottom: 3rem;
            text-align: center;
        }

        .dashboard-header h1 {
            font-size: 3rem;
            color: white;
            font-weight: 800;
            margin-bottom: 0.5rem;
            letter-spacing: -0.5px;
            text-shadow: 0 2px 20px rgba(102, 126, 234, 0.5);
        }

        .dashboard-header p {
            color: rgba(255, 255, 255, 0.8);
            font-size: 1.2rem;
            font-weight: 300;
        }

        .quick-actions {
            background: rgba(255, 255, 255, 0.05);
            backdrop-filter: blur(20px);
            border-radius: 24px;
            padding: 2.5rem;
            box-shadow: 0 20px 60px rgba(0,0,0,0.3);
            margin-bottom: 3rem;
            border: 1px solid rgba(59, 130, 246, 0.2);
        }

        .quick-actions h2 {
            margin-bottom: 2rem;
            color: white;
            font-size: 1.8rem;
            font-weight: 700;
        }

        .action-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
            gap: 1.5rem;
        }

        .action-card {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 2.5rem;
            border-radius: 20px;
            text-decoration: none;
            transition: all 0.4s cubic-bezier(0.4, 0, 0.2, 1);
            opacity: 0;
            transform: translateY(30px);
            animation: dashboardCardSlideIn 0.7s ease-out forwards;
            position: relative;
            overflow: hidden;
            box-shadow: 0 10px 40px rgba(102, 126, 234, 0.3);
        }

        .action-card::before {
            content: '';
            position: absolute;
            top: 0;
            left: 0;
            right: 0;
            bottom: 0;
            background: linear-gradient(135deg, rgba(255,255,255,0.1) 0%, rgba(255,255,255,0) 100%);
            opacity: 0;
            transition: opacity 0.3s;
        }

        .action-card:hover::before {
            opacity: 1;
        }

        .action-card:nth-child(1) { animation-delay: 0.1s; background: linear-gradient(135deg, #667eea 0%, #764ba2 100%); }
        .action-card:nth-child(2) { animation-delay: 0.2s; background: linear-gradient(135deg, #f093fb 0%, #f5576c 100%); }
        .action-card:nth-child(3) { animation-delay: 0.3s; background: linear-gradient(135deg, #4facfe 0%, #00f2fe 100%); }
        .action-card:nth-child(4) { animation-delay: 0.4s; background: linear-gradient(135deg, #43e97b 0%, #38f9d7 100%); }

        .action-card:hover {
            transform: translateY(-10px) scale(1.03);
            color: white;
            box-shadow: 0 20px 60px rgba(102, 126, 234, 0.5);
        }

        @keyframes dashboardCardSlideIn {
            to {
                opacity: 1;
                transform: translateY(0);
            }
        }

        .action-card h3 {
            margin-bottom: 0.5rem;
            display: flex;
            align-items: center;
            gap: 0.5rem;
        }

        .recent-chats {
            background: rgba(255, 255, 255, 0.05);
            backdrop-filter: blur(20px);
            border-radius: 24px;
            padding: 2.5rem;
            box-shadow: 0 20px 60px rgba(0,0,0,0.3);
            border: 1px solid rgba(59, 130, 246, 0.2);
        }

        .recent-chats h2 {
            margin-bottom: 2rem;
            color: white;
            font-size: 1.8rem;
            font-weight: 700;
        }

        .tabs {
            display: flex;
            gap: 1rem;
            margin-bottom: 2rem;
            border-bottom: 2px solid rgba(59, 130, 246, 0.2);
        }

        .tab-button {
            padding: 1rem 2rem;
            background: transparent;
            border: none;
            color: rgba(255, 255, 255, 0.6);
            cursor: pointer;
            font-size: 1rem;
            font-weight: 600;
            transition: all 0.3s;
            border-bottom: 3px solid transparent;
            margin-bottom: -2px;
        }

        .tab-button:hover {
            color: rgba(255, 255, 255, 0.9);
        }

        .tab-button.active {
            color: #3b82f6;
            border-bottom-color: #3b82f6;
        }

        .tab-content {
            display: none;
        }

        .tab-content.active {
            display: block;
        }

        .chat-list {
            list-style: none;
        }

        .chat-item {
            padding: 1.5rem;
            background: rgba(255, 255, 255, 0.03);
            backdrop-filter: blur(10px);
            border: 1px solid rgba(59, 130, 246, 0.15);
            display: flex;
            justify-content: space-between;
            align-items: center;
            transition: all 0.3s;
            border-radius: 12px;
            margin-bottom: 0.75rem;
            cursor: pointer;
        }

        .chat-item:hover {
            background: rgba(59, 130, 246, 0.15);
            backdrop-filter: blur(15px);
            transform: translateX(5px);
            border-left: 3px solid #3b82f6;
            border-color: rgba(59, 130, 246, 0.4);
            box-shadow: 0 4px 15px rgba(59, 130, 246, 0.2);
        }

        .chat-item:last-child {
            border-bottom: none;
        }

        .chat-info {
            flex: 1;
        }

        .chat-title {
            font-weight: 600;
            color: #e8e8e8;
            margin-bottom: 0.25rem;
        }

        .chat-time {
            font-size: 0.875rem;
            color: rgba(255, 255, 255, 0.6);
        }

        .chat-date {
            color: rgba(255, 255, 255, 0.6);
            font-size: 0.9rem;
        }

        .empty-state {
            text-align: center;
            color: rgba(255, 255, 255, 0.6);
            padding: 3rem;
        }

        .empty-state h3 {
            margin-bottom: 1rem;
            color: white;
        }
    </style>
</head>
<body>
    <header class="header">
        <div class="nav">
            <a href="/dashboard" class="logo">🎬 Agentic Video Editor</a>
            <div class="user-menu">
                <span id="userWelcome">Welcome back!</span>
                <button onclick="logout()" class="btn btn-secondary">Logout</button>
            </div>
        </div>
    </header>

    <div class="container">
        <div class="dashboard-header">
            <h1>Your Dashboard</h1>
            <p>Manage your video editing projects and start new conversations with our AI assistant.</p>
        </div>

        <div class="quick-actions">
            <h2>Quick Actions</h2>
            <div class="action-grid">
                <a href="/chat" class="action-card">
                    <h3>💬 Start New Chat</h3>
                    <p>Begin a new video editing session with our AI assistant</p>
                </a>
                <a href="/youtube/manage" class="action-card">
                    <h3>📺 Connect YouTube Channels</h3>
                    <p>Connect and manage your YouTube channels for seamless publishing</p>
                </a>
                <a href="/analytics" class="action-card">
                    <h3>📊 Analytics Dashboard</h3>
                    <p>View YouTube channel performance and video analytics</p>
                </a>
                <a href="/help" class="action-card">
                    <h3>📖 Help & Guide</h3>
                    <p>Learn how to use the AI video editor and YouTube features</p>
                </a>
            </div>
        </div>

        <div class="recent-chats">
            <div class="tabs">
                <button class="tab-button active" onclick="switchTab('recent')">Recent Chats (Last 10)</button>
                <button class="tab-button" onclick="switchTab('all')">All Chats</button>
            </div>

            <div id="recentTab" class="tab-content active">
                <div id="chatList">
                    <div class="empty-state">
                        <h3>No chats yet</h3>
                        <p>Start your first conversation with our AI assistant to see your chat history here.</p>
                        <a href="/chat" class="btn btn-primary" style="margin-top: 1rem; display: inline-block;">Start First Chat</a>
                    </div>
                </div>
            </div>

            <div id="allTab" class="tab-content">
                <div id="allChatsList">
                    <div class="loading">Loading all chats...</div>
                </div>
                <div id="pagination" style="display: none; text-align: center; margin-top: 20px;">
                    <button onclick="loadPage(currentPage - 1)" id="prevBtn" class="btn btn-secondary" style="margin: 0 5px;">Previous</button>
                    <span id="pageInfo" style="margin: 0 15px; color: #e8e8e8;"></span>
                    <button onclick="loadPage(currentPage + 1)" id="nextBtn" class="btn btn-secondary" style="margin: 0 5px;">Next</button>
                </div>
            </div>
        </div>
    </div>

    <script>
        // Check authentication
        const authToken = localStorage.getItem('authToken');
        if (!authToken) {
            window.location.href = '/login';
        }

        // Set user welcome message
        const user = JSON.parse(localStorage.getItem('user') || '{}');
        if (user.username) {
            document.getElementById('userWelcome').textContent = `Welcome back, ${user.username}!`;
        }

        function logout() {
            localStorage.removeItem('authToken');
            localStorage.removeItem('user');
            window.location.href = '/';
        }

        function uploadVideo() {
            window.location.href = '/chat?action=upload';
        }

        function viewProjects() {
            alert('Projects feature coming soon!');
        }

        function viewHelp() {
            alert('Help documentation coming soon!');
        }

        // Load recent chats
        async function loadRecentChats() {
            try {
                const authToken = localStorage.getItem('authToken');
                const response = await fetch('/api/chat/recent', {
                    headers: {
                        'Authorization': `Bearer ${authToken}`
                    }
                });

                if (response.ok) {
                    const data = await response.json();
                    const chatList = document.getElementById('chatList');
                    
                    if (data.success && data.chats && data.chats.length > 0) {
                        chatList.innerHTML = data.chats.map(chat => `
                            <div class="chat-item" onclick="openChat('${chat.session_id}')">
                                <div class="chat-info">
                                    <div class="chat-title">${chat.title}</div>
                                    <div class="chat-time">${new Date(chat.created_at).toLocaleString()}</div>
                                </div>
                            </div>
                        `).join('');
                    } else {
                        // Keep the empty state if no chats
                        chatList.innerHTML = `
                            <div class="empty-state">
                                <h3>No chats yet</h3>
                                <p>Start your first conversation with our AI assistant to see your chat history here.</p>
                                <a href="/chat" class="btn btn-primary" style="margin-top: 1rem; display: inline-block;">Start First Chat</a>
                            </div>
                        `;
                    }
                } else {
                    console.error('Failed to load recent chats');
                }
            } catch (error) {
                console.error('Error loading recent chats:', error);
            }
        }

        function openChat(sessionId) {
            window.location.href = `/chat/${sessionId}`;
        }

        // Tab switching
        let currentPage = 1;
        let totalPages = 1;

        function switchTab(tabName) {
            // Update tab buttons
            document.querySelectorAll('.tab-button').forEach(btn => btn.classList.remove('active'));
            event.target.classList.add('active');

            // Update tab content
            document.querySelectorAll('.tab-content').forEach(content => content.classList.remove('active'));

            if (tabName === 'recent') {
                document.getElementById('recentTab').classList.add('active');
            } else if (tabName === 'all') {
                document.getElementById('allTab').classList.add('active');
                loadAllChats(1);
            }
        }

        // Load all chats with pagination
        async function loadAllChats(page = 1) {
            try {
                const authToken = localStorage.getItem('authToken');
                const response = await fetch(`/api/chat/all?page=${page}&limit=20`, {
                    headers: {
                        'Authorization': `Bearer ${authToken}`
                    }
                });

                if (response.ok) {
                    const data = await response.json();
                    const allChatsList = document.getElementById('allChatsList');
                    const paginationDiv = document.getElementById('pagination');

                    if (data.success && data.chats && data.chats.length > 0) {
                        allChatsList.innerHTML = data.chats.map(chat => `
                            <div class="chat-item" onclick="openChat('${chat.session_id}')">
                                <div class="chat-info">
                                    <div class="chat-title">${chat.title}</div>
                                    <div class="chat-time">${new Date(chat.created_at).toLocaleString()} • ${chat.message_count} messages</div>
                                </div>
                            </div>
                        `).join('');

                        // Update pagination
                        currentPage = data.pagination.page;
                        totalPages = data.pagination.total_pages;

                        document.getElementById('pageInfo').textContent = `Page ${currentPage} of ${totalPages} (${data.pagination.total} total chats)`;
                        document.getElementById('prevBtn').disabled = currentPage <= 1;
                        document.getElementById('nextBtn').disabled = currentPage >= totalPages;
                        paginationDiv.style.display = totalPages > 1 ? 'block' : 'none';
                    } else {
                        allChatsList.innerHTML = `
                            <div class="empty-state">
                                <h3>No chats yet</h3>
                                <p>Start your first conversation with our AI assistant.</p>
                                <a href="/chat" class="btn btn-primary" style="margin-top: 1rem; display: inline-block;">Start First Chat</a>
                            </div>
                        `;
                        paginationDiv.style.display = 'none';
                    }
                } else {
                    console.error('Failed to load all chats');
                }
            } catch (error) {
                console.error('Error loading all chats:', error);
            }
        }

        function loadPage(page) {
            if (page >= 1 && page <= totalPages) {
                loadAllChats(page);
            }
        }

        loadRecentChats();
    </script>
</body>
</html>
    "###;
    
    Html(html.to_string())
}

pub async fn chat_interface_with_session(
    axum::extract::Path(session_id): axum::extract::Path<String>,
) -> Html<String> {
    // Pass the session ID to the chat interface
    chat_interface_with_session_id(Some(session_id)).await
}

pub async fn chat_interface() -> Html<String> {
    chat_interface_with_session_id(None).await
}

pub async fn chat_interface_with_session_id(session_id: Option<String>) -> Html<String> {
    let session_id_js = match session_id {
        Some(id) => format!("'{}'", id),
        None => "null".to_string()
    };
    
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>🎬 VideoSync</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        html {
            scroll-behavior: smooth;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            background-size: cover;
            background-position: center;
            background-attachment: fixed;
            transition: background-image 1s ease-in-out;
            height: 100vh;
            overflow: hidden;
        }

        .app-container {
            display: flex;
            height: 100vh;
            max-width: 1400px;
            margin: 0 auto;
            background: rgba(255, 255, 255, 0.95);
            backdrop-filter: blur(10px);
            box-shadow: 0 0 50px rgba(0,0,0,0.1);
        }

        /* Sidebar */
        .sidebar {
            width: 300px;
            background: #2c3e50;
            color: white;
            display: flex;
            flex-direction: column;
            border-right: 1px solid #34495e;
        }

        .sidebar-header {
            padding: 20px;
            background: #1a252f;
            border-bottom: 1px solid #34495e;
        }

        .sidebar-header h1 {
            font-size: 1.5rem;
            margin-bottom: 0.5rem;
        }

        .sidebar-header p {
            color: #bdc3c7;
            font-size: 0.9rem;
        }

        .file-manager {
            flex: 1;
            padding: 20px;
            overflow-y: auto;
        }

        .file-manager h3 {
            margin-bottom: 15px;
            color: #ecf0f1;
            font-size: 1rem;
        }

        .file-list {
            space-y: 8px;
        }

        .file-item {
            background: #34495e;
            padding: 12px;
            border-radius: 8px;
            margin-bottom: 8px;
            cursor: pointer;
            transition: background-color 0.2s;
        }

        .file-item:hover {
            background: #3b4f61;
        }

        .file-name {
            font-weight: 500;
            font-size: 0.9rem;
            margin-bottom: 4px;
        }

        .file-meta {
            font-size: 0.8rem;
            color: #95a5a6;
        }

        .upload-btn {
            margin: 20px;
            padding: 12px;
            background: #3498db;
            color: white;
            border: none;
            border-radius: 8px;
            cursor: pointer;
            font-weight: 500;
            transition: background-color 0.2s;
        }

        .upload-btn:hover {
            background: #2980b9;
        }

        /* Main Content */
        .main-content {
            flex: 1;
            display: flex;
            flex-direction: column;
        }

        .chat-header {
            padding: 20px;
            background: #f8f9fa;
            border-bottom: 1px solid #e9ecef;
            display: flex;
            justify-content: space-between;
            align-items: center;
        }

        .chat-title {
            font-size: 1.2rem;
            font-weight: 600;
            color: #2c3e50;
        }

        .status-indicator {
            display: flex;
            align-items: center;
            gap: 8px;
            font-size: 0.9rem;
            color: #6c757d;
        }

        .status-dot {
            width: 8px;
            height: 8px;
            border-radius: 50%;
            background: #28a745;
        }

        .status-dot.disconnected {
            background: #dc3545;
        }

        /* Chat Area */
        .chat-container {
            flex: 1;
            display: flex;
            flex-direction: column;
            overflow: hidden;
            position: relative; /* Ensure absolute positioned children stay inside */
        }

        .chat-messages {
            flex: 1;
            padding: 20px;
            overflow-y: auto;
            background: #ffffff;
            scroll-behavior: smooth;
        }

        .message {
            margin-bottom: 20px;
            display: flex;
            align-items: flex-start;
            gap: 12px;
        }

        .message.user {
            flex-direction: row-reverse;
        }

        .message-avatar {
            width: 40px;
            height: 40px;
            border-radius: 50%;
            display: flex;
            align-items: center;
            justify-content: center;
            font-weight: bold;
            color: white;
            font-size: 0.9rem;
        }

        .message.user .message-avatar {
            background: #3498db;
        }

        .message.assistant .message-avatar {
            background: #e74c3c;
        }

        .message-content {
            max-width: 70%;
            padding: 12px 16px;
            border-radius: 18px;
            line-height: 1.4;
        }

        .message.user .message-content {
            background: #3498db;
            color: white;
            border-bottom-right-radius: 4px;
        }

        .message.assistant .message-content {
            background: #f1f3f4;
            color: #2c3e50;
            border-bottom-left-radius: 4px;
        }

        .message-time {
            font-size: 0.8rem;
            color: #6c757d;
            margin-top: 4px;
        }

        /* Download and Stream Buttons */
        .download-button, .stream-button, .youtube-button {
            display: inline-block;
            margin: 10px 5px;
            padding: 10px 20px;
            color: white;
            text-decoration: none;
            border-radius: 25px;
            font-weight: 600;
            transition: transform 0.2s, box-shadow 0.2s;
            border: none;
            cursor: pointer;
        }

        .download-button {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
        }

        .stream-button {
            background: linear-gradient(135deg, #f093fb 0%, #f5576c 100%);
        }

        .youtube-button {
            background: linear-gradient(135deg, #FF0000 0%, #CC0000 100%);
        }

        .download-button:hover, .stream-button:hover, .youtube-button:hover {
            transform: translateY(-2px);
            box-shadow: 0 10px 20px rgba(102, 126, 234, 0.4);
        }

        /* Input Area */
        .chat-input-container {
            padding: 20px;
            background: #f8f9fa;
            border-top: 1px solid #e9ecef;
        }

        .chat-input-wrapper {
            display: flex;
            gap: 12px;
            align-items: flex-end;
        }

        .chat-input {
            flex: 1;
            min-height: 44px;
            max-height: 120px;
            padding: 12px 16px;
            border: 2px solid #e9ecef;
            border-radius: 22px;
            font-size: 1rem;
            resize: none;
            outline: none;
            transition: border-color 0.2s;
        }

        .chat-input:focus {
            border-color: #3498db;
        }

        .send-btn {
            width: 44px;
            height: 44px;
            border: none;
            border-radius: 50%;
            background: #3498db;
            color: white;
            cursor: pointer;
            display: flex;
            align-items: center;
            justify-content: center;
            transition: background-color 0.2s;
        }

        .send-btn:hover:not(:disabled) {
            background: #2980b9;
        }

        .send-btn:disabled {
            background: #bdc3c7;
            cursor: not-allowed;
        }

        /* Progress Bar */
        .progress-container {
            position: absolute; /* Changed from fixed to absolute to stay within chat-container */
            bottom: 100px; /* Positioned above the chat input */
            left: 50%;
            transform: translateX(-50%);
            width: 80%; /* Reduced width to prevent touching edges */
            max-width: 500px;
            background: rgba(26, 26, 46, 0.95);
            border-radius: 15px;
            padding: 15px;
            box-shadow: 0 10px 30px rgba(0, 0, 0, 0.3);
            backdrop-filter: blur(10px);
            display: none;
            z-index: 100; /* Lower z-index needed since it's inside container */
        }
        
        .progress-container.show {
            display: block;
            animation: slideUp 0.3s ease;
        }
        
        @keyframes slideUp {
            from {
                opacity: 0;
                transform: translateX(-50%) translateY(20px);
            }
            to {
                opacity: 1;
                transform: translateX(-50%) translateY(0);
            }
        }
        
        .progress-title {
            color: white;
            font-weight: 600;
            margin-bottom: 10px;
            display: flex;
            align-items: center;
            gap: 10px;
        }
        
        .progress-bar-outer {
            width: 100%;
            height: 8px;
            background: rgba(255, 255, 255, 0.1);
            border-radius: 10px;
            overflow: hidden;
            margin-bottom: 10px;
        }
        
        .progress-bar-inner {
            height: 100%;
            background: linear-gradient(90deg, #3498db, #667eea);
            border-radius: 10px;
            transition: width 0.3s ease;
            position: relative;
            overflow: hidden;
        }
        
        .progress-bar-inner::after {
            content: '';
            position: absolute;
            top: 0;
            left: 0;
            bottom: 0;
            right: 0;
            background: linear-gradient(
                90deg,
                transparent,
                rgba(255, 255, 255, 0.3),
                transparent
            );
            animation: shimmer 2s infinite;
        }
        
        @keyframes shimmer {
            0% {
                transform: translateX(-100%);
            }
            100% {
                transform: translateX(100%);
            }
        }
        
        .progress-text {
            color: #95a5a6;
            font-size: 14px;
        }
        
        /* Tool Execution Display */
        .tool-execution {
            background: rgba(52, 152, 219, 0.1);
            border-left: 4px solid #3498db;
            padding: 10px 15px;
            margin: 10px 0;
            border-radius: 5px;
            animation: fadeIn 0.3s ease;
        }
        
        .tool-execution-title {
            color: #3498db;
            font-weight: 600;
            margin-bottom: 5px;
        }
        
        .tool-execution-details {
            color: #95a5a6;
            font-size: 14px;
        }
        
        /* Welcome Screen */
        .welcome-screen {
            display: flex;
            flex-direction: column;
            align-items: center;
            justify-content: center;
            height: 100%;
            text-align: center;
            color: #6c757d;
        }

        .welcome-screen h2 {
            font-size: 1.5rem;
            margin-bottom: 1rem;
            color: #2c3e50;
        }

        .welcome-screen p {
            max-width: 400px;
            line-height: 1.6;
            margin-bottom: 2rem;
        }

        .example-prompts {
            display: flex;
            flex-direction: column;
            gap: 8px;
        }

        .example-prompt {
            padding: 8px 16px;
            background: #f8f9fa;
            border: 1px solid #e9ecef;
            border-radius: 20px;
            cursor: pointer;
            transition: all 0.2s;
            font-size: 0.9rem;
        }

        .example-prompt:hover {
            background: #e9ecef;
            transform: translateY(-2px);
            box-shadow: 0 4px 12px rgba(0,0,0,0.1);
        }

        /* Loading animation */
        .typing-indicator {
            display: none;
            align-items: center;
            gap: 8px;
            padding: 12px 16px;
            background: #f1f3f4;
            border-radius: 18px;
            margin-bottom: 20px;
        }

        .typing-indicator.show {
            display: flex;
        }

        .typing-dots {
            display: flex;
            gap: 4px;
        }

        .typing-dot {
            width: 8px;
            height: 8px;
            border-radius: 50%;
            background: #6c757d;
            animation: typing 1.4s infinite;
        }

        .typing-dot:nth-child(2) {
            animation-delay: 0.2s;
        }

        .typing-dot:nth-child(3) {
            animation-delay: 0.4s;
        }

        @keyframes typing {
            0%, 60%, 100% {
                transform: translateY(0);
                opacity: 0.4;
            }
            30% {
                transform: translateY(-10px);
                opacity: 1;
            }
        }
    </style>
</head>
<body>
    <div class="app-container">
        <!-- Sidebar -->
        <div class="sidebar">
            <div class="sidebar-header">
                <h1>🎬 Video Editor</h1>
                <p>AI-powered video editing</p>
            </div>
            
            <div class="file-manager">
                <h3>📁 Uploaded Files</h3>
                <div id="fileList" class="file-list">
                    <div class="file-item" style="opacity: 0.5;">
                        <div class="file-name">No files uploaded yet</div>
                        <div class="file-meta">Upload files to get started</div>
                    </div>
                </div>
            </div>
            
            <button class="upload-btn" onclick="uploadFiles()">
                📤 Upload Files
            </button>
        </div>

        <!-- Main Content -->
        <div class="main-content">
            <div class="chat-header">
                <div class="chat-title">Video Editing Assistant</div>
                <div style="display: flex; gap: 15px; align-items: center;">
                    <div class="status-indicator">
                        <div id="statusDot" class="status-dot disconnected"></div>
                        <span id="statusText">Connecting...</span>
                    </div>
                </div>
            </div>

            <div class="chat-container">
                <div id="chatMessages" class="chat-messages">
                    <div class="welcome-screen">
                        <h2>Welcome to your AI Video Editor! 🎬</h2>
                        <p>I can help you edit videos using natural language. Upload your files and tell me what you'd like to do!</p>
                        
                        <div class="example-prompts">
                            <div class="example-prompt" onclick="sendExamplePrompt('Trim my video from 10 seconds to 30 seconds')">
                                "Trim my video from 10 seconds to 30 seconds"
                            </div>
                            <div class="example-prompt" onclick="sendExamplePrompt('Add text overlay saying Hello World')">
                                "Add text overlay saying 'Hello World'"
                            </div>
                            <div class="example-prompt" onclick="sendExamplePrompt('Convert my video to MP4 format')">
                                "Convert my video to MP4 format"
                            </div>
                            <div class="example-prompt" onclick="sendExamplePrompt('Analyze my video and tell me its properties')">
                                "Analyze my video and tell me its properties"
                            </div>
                        </div>
                    </div>
                </div>

                <div class="typing-indicator" id="typingIndicator">
                    <div class="message-avatar" style="background: #e74c3c;">🤖</div>
                    <div style="display: flex; align-items: center; gap: 8px;">
                        <span id="thinkingText">AI is thinking</span>
                        <div class="typing-dots">
                            <div class="typing-dot"></div>
                            <div class="typing-dot"></div>
                            <div class="typing-dot"></div>
                        </div>
                    </div>
                </div>
                
                <!-- Progress Bar Container -->
                <div class="progress-container" id="progressContainer">
                    <div class="progress-title">
                        <span>🎬</span>
                        <span id="progressTitle">Processing video...</span>
                    </div>
                    <div class="progress-bar-outer">
                        <div class="progress-bar-inner" id="progressBar" style="width: 0%"></div>
                    </div>
                    <div class="progress-text" id="progressText">Initializing...</div>
                </div>

                <div class="chat-input-container">
                    <div class="chat-input-wrapper">
                        <textarea 
                            id="chatInput" 
                            class="chat-input" 
                            placeholder="Ask me to edit your videos... (e.g., 'trim my video from 10s to 30s')"
                            rows="1"
                        ></textarea>
                        <button id="sendBtn" class="send-btn" onclick="sendMessage()">
                            <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor">
                                <path d="M2.01 21L23 12 2.01 3 2 10l15 2-15 2z"/>
                            </svg>
                        </button>
                    </div>
                </div>
            </div>
        </div>
    </div>

    <!-- Hidden file input -->
    <input type="file" id="fileInput" multiple accept="video/*,audio/*,image/*,.pdf,.doc,.docx,.txt" style="display: none;">

    <!-- YouTube Upload Modal -->
    <div id="youtubeModal" style="display: none; position: fixed; top: 0; left: 0; width: 100%; height: 100%; background: rgba(0,0,0,0.7); z-index: 10000; justify-content: center; align-items: center;">
        <div style="background: white; border-radius: 15px; padding: 2rem; max-width: 600px; width: 90%; max-height: 80vh; overflow-y: auto;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.5rem;">
                <h2 style="color: #2c3e50; margin: 0;">📺 Post to YouTube</h2>
                <button onclick="closeYouTubeModal()" style="background: none; border: none; font-size: 1.5rem; cursor: pointer; color: #6c757d;">×</button>
            </div>

            <div id="youtubeModalContent">
                <p style="text-align: center; padding: 2rem; color: #6c757d;">Loading your channels...</p>
            </div>
        </div>
    </div>

    <script>
        console.log('✅ SCRIPT TAG LOADED - JavaScript is executing');

        let ws = null;
        let isConnected = false;
        let uploadedFiles = [];
        // Session ID passed from server (either specific session or null for new)
        let providedSessionId = SESSION_ID_PLACEHOLDER;

        console.log('📝 Provided Session ID:', providedSessionId);

        let sessionUuid = providedSessionId || generateUUID();

        console.log('🆔 Final Session UUID:', sessionUuid);

        // Initialize the application
        document.addEventListener('DOMContentLoaded', function() {
            console.log('🚀 DOMContentLoaded event fired - initializing chat interface');
            console.log('Auth token present:', !!localStorage.getItem('authToken'));

            try {
                initializeSession();
                initializeWebSocket();
                setupEventListeners();
                loadUploadedFiles();
                // Load chat history if we have an existing session
                if (providedSessionId) {
                    loadChatHistory(providedSessionId);
                }
            } catch (error) {
                console.error('❌ FATAL: Error during initialization:', error);
                alert('Failed to initialize chat: ' + error.message);
            }
        });

        function generateUUID() {
            return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function(c) {
                var r = Math.random() * 16 | 0, v = c == 'x' ? r : (r & 0x3 | 0x8);
                return v.toString(16);
            });
        }

        function initializeSession() {
            console.log('Session UUID:', sessionUuid);
            if (providedSessionId) {
                console.log('Loading existing session:', providedSessionId);
            } else {
                console.log('Starting new session');
            }
        }
        
        async function loadChatHistory(sessionId) {
            try {
                const authToken = localStorage.getItem('authToken');
                if (!authToken) {
                    console.warn('No auth token, cannot load chat history');
                    return;
                }
                
                const response = await fetch(`/api/chat/history/${sessionId}`, {
                    headers: {
                        'Authorization': `Bearer ${authToken}`
                    }
                });
                
                if (response.ok) {
                    const data = await response.json();
                    if (data.success && data.history && data.history.length > 0) {
                        // Clear welcome screen
                        const messagesContainer = document.getElementById('chatMessages');
                        const welcomeScreen = messagesContainer.querySelector('.welcome-screen');
                        if (welcomeScreen) {
                            welcomeScreen.style.display = 'none';
                        }
                        
                        // Add historical messages with actual timestamps
                        console.log('History data:', data.history);
                        data.history.forEach(msg => {
                            // Each history item has user_message, agent_response, and timestamp
                            // The timestamp represents when the user sent the message
                            if (msg.user_message && msg.user_message.trim() !== '') {
                                addMessage('user', msg.user_message, false, msg.timestamp);
                            }
                            if (msg.agent_response && msg.agent_response.trim() !== '') {
                                // Use same timestamp for the response (it came right after)
                                addMessage('assistant', msg.agent_response, false, msg.timestamp);
                            }
                        });
                        
                        // Scroll to bottom after loading all messages
                        messagesContainer.scrollTop = messagesContainer.scrollHeight;
                        
                        console.log(`Loaded ${data.history.length} historical messages`);
                    }
                } else {
                    const errorData = await response.json();
                    if (errorData.message && errorData.message.includes('Access denied')) {
                        console.error('Access denied to this chat session');
                        addMessage('assistant', "⚠️ You don't have permission to view this chat session.");
                        // Redirect to new chat after 2 seconds
                        setTimeout(() => {
                            window.location.href = '/chat';
                        }, 2000);
                    } else {
                        console.error('Failed to load chat history');
                    }
                }
            } catch (error) {
                console.error('Error loading chat history:', error);
            }
        }

        function initializeWebSocket() {
            try {
                const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
                const wsUrl = `${protocol}//${window.location.host}/ws?session=${sessionUuid}`;

                console.log('Attempting WebSocket connection to:', wsUrl);
                console.log('Session UUID:', sessionUuid);

                ws = new WebSocket(wsUrl);

                ws.onopen = function() {
                    isConnected = true;
                    updateConnectionStatus(true);
                    console.log('✅ Successfully connected to video editing assistant');
                };

            ws.onmessage = function(event) {
                const data = event.data;
                
                try {
                    const jsonData = JSON.parse(data);
                    
                    console.log('Received message:', jsonData.type, jsonData.content?.substring(0, 100));

                    switch (jsonData.type) {
                        case 'message':
                            hideTypingIndicator();
                            hideProgressBar();
                            addMessage('assistant', jsonData.content);
                            break;
                        case 'thinking':
                            // Update typing indicator with agent thinking/tool calling details
                            console.log('Thinking update:', jsonData.content);
                            updateThinkingIndicator(jsonData.content);
                            break;
                        case 'progress':
                            // Show agent progress in typing indicator, background job progress in progress bar
                            if (jsonData.content.includes('🤖') || jsonData.content.includes('🔧') ||
                                jsonData.content.includes('🧠') || jsonData.content.includes('📚') ||
                                jsonData.content.includes('💾') || jsonData.content.includes('🔮')) {
                                // Agent-related progress - update typing indicator
                                updateThinkingIndicator(jsonData.content);
                            } else {
                                // Background job progress - show in progress bar
                                updateProgressBar(jsonData);
                            }
                            break;
                        case 'tool_call':
                            showToolExecution(jsonData.details);
                            break;
                        default:
                            // Fallback for unknown JSON types
                            hideTypingIndicator();
                            addMessage('assistant', data);
                    }
                } catch (error) {
                    // Data is not JSON, treat as plain text
                    if (data.startsWith('PROGRESS:')) {
                        const progressInfo = data.substring(9);
                        // This is a legacy format, we should adapt it to the new progress bar
                        const parts = progressInfo.split('|');
                        const progressData = {
                            status: {
                                progress_percent: parseInt(parts[0]) || 0,
                                current_step: parts[1] || 'Processing...',
                            },
                            message: parts[2] || 'Please wait...',
                        };
                        updateProgressBar(progressData);
                    } else if (data.startsWith('TOOL_CALL:')) {
                        const toolInfo = data.substring(10);
                        showToolExecution(toolInfo);
                    } else {
                        hideTypingIndicator();
                        hideProgressBar();
                        addMessage('assistant', data);
                    }
                }
            };
            
            ws.onclose = function() {
                isConnected = false;
                updateConnectionStatus(false);
                console.log('Disconnected from assistant');
                
                // Try to reconnect after 3 seconds
                setTimeout(initializeWebSocket, 3000);
            };
            
            ws.onerror = function(error) {
                console.error('❌ WebSocket error:', error);
                console.error('Error details:', {
                    type: error.type,
                    target: error.target?.url || 'unknown'
                });
                hideTypingIndicator();
                updateConnectionStatus(false);
            };
            } catch (error) {
                console.error('❌ Failed to initialize WebSocket:', error);
                updateConnectionStatus(false);
            }
        }

        function setupEventListeners() {
            const chatInput = document.getElementById('chatInput');
            const sendBtn = document.getElementById('sendBtn');
            
            // Auto-resize textarea
            chatInput.addEventListener('input', function() {
                this.style.height = 'auto';
                this.style.height = Math.min(this.scrollHeight, 120) + 'px';
            });
            
            // Send on Enter, new line on Shift+Enter
            chatInput.addEventListener('keydown', function(e) {
                if (e.key === 'Enter' && !e.shiftKey) {
                    e.preventDefault();
                    sendMessage();
                }
            });
            
            // File input handler
            document.getElementById('fileInput').addEventListener('change', handleFileUpload);
        }

        function updateConnectionStatus(connected) {
            const statusDot = document.getElementById('statusDot');
            const statusText = document.getElementById('statusText');
            
            if (connected) {
                statusDot.classList.remove('disconnected');
                statusText.textContent = 'Connected';
            } else {
                statusDot.classList.add('disconnected');
                statusText.textContent = 'Disconnected';
            }
        }

        function sendMessage() {
            const input = document.getElementById('chatInput');
            const message = input.value.trim();
            
            if (!message || !isConnected) return;
            
            // Add user message to chat
            addMessage('user', message);
            
            // Clear input
            input.value = '';
            input.style.height = 'auto';
            
            // Show typing indicator
            showTypingIndicator();
            
            // Send to WebSocket
            ws.send(message);
        }

        function sendExamplePrompt(prompt) {
            document.getElementById('chatInput').value = prompt;
            sendMessage();
        }

        function addMessage(sender, content, shouldScroll = true, timestamp = null) {
            const messagesContainer = document.getElementById('chatMessages');
            const welcomeScreen = messagesContainer.querySelector('.welcome-screen');

            // Hide welcome screen on first message
            if (welcomeScreen) {
                welcomeScreen.style.display = 'none';
            }

            const messageDiv = document.createElement('div');
            messageDiv.className = `message ${sender}`;

            // Use provided timestamp or current time
            const messageTime = timestamp ? new Date(timestamp) : new Date();
            const timeString = messageTime.toLocaleTimeString([], {hour: '2-digit', minute:'2-digit'});
            
            // Process content to add download links if it's from assistant
            let processedContent = content;
            if (sender === 'assistant') {
                processedContent = parseAndRenderDownloadLinks(content);
            }
            
            messageDiv.innerHTML = `
                <div class="message-avatar">
                    ${sender === 'user' ? '👤' : '🤖'}
                </div>
                <div class="message-content">
                    ${processedContent}
                    <div class="message-time">${timeString}</div>
                </div>
            `;
            
            messagesContainer.appendChild(messageDiv);
            
            // Only scroll if requested (not when loading history)
            if (shouldScroll) {
                messagesContainer.scrollTop = messagesContainer.scrollHeight;
            }
        }
        
        function parseAndRenderDownloadLinks(content) {
            console.log('Original content:', content);

            // Parse download/stream/YouTube URLs and create clickable buttons
            const downloadRegex = /Download:\s*`([^`]+)`/g;
            const streamRegex = /Stream:\s*`([^`]+)`/g;
            const youtubeRegex = /YouTube:\s*`([^|]+)\|([^`]+)`/g;
            const fileNameRegex = /\*\*([^*]+\.mp4)\*\*/g;

            let processedContent = content;
            let fileName = '';

            // Extract filename from markdown bold syntax
            const fileMatch = fileNameRegex.exec(content);
            if (fileMatch) {
                fileName = fileMatch[1].trim();
                console.log('Extracted filename:', fileName);
            }

            // Replace download links with buttons FIRST (before converting newlines)
            processedContent = processedContent.replace(downloadRegex, (match, url) => {
                console.log('Replacing download URL:', url);
                return `<a href="${url}" download="${fileName}" class="download-button">📥 Download Video</a>`;
            });

            // Replace stream links with buttons
            processedContent = processedContent.replace(streamRegex, (match, url) => {
                console.log('Replacing stream URL:', url);
                return `<a href="${url}" target="_blank" class="stream-button">▶️ Stream Video</a>`;
            });

            // Replace YouTube links with buttons that open channel selector
            processedContent = processedContent.replace(youtubeRegex, (match, videoPath, videoName) => {
                console.log('Replacing YouTube link:', videoPath, videoName);
                return `<button onclick="openYouTubeUploadModal('${videoPath}', '${videoName}')" class="youtube-button">📺 Post to YouTube</button>`;
            });

            // Also handle the case where URLs are shown directly (not in backticks)
            processedContent = processedContent.replace(/Download:\s*(\/api\/outputs\/download\/[a-f0-9]+)/gi, (match, url) => {
                console.log('Replacing direct download URL:', url);
                return `<a href="${url}" download="${fileName}" class="download-button">📥 Download Video</a>`;
            });

            processedContent = processedContent.replace(/Stream:\s*(\/api\/outputs\/stream\/[a-f0-9]+)/gi, (match, url) => {
                console.log('Replacing direct stream URL:', url);
                return `<a href="${url}" target="_blank" class="stream-button">▶️ Stream Video</a>`;
            });

            // Format the content better with proper line breaks and styling
            processedContent = processedContent
                .replace(/\*\*(.*?)\*\*/g, '<strong>$1</strong>')
                .replace(/•/g, '<br>•')
                .replace(/\n/g, '<br>');

            console.log('Processed content:', processedContent);
            return processedContent;
        }

        // ============================================================================
        // YouTube Upload Modal Functions
        // ============================================================================

        let currentYouTubeVideo = null;

        async function openYouTubeUploadModal(videoPath, videoName) {
            currentYouTubeVideo = { path: videoPath, name: videoName };
            const modal = document.getElementById('youtubeModal');
            const content = document.getElementById('youtubeModalContent');

            modal.style.display = 'flex';
            content.innerHTML = '<p style="text-align: center; padding: 2rem; color: #6c757d;">Loading your channels...</p>';

            try {
                const authToken = localStorage.getItem('authToken');
                if (!authToken) {
                    content.innerHTML = `
                        <div style="text-align: center; padding: 2rem;">
                            <p style="color: #dc3545; margin-bottom: 1rem;">Please log in to upload to YouTube</p>
                            <button onclick="window.location.href='/login'" class="btn">Go to Login</button>
                        </div>
                    `;
                    return;
                }

                const response = await fetch('/api/youtube/channels', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();

                if (data.success && data.channels.length > 0) {
                    content.innerHTML = `
                        <div style="margin-bottom: 1.5rem;">
                            <h3 style="color: #2c3e50; margin-bottom: 1rem;">Select a channel:</h3>
                            <div id="channelList" style="display: flex; flex-direction: column; gap: 1rem;">
                                ${data.channels.map(channel => `
                                    <div onclick="selectYouTubeChannel(${channel.id}, '${channel.channel_name}')"
                                         style="display: flex; align-items: center; gap: 1rem; padding: 1rem; border: 2px solid #e9ecef; border-radius: 10px; cursor: pointer; transition: all 0.2s;"
                                         onmouseover="this.style.borderColor='#3b82f6'; this.style.background='#f8f9fa'"
                                         onmouseout="this.style.borderColor='#e9ecef'; this.style.background='white'">
                                        ${channel.channel_thumbnail_url ?
                                            `<img src="${channel.channel_thumbnail_url}" style="width: 40px; height: 40px; border-radius: 50%;" alt="${channel.channel_name}">` :
                                            '<div style="width: 40px; height: 40px; border-radius: 50%; background: linear-gradient(135deg, #FF0000, #CC0000); display: flex; align-items: center; justify-content: center; color: white; font-size: 1.2rem;">📺</div>'
                                        }
                                        <div style="flex: 1;">
                                            <div style="font-weight: 600; color: #2c3e50;">${channel.channel_name}</div>
                                            <div style="font-size: 0.85rem; color: #6c757d;">
                                                ${channel.subscriber_count !== null ? channel.subscriber_count.toLocaleString() + ' subscribers' : ''}
                                            </div>
                                        </div>
                                    </div>
                                `).join('')}
                            </div>
                        </div>
                        <div style="text-align: center; padding-top: 1rem; border-top: 1px solid #e9ecef;">
                            <a href="/youtube/manage" style="color: #3b82f6; text-decoration: none; font-weight: 500;">Manage Channels</a>
                        </div>
                    `;
                } else {
                    content.innerHTML = `
                        <div style="text-align: center; padding: 2rem;">
                            <div style="font-size: 3rem; margin-bottom: 1rem;">📺</div>
                            <h3 style="color: #2c3e50; margin-bottom: 1rem;">No YouTube Channels Connected</h3>
                            <p style="color: #6c757d; margin-bottom: 1.5rem;">Connect your YouTube channel to start uploading videos directly</p>
                            <button onclick="window.location.href='/youtube/connect?redirect_to=' + encodeURIComponent(window.location.pathname)" class="btn">Connect YouTube Channel</button>
                        </div>
                    `;
                }
            } catch (error) {
                console.error('Error loading channels:', error);
                content.innerHTML = `
                    <div style="text-align: center; padding: 2rem;">
                        <p style="color: #dc3545; margin-bottom: 1rem;">❌ Error loading channels</p>
                        <p style="color: #6c757d;">${error.message}</p>
                    </div>
                `;
            }
        }

        function closeYouTubeModal() {
            document.getElementById('youtubeModal').style.display = 'none';
            currentYouTubeVideo = null;
        }

        async function selectYouTubeChannel(channelId, channelName) {
            if (!currentYouTubeVideo) {
                alert('No video selected');
                return;
            }

            const title = prompt("Enter title for your YouTube video:", currentYouTubeVideo.name.replace('.mp4', ''));
            if (!title) return;

            const description = prompt('Enter description (optional):', 'Created with VideoSync');
            const privacyStatus = prompt('Privacy status (public/private/unlisted):', 'private');

            if (!['public', 'private', 'unlisted'].includes(privacyStatus.toLowerCase())) {
                alert('Invalid privacy status. Using "private"');
            }

            const modal = document.getElementById('youtubeModalContent');
            modal.innerHTML = `
                <div style="text-align: center; padding: 2rem;">
                    <div style="font-size: 3rem; margin-bottom: 1rem;">📤</div>
                    <p style="color: #2c3e50; font-weight: 600; margin-bottom: 0.5rem;">Uploading to YouTube...</p>
                    <p style="color: #6c757d;">This may take a few moments</p>
                </div>
            `;

            try {
                const authToken = localStorage.getItem('authToken');
                const response = await fetch('/api/youtube/upload', {
                    method: 'POST',
                    headers: {
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify({
                        channel_id: channelId,
                        video_path: currentYouTubeVideo.path,
                        title: title,
                        description: description || 'Created with VideoSync',
                        privacy_status: privacyStatus.toLowerCase(),
                        category: '22',
                        tags: ['AI', 'Video Editing', 'VideoSync']
                    })
                });

                const data = await response.json();

                if (data.success) {
                    modal.innerHTML = `
                        <div style="text-align: center; padding: 2rem;">
                            <div style="font-size: 3rem; margin-bottom: 1rem;">✅</div>
                            <h3 style="color: #28a745; margin-bottom: 1rem;">Upload Successful!</h3>
                            <p style="color: #2c3e50; margin-bottom: 1.5rem;">Your video has been uploaded to <strong>${channelName}</strong></p>
                            <a href="${data.upload.youtube_url}" target="_blank" style="display: inline-block; padding: 0.75rem 1.5rem; background: #FF0000; color: white; text-decoration: none; border-radius: 10px; font-weight: 600; margin-bottom: 1rem;">🎬 View on YouTube</a>
                            <br>
                            <button onclick="closeYouTubeModal()" style="padding: 0.5rem 1.5rem; background: #6c757d; color: white; border: none; border-radius: 10px; cursor: pointer;">Close</button>
                        </div>
                    `;
                } else {
                    modal.innerHTML = `
                        <div style="text-align: center; padding: 2rem;">
                            <div style="font-size: 3rem; margin-bottom: 1rem;">❌</div>
                            <h3 style="color: #dc3545; margin-bottom: 1rem;">Upload Failed</h3>
                            <p style="color: #6c757d; margin-bottom: 1.5rem;">${data.message}</p>
                            <button onclick="closeYouTubeModal()" style="padding: 0.75rem 1.5rem; background: #3b82f6; color: white; border: none; border-radius: 10px; cursor: pointer;">Close</button>
                        </div>
                    `;
                }
            } catch (error) {
                console.error('YouTube upload error:', error);
                modal.innerHTML = `
                    <div style="text-align: center; padding: 2rem;">
                        <div style="font-size: 3rem; margin-bottom: 1rem;">❌</div>
                        <h3 style="color: #dc3545; margin-bottom: 1rem;">Upload Error</h3>
                        <p style="color: #6c757d; margin-bottom: 1.5rem;">${error.message}</p>
                        <button onclick="closeYouTubeModal()" style="padding: 0.75rem 1.5rem; background: #3b82f6; color: white; border: none; border-radius: 10px; cursor: pointer;">Close</button>
                    </div>
                `;
            }
        }

        function showTypingIndicator() {
            document.getElementById('typingIndicator').classList.add('show');
            const messagesContainer = document.getElementById('chatMessages');
            messagesContainer.scrollTop = messagesContainer.scrollHeight;
        }

        function hideTypingIndicator() {
            document.getElementById('typingIndicator').classList.remove('show');
            // Reset to default text
            document.getElementById('thinkingText').textContent = 'AI is thinking';
        }

        function updateThinkingIndicator(message) {
            // Show the typing indicator if not already visible
            const indicator = document.getElementById('typingIndicator');
            if (!indicator.classList.contains('show')) {
                indicator.classList.add('show');
            }

            // Update the thinking text with real-time agent progress
            document.getElementById('thinkingText').textContent = message;

            // Auto-scroll to show the updated thinking message
            const messagesContainer = document.getElementById('chatMessages');
            messagesContainer.scrollTop = messagesContainer.scrollHeight;
        }

        function updateProgressBar(progressData) {
            const container = document.getElementById('progressContainer');
            const bar = document.getElementById('progressBar');
            const title = document.getElementById('progressTitle');
            const text = document.getElementById('progressText');

            console.log('Progress update:', progressData); // Debug

            // Check if job completed (lowercase due to serde rename_all)
            if (progressData.status && progressData.status.status === 'completed') {
                hideProgressBar();
                hideTypingIndicator();
                // Add completion message to chat
                const result = progressData.status.result || progressData.message;
                addMessage('assistant', result);
                return;
            }

            // Check if job failed (lowercase due to serde rename_all)
            if (progressData.status && progressData.status.status === 'failed') {
                hideProgressBar();
                hideTypingIndicator();
                const error = progressData.status.error || progressData.message;
                addMessage('assistant', '❌ ' + error);
                return;
            }

            // Running status
            if (progressData.status && progressData.status.status === 'running') {
                const percentage = progressData.status.progress_percent || 0;
                const progressTitle = progressData.status.current_step || 'Processing video...';
                const progressDesc = progressData.message || 'Please wait...';

            // Show progress container
            container.classList.add('show');

            // Update content
            title.textContent = progressTitle;
            text.textContent = `${progressDesc} (${percentage.toFixed(1)}%)`;
            bar.style.width = `${percentage}%`;

            // Hide after 100%
            if (percentage >= 100) {
                setTimeout(() => {
                    hideProgressBar();
                }, 3000);
            }
            }
        }

        function hideProgressBar() {
            const container = document.getElementById('progressContainer');
            container.classList.remove('show');
        }
        
        function showToolExecution(toolInfo) {
            const messagesContainer = document.getElementById('chatMessages');
            
            // Parse tool info (expected format: "toolName|parameters")
            const parts = toolInfo.split('|');
            const toolName = parts[0] || 'Processing';
            const parameters = parts[1] || '';
            
            const toolDiv = document.createElement('div');
            toolDiv.className = 'tool-execution';
            toolDiv.innerHTML = `
                <div class="tool-execution-title">⚡ Executing: ${toolName}</div>
                <div class="tool-execution-details">${parameters}</div>
            `;
            
            messagesContainer.appendChild(toolDiv);
            messagesContainer.scrollTop = messagesContainer.scrollHeight;
        }

        function uploadFiles() {
            document.getElementById('fileInput').click();
        }

        async function handleFileUpload(event) {
            const files = event.target.files;
            if (files.length === 0) return;
            
            // Get JWT token from localStorage
            const authToken = localStorage.getItem('authToken');
            if (!authToken) {
                addMessage('assistant', '❌ Please log in to upload files.');
                return;
            }
            
            const formData = new FormData();
            for (let file of files) {
                formData.append('files', file);
            }
            
            try {
                const response = await fetch(`/upload/session/${sessionUuid}`, {
                    method: 'POST',
                    headers: {
                        'Authorization': `Bearer ${authToken}`
                    },
                    body: formData
                });
                
                const result = await response.json();
                
                if (result.success) {
                    uploadedFiles = [...uploadedFiles, ...result.files];
                    updateFileList();
                    addMessage('assistant', `✅ Successfully uploaded ${result.files.length} files: ${result.files.map(f => f.original_name).join(', ')}`);
                } else {
                    addMessage('assistant', '❌ Failed to upload files. Please try again.');
                }
            } catch (error) {
                console.error('Upload error:', error);
                addMessage('assistant', '❌ Error uploading files: ' + error.message);
            }
            
            // Clear the file input
            event.target.value = '';
        }

        function updateFileList() {
            const fileList = document.getElementById('fileList');
            
            if (uploadedFiles.length === 0) {
                fileList.innerHTML = `
                    <div class="file-item" style="opacity: 0.5;">
                        <div class="file-name">No files uploaded yet</div>
                        <div class="file-meta">Upload files to get started</div>
                    </div>
                `;
                return;
            }
            
            fileList.innerHTML = uploadedFiles.map(file => `
                <div class="file-item" onclick="selectFile('${file.id}')">
                    <div class="file-name">${file.original_name}</div>
                    <div class="file-meta">${(file.file_size / 1024 / 1024).toFixed(2)} MB • ${file.file_type}</div>
                </div>
            `).join('');
        }

        function selectFile(fileId) {
            const file = uploadedFiles.find(f => f.id === fileId);
            if (file) {
                const input = document.getElementById('chatInput');
                input.value = `Please work with my file: ${file.original_name}`;
                input.focus();
            }
        }

        async function loadUploadedFiles() {
            // Get JWT token from localStorage
            const authToken = localStorage.getItem('authToken');
            if (!authToken) {
                return; // No auth token, can't load files
            }
            
            try {
                const response = await fetch(`/files/session/${sessionUuid}`, {
                    method: 'GET',
                    headers: {
                        'Authorization': `Bearer ${authToken}`
                    }
                });
                
                if (response.ok) {
                    const result = await response.json();
                    if (result.success && result.files) {
                        uploadedFiles = result.files;
                        updateFileList();
                    }
                }
            } catch (error) {
                console.error('Error loading uploaded files:', error);
            }
        }

        // Dynamic Background Management for Chat Interface
        class ChatDynamicBackgroundManager {
            constructor() {
                this.lastBackgroundUpdate = Date.now();
                this.updateInterval = 5 * 60 * 1000; // 5 minutes
                this.retryDelay = 30 * 1000; // 30 seconds on error
                this.isUpdating = false;
                
                this.init();
            }

            async init() {
                // Load initial background
                await this.updateBackground();
                
                // Set up periodic updates
                setInterval(() => {
                    this.checkAndUpdateBackground();
                }, 60 * 1000); // Check every minute
            }

            async checkAndUpdateBackground() {
                if (this.isUpdating) return;
                
                const timeSinceLastUpdate = Date.now() - this.lastBackgroundUpdate;
                if (timeSinceLastUpdate >= this.updateInterval) {
                    await this.updateBackground();
                }
            }

            async updateBackground() {
                if (this.isUpdating) return;
                
                this.isUpdating = true;
                
                try {
                    const response = await fetch('/api/background/image');
                    
                    if (response.ok) {
                        const contentType = response.headers.get('content-type');
                        
                        if (contentType && contentType.includes('application/json')) {
                            // Fallback gradient
                            const data = await response.json();
                            if (data.fallback && data.gradient) {
                                document.body.style.background = data.gradient;
                            }
                        } else {
                            // Image response
                            const blob = await response.blob();
                            const imageUrl = URL.createObjectURL(blob);
                            
                            // Create overlay for smooth transition
                            const overlay = document.createElement('div');
                            overlay.style.cssText = `
                                position: fixed;
                                top: 0;
                                left: 0;
                                width: 100%;
                                height: 100%;
                                background-image: url(${imageUrl});
                                background-size: cover;
                                background-position: center;
                                background-attachment: fixed;
                                opacity: 0;
                                transition: opacity 1s ease-in-out;
                                z-index: -1;
                                pointer-events: none;
                            `;
                            
                            document.body.appendChild(overlay);
                            
                            // Trigger fade in with moderate opacity for chat interface
                            setTimeout(() => {
                                overlay.style.opacity = '0.35'; // Visible but not distracting
                            }, 100);
                            
                            // Clean up old overlays after transition
                            setTimeout(() => {
                                const oldOverlays = document.querySelectorAll('div[style*="background-image"]');
                                oldOverlays.forEach((old, index) => {
                                    if (index < oldOverlays.length - 1) {
                                        old.remove();
                                    }
                                });
                            }, 1100);
                        }
                        
                        this.lastBackgroundUpdate = Date.now();
                    }
                } catch (error) {
                    console.error('Error updating chat background:', error);
                    setTimeout(() => {
                        this.lastBackgroundUpdate = Date.now() - this.updateInterval + this.retryDelay;
                    }, this.retryDelay);
                } finally {
                    this.isUpdating = false;
                }
            }
        }

        // Initialize dynamic background manager for chat
        document.addEventListener('DOMContentLoaded', () => {
            new ChatDynamicBackgroundManager();
        });
    </script>
</body>
</html>
    "###;
    
    // Replace the session ID placeholder with the actual value
    let html = html.replace("SESSION_ID_PLACEHOLDER", &session_id_js);
    
    Html(html)
}

// ============================================================================
// Analytics Dashboard Page (with dark theme + dynamic background)
// ============================================================================

pub async fn analytics_dashboard_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>📊 Analytics - VideoSync</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            background-size: cover;
            background-attachment: fixed;
            transition: background-image 1s ease-in-out;
            min-height: 100vh;
            color: #e8e8e8;
            padding: 20px;
        }

        .container {
            max-width: 1400px;
            margin: 0 auto;
            background: rgba(26, 26, 46, 0.95);
            backdrop-filter: blur(20px);
            border-radius: 20px;
            padding: 40px;
            box-shadow: 0 20px 60px rgba(0,0,0,0.5);
            border: 1px solid rgba(59, 130, 246, 0.3);
        }

        .nav-link {
            color: #3b82f6;
            text-decoration: none;
            font-weight: 600;
            margin-bottom: 20px;
            display: inline-block;
        }

        h1 { color: #fff; font-size: 2.5rem; margin-bottom: 10px; }
        .subtitle { color: #bdc3c7; margin-bottom: 30px; }

        .info-box {
            background: rgba(59, 130, 246, 0.1);
            border-left: 4px solid #3b82f6;
            padding: 20px;
            border-radius: 10px;
            margin: 20px 0;
        }

        .info-box h3 { color: #fff; margin-bottom: 15px; }
        .info-box p { color: #bdc3c7; line-height: 1.6; margin-bottom: 10px; }
        .info-box ol { margin-left: 20px; margin-top: 10px; color: #bdc3c7; }

        .feature-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 20px;
            margin-top: 30px;
        }

        .feature-card {
            background: rgba(255,255,255,0.05);
            padding: 20px;
            border-radius: 12px;
            border: 1px solid rgba(59, 130, 246, 0.2);
            transition: transform 0.2s, border-color 0.2s;
        }

        .feature-card:hover {
            transform: translateY(-5px);
            border-color: rgba(59, 130, 246, 0.5);
        }

        .feature-card h4 { color: #3b82f6; margin-bottom: 10px; font-size: 1.2rem; }
        .feature-card ul { margin-left: 20px; margin-top: 10px; color: #bdc3c7; }

        .btn {
            display: inline-block;
            background: linear-gradient(135deg, #3b82f6, #1d4ed8);
            color: white;
            padding: 12px 24px;
            border-radius: 25px;
            text-decoration: none;
            font-weight: 600;
            margin: 10px 5px;
            transition: all 0.3s;
            border: 1px solid rgba(59, 130, 246, 0.3);
        }

        .btn:hover {
            transform: translateY(-2px);
            box-shadow: 0 10px 20px rgba(59, 130, 246, 0.3);
        }

        .coming-soon {
            background: rgba(255, 193, 7, 0.1);
            border-left: 4px solid #ffc107;
            padding: 15px;
            border-radius: 8px;
            margin-top: 20px;
            color: #ffc107;
        }
    </style>
</head>
<body>
    <div class="container">
        <a href="/dashboard" class="nav-link">← Back to Dashboard</a>
        <h1>📊 Analytics Dashboard</h1>
        <p class="subtitle">YouTube Channel Performance & Video Analytics</p>

        <div class="info-box">
            <h3>🚀 Connect YouTube First</h3>
            <p>Connect your YouTube channel with analytics permissions to unlock performance insights.</p>
            <ol>
                <li>Click "Connect YouTube Channel"</li>
                <li>Grant <strong>YouTube Analytics</strong> permission</li>
                <li>Return here to view analytics</li>
            </ol>
            <a href="/youtube/connect" class="btn">📺 Connect YouTube Channel</a>
        </div>

        <div class="feature-grid">
            <div class="feature-card">
                <h4>📹 Video Analytics</h4>
                <ul>
                    <li>Views and watch time</li>
                    <li>Engagement metrics</li>
                    <li>Subscriber growth</li>
                    <li>Average view duration</li>
                </ul>
            </div>
            <div class="feature-card">
                <h4>📺 Channel Analytics</h4>
                <ul>
                    <li>Total views/subscribers</li>
                    <li>Revenue estimates</li>
                    <li>Demographics</li>
                    <li>Traffic sources</li>
                </ul>
            </div>
            <div class="feature-card">
                <h4>📊 Real-Time Stats</h4>
                <ul>
                    <li>Current view counts</li>
                    <li>Live engagement</li>
                    <li>Recent comments</li>
                    <li>Trending status</li>
                </ul>
            </div>
            <div class="feature-card">
                <h4>🤖 AI Insights</h4>
                <ul>
                    <li>Performance analysis</li>
                    <li>Optimization tips</li>
                    <li>Content suggestions</li>
                    <li>Competitor research</li>
                </ul>
            </div>
        </div>

        <div class="coming-soon">
            <strong>🔧 Coming Soon:</strong> Interactive charts, date ranges, export reports
        </div>

        <div style="margin-top: 30px; text-align: center;">
            <a href="/youtube/manage" class="btn">Manage Channels</a>
            <a href="/dashboard" class="btn">Dashboard</a>
        </div>
    </div>

    <script>
        class DynamicBackgroundManager {
            constructor() {
                this.updateBackground();
                setInterval(() => this.updateBackground(), 5 * 60 * 1000);
            }

            async updateBackground() {
                try {
                    const response = await fetch('/api/background/image');
                    if (response.ok) {
                        const blob = await response.blob();
                        const url = URL.createObjectURL(blob);
                        const overlay = document.createElement('div');
                        overlay.style.cssText = `position:fixed;top:0;left:0;width:100%;height:100%;background-image:url(${url});background-size:cover;background-position:center;opacity:0;transition:opacity 1s;z-index:-1;pointer-events:none`;
                        document.body.appendChild(overlay);
                        setTimeout(() => overlay.style.opacity = '0.3', 100);
                        setTimeout(() => {
                            const old = document.querySelectorAll('div[style*="background-image"]');
                            old.forEach((o, i) => { if (i < old.length - 1) o.remove(); });
                        }, 1100);
                    }
                } catch (e) { console.error('Background error:', e); }
            }
        }
        new DynamicBackgroundManager();
    </script>
</body>
</html>
    "###;
    Html(html.to_string())
}

// ============================================================================
// Help & Guide Page (with dark theme + dynamic background)
// ============================================================================

pub async fn help_guide_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>📖 Help - VideoSync</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            background-attachment: fixed;
            transition: background-image 1s ease-in-out;
            min-height: 100vh;
            color: #e8e8e8;
            padding: 20px;
        }

        .container {
            max-width: 1200px;
            margin: 0 auto;
            background: rgba(26, 26, 46, 0.95);
            backdrop-filter: blur(20px);
            border-radius: 20px;
            padding: 40px;
            box-shadow: 0 20px 60px rgba(0,0,0,0.5);
            border: 1px solid rgba(59, 130, 246, 0.3);
        }

        .nav-link { color: #3b82f6; text-decoration: none; font-weight: 600; display: inline-block; margin-bottom: 20px; }
        h1 { color: #fff; font-size: 2.5rem; margin-bottom: 10px; }
        .subtitle { color: #bdc3c7; margin-bottom: 30px; }

        .toc {
            background: rgba(255,255,255,0.05);
            padding: 20px;
            border-radius: 10px;
            margin-bottom: 30px;
            border: 1px solid rgba(59, 130, 246, 0.2);
        }

        .toc h3 { color: #3b82f6; margin-bottom: 15px; }
        .toc a { display: block; color: #3b82f6; text-decoration: none; padding: 5px 0; transition: padding-left 0.2s; }
        .toc a:hover { padding-left: 10px; }

        .section { margin-bottom: 40px; }
        .section h2 { color: #3b82f6; font-size: 1.8rem; margin-bottom: 15px; padding-bottom: 10px; border-bottom: 2px solid rgba(59, 130, 246, 0.3); }
        .section h3 { color: #fff; font-size: 1.3rem; margin: 20px 0 10px; }
        .section p { color: #bdc3c7; line-height: 1.8; margin-bottom: 15px; }
        .section ul, .section ol { margin-left: 30px; color: #bdc3c7; line-height: 1.8; }
        .section li { margin-bottom: 10px; }

        .example-box {
            background: rgba(59, 130, 246, 0.1);
            border-left: 4px solid #3b82f6;
            padding: 15px;
            border-radius: 5px;
            margin: 15px 0;
            font-family: monospace;
            color: #e8e8e8;
        }

        .warning-box {
            background: rgba(255, 193, 7, 0.1);
            border-left: 4px solid #ffc107;
            padding: 15px;
            border-radius: 5px;
            margin: 15px 0;
            color: #ffc107;
        }

        .success-box {
            background: rgba(40, 167, 69, 0.1);
            border-left: 4px solid #28a745;
            padding: 15px;
            border-radius: 5px;
            margin: 15px 0;
            color: #28a745;
        }

        .btn {
            display: inline-block;
            background: linear-gradient(135deg, #3b82f6, #1d4ed8);
            color: white;
            padding: 15px 30px;
            border-radius: 25px;
            text-decoration: none;
            font-weight: 600;
            margin: 5px;
            transition: all 0.3s;
        }

        .btn:hover {
            transform: translateY(-2px);
            box-shadow: 0 10px 20px rgba(59, 130, 246, 0.3);
        }
    </style>
</head>
<body>
    <div class="container">
        <a href="/dashboard" class="nav-link">← Dashboard</a>
        <h1>📖 Help & User Guide</h1>
        <p class="subtitle">Complete guide to AI-powered video editing</p>

        <div class="toc">
            <h3>📑 Quick Links</h3>
            <a href="#start">Getting Started</a>
            <a href="#chat">AI Chat Commands</a>
            <a href="#edit">Video Editing</a>
            <a href="#youtube">YouTube Integration</a>
            <a href="#ai">AI Tools</a>
            <a href="#trouble">Troubleshooting</a>
        </div>

        <div class="section" id="start">
            <h2>1. Getting Started</h2>
            <h3>First Video Edit</h3>
            <ol>
                <li>Click <strong>Start New Chat</strong></li>
                <li>Upload video file (📎 button)</li>
                <li>Tell AI what you want</li>
                <li>Download result!</li>
            </ol>
            <div class="example-box">
"Make this black and white and add text Epic at 5 seconds"
            </div>
        </div>

        <div class="section" id="chat">
            <h2>2. AI Chat Commands</h2>
            <div class="example-box">
✂️ "Trim from 10 to 30 seconds"<br>
🎨 "Apply vintage filter"<br>
📝 "Add subtitles: Hello World"<br>
🎬 "Merge video1.mp4 and video2.mp4"<br>
📺 "Export for YouTube"
            </div>
        </div>

        <div class="section" id="edit">
            <h2>3. Editing Features</h2>
            <h3>Core</h3>
            <ul>
                <li>Trim, Merge, Split, Resize, Crop, Rotate</li>
            </ul>
            <h3>Effects</h3>
            <ul>
                <li>Filters, Text, Colors, Subtitles</li>
            </ul>
            <h3>Audio</h3>
            <ul>
                <li>Extract, Mix, Volume, AI Voiceover (17+ voices), AI Music</li>
            </ul>
        </div>

        <div class="section" id="youtube">
            <h2>4. YouTube Integration</h2>
            <h3>Connect Channel</h3>
            <ol>
                <li>Go to <strong>Connect YouTube Channels</strong></li>
                <li>Sign in with Google</li>
                <li>Grant permissions</li>
            </ol>
            <h3>Features</h3>
            <ul>
                <li>Upload, Update metadata, Delete videos</li>
                <li>Custom thumbnails, Playlists</li>
                <li>Analytics, Comments, Captions</li>
            </ul>
            <div class="example-box">
AI YouTube Tools:<br>
"Optimize metadata for YouTube gaming"<br>
"What's trending in tech?"<br>
"Search for cooking channels"
            </div>
        </div>

        <div class="section" id="ai">
            <h2>5. AI Tools</h2>
            <ul>
                <li><strong>Stock Media:</strong> Free videos/photos from Pexels</li>
                <li><strong>TTS:</strong> 17+ natural voices (75ms latency)</li>
                <li><strong>Music:</strong> Studio-quality background tracks</li>
                <li><strong>Sound FX:</strong> Custom sound effects</li>
            </ul>
        </div>

        <div class="section" id="trouble">
            <h2>6. Troubleshooting</h2>
            <div class="warning-box">
                <strong>Stuck "Connecting":</strong> Hard refresh (Ctrl+Shift+R)
            </div>
            <div class="warning-box">
                <strong>YouTube Permissions:</strong> Reconnect at /youtube/connect
            </div>
            <div class="success-box">
                <strong>Pro Tip:</strong> Be specific with commands for best results!
            </div>
        </div>

        <div style="text-align: center; margin-top: 40px;">
            <a href="/chat" class="btn">🎬 Start Editing</a>
            <a href="/dashboard" class="btn">📊 Dashboard</a>
        </div>
    </div>

    <script>
        class DynamicBackgroundManager {
            constructor() {
                this.updateBackground();
                setInterval(() => this.updateBackground(), 5 * 60 * 1000);
            }
            async updateBackground() {
                try {
                    const r = await fetch('/api/background/image');
                    if (r.ok) {
                        const blob = await r.blob();
                        const url = URL.createObjectURL(blob);
                        const o = document.createElement('div');
                        o.style.cssText = `position:fixed;top:0;left:0;width:100%;height:100%;background-image:url(${url});background-size:cover;background-position:center;opacity:0;transition:opacity 1s;z-index:-1;pointer-events:none`;
                        document.body.appendChild(o);
                        setTimeout(() => o.style.opacity = '0.3', 100);
                        setTimeout(() => {
                            const old = document.querySelectorAll('div[style*="background-image"]');
                            old.forEach((e, i) => { if (i < old.length - 1) e.remove(); });
                        }, 1100);
                    }
                } catch (e) { console.error(e); }
            }
        }
        new DynamicBackgroundManager();
    </script>
</body>
</html>
    "###;
    Html(html.to_string())
}

// ============================================================================
// Privacy Policy Page
// ============================================================================

pub async fn privacy_policy_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>Privacy Policy - VideoSync</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            background-attachment: fixed;
            transition: background-image 1s;
            min-height: 100vh;
            color: #e8e8e8;
            padding: 20px;
            line-height: 1.6;
        }
        .container {
            max-width: 900px;
            margin: 0 auto;
            background: rgba(26, 26, 46, 0.95);
            backdrop-filter: blur(20px);
            border-radius: 20px;
            padding: 40px;
            box-shadow: 0 20px 60px rgba(0,0,0,0.5);
            border: 1px solid rgba(59, 130, 246, 0.3);
        }
        .nav-link { color: #3b82f6; text-decoration: none; font-weight: 600; display: inline-block; margin-bottom: 20px; }
        h1 { color: #fff; font-size: 2.5rem; margin-bottom: 10px; }
        h2 { color: #3b82f6; font-size: 1.8rem; margin: 30px 0 15px; padding-bottom: 10px; border-bottom: 2px solid rgba(59, 130, 246, 0.3); }
        h3 { color: #fff; font-size: 1.3rem; margin: 20px 0 10px; }
        p { color: #bdc3c7; margin-bottom: 15px; }
        ul, ol { margin-left: 30px; color: #bdc3c7; margin-bottom: 15px; }
        li { margin-bottom: 8px; }
        .date { color: #7f8c8d; font-size: 0.9rem; margin-bottom: 30px; }
        .section { margin-bottom: 30px; }
        strong { color: #fff; }
        a { color: #3b82f6; text-decoration: none; }
        a:hover { text-decoration: underline; }
        .highlight { background: rgba(59, 130, 246, 0.1); padding: 15px; border-left: 4px solid #3b82f6; border-radius: 5px; margin: 15px 0; }
    </style>
</head>
<body>
    <div class="container">
        <a href="/" class="nav-link">← Home</a>
        <h1>Privacy Policy</h1>
        <p class="date">Last Updated: December 23, 2025</p>

        <div class="section">
            <h2>Introduction</h2>
            <p>Agentic Video Editor ("VideoSync") operates an AI-powered video editing platform. This Privacy Policy explains how we collect, use, and protect your information.</p>
        </div>

        <div class="section">
            <h2>Information We Collect</h2>
            
            <h3>1. Account Information</h3>
            <ul>
                <li>Email address and username</li>
                <li>Password (encrypted)</li>
                <li>Profile information (if using Google sign-in)</li>
            </ul>

            <h3>2. YouTube Data (When Connected)</h3>
            <ul>
                <li>Channel name and statistics</li>
                <li>Video metadata (titles, descriptions, tags)</li>
                <li>Analytics data (views, engagement)</li>
                <li>OAuth tokens (encrypted)</li>
            </ul>

            <h3>3. Video Content</h3>
            <ul>
                <li>Videos you upload for editing</li>
                <li>Edited video outputs</li>
                <li>AI-generated content</li>
            </ul>
        </div>

        <div class="section">
            <h2>How We Use Your Information</h2>
            <ul>
                <li><strong>Video Editing</strong> - Process and edit your videos using AI</li>
                <li><strong>YouTube Integration</strong> - Upload and manage your YouTube content</li>
                <li><strong>AI Assistance</strong> - Generate metadata, voiceovers, and insights</li>
                <li><strong>Analytics</strong> - Show performance data from YouTube</li>
                <li><strong>Security</strong> - Prevent fraud and unauthorized access</li>
            </ul>
        </div>

        <div class="section">
            <h2>Third-Party Services</h2>
            <p>We integrate with:</p>
            <ul>
                <li><strong>Google</strong> - YouTube API, Gemini AI, OAuth</li>
                <li><strong>Anthropic</strong> - Claude AI for video editing</li>
                <li><strong>Eleven Labs</strong> - Voice and audio generation</li>
                <li><strong>Pexels</strong> - Stock media library</li>
            </ul>
        </div>

        <div class="section">
            <h2>YouTube Data Usage</h2>
            <div class="highlight">
                <p><strong>Important:</strong> We access YouTube data solely to provide features you request. You can revoke access anytime via <a href="https://myaccount.google.com/permissions" target="_blank">Google Account Permissions</a>.</p>
            </div>
            <p><strong>What we access:</strong> Channel info, video metadata, analytics, comments, playlists</p>
            <p><strong>What we DON'T do:</strong> We do NOT download other users' videos or share your data</p>
        </div>

        <div class="section">
            <h2>Data Security</h2>
            <ul>
                <li>✅ Encryption in transit (HTTPS/TLS)</li>
                <li>✅ Encrypted storage for OAuth tokens</li>
                <li>✅ Password hashing with bcrypt</li>
                <li>✅ JWT authentication</li>
                <li>✅ Rate limiting protection</li>
            </ul>
        </div>

        <div class="section">
            <h2>Your Rights</h2>
            <ul>
                <li><strong>Access</strong> - View all data we store about you</li>
                <li><strong>Delete</strong> - Request account and data deletion</li>
                <li><strong>Export</strong> - Download your videos and data</li>
                <li><strong>Disconnect</strong> - Revoke YouTube access anytime</li>
            </ul>
        </div>

        <div class="section">
            <h2>Data Retention</h2>
            <ul>
                <li>Active accounts: Data retained while account is active</li>
                <li>Temporary files: Deleted after 30 days</li>
                <li>Deleted accounts: All data purged within 30 days</li>
                <li>Analytics cache: 24 hours</li>
            </ul>
        </div>

        <div class="section">
            <h2>Contact Us</h2>
            <p>Questions about this Privacy Policy? Contact us at:</p>
            <p><strong>Email:</strong> support@yourapp.com</p>
            <p><strong>For data deletion:</strong> privacy@yourapp.com</p>
        </div>

        <div class="section">
            <h2>Compliance</h2>
            <p>This Privacy Policy complies with:</p>
            <ul>
                <li>General Data Protection Regulation (GDPR)</li>
                <li>California Consumer Privacy Act (CCPA)</li>
                <li><a href="https://developers.google.com/youtube/terms/api-services-terms-of-service" target="_blank">YouTube API Services Terms</a></li>
                <li><a href="https://developers.google.com/terms/api-services-user-data-policy" target="_blank">Google API Services User Data Policy</a></li>
            </ul>
        </div>

        <div style="text-align: center; margin-top: 40px; padding-top: 20px; border-top: 1px solid rgba(59, 130, 246, 0.3);">
            <a href="/" style="display: inline-block; background: linear-gradient(135deg, #3b82f6, #1d4ed8); color: white; padding: 12px 24px; border-radius: 25px; text-decoration: none; margin: 5px;">← Back to Home</a>
            <a href="/terms" style="display: inline-block; background: #6c757d; color: white; padding: 12px 24px; border-radius: 25px; text-decoration: none; margin: 5px;">View Terms of Service</a>
        </div>
    </div>

    <script>
        class DynamicBackgroundManager {
            constructor() { this.updateBackground(); setInterval(() => this.updateBackground(), 5*60*1000); }
            async updateBackground() {
                try {
                    const r = await fetch('/api/background/image');
                    if (r.ok) {
                        const blob = await r.blob();
                        const url = URL.createObjectURL(blob);
                        const o = document.createElement('div');
                        o.style.cssText = `position:fixed;top:0;left:0;width:100%;height:100%;background-image:url(${url});background-size:cover;background-position:center;opacity:0;transition:opacity 1s;z-index:-1;pointer-events:none`;
                        document.body.appendChild(o);
                        setTimeout(() => o.style.opacity = '0.3', 100);
                        setTimeout(() => {
                            const old = document.querySelectorAll('div[style*="background-image"]');
                            old.forEach((e, i) => { if (i < old.length - 1) e.remove(); });
                        }, 1100);
                    }
                } catch (e) { console.error(e); }
            }
        }
        new DynamicBackgroundManager();
    </script>
</body>
</html>
    "###;
    Html(html.to_string())
}

// ============================================================================
// Terms of Service Page
// ============================================================================

pub async fn terms_of_service_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <title>Terms of Service - VideoSync</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            background-attachment: fixed;
            transition: background-image 1s;
            min-height: 100vh;
            color: #e8e8e8;
            padding: 20px;
            line-height: 1.6;
        }
        .container {
            max-width: 900px;
            margin: 0 auto;
            background: rgba(26, 26, 46, 0.95);
            backdrop-filter: blur(20px);
            border-radius: 20px;
            padding: 40px;
            box-shadow: 0 20px 60px rgba(0,0,0,0.5);
            border: 1px solid rgba(59, 130, 246, 0.3);
        }
        .nav-link { color: #3b82f6; text-decoration: none; font-weight: 600; display: inline-block; margin-bottom: 20px; }
        h1 { color: #fff; font-size: 2.5rem; margin-bottom: 10px; }
        h2 { color: #3b82f6; font-size: 1.8rem; margin: 30px 0 15px; padding-bottom: 10px; border-bottom: 2px solid rgba(59, 130, 246, 0.3); }
        h3 { color: #fff; font-size: 1.3rem; margin: 20px 0 10px; }
        p { color: #bdc3c7; margin-bottom: 15px; }
        ul, ol { margin-left: 30px; color: #bdc3c7; margin-bottom: 15px; }
        li { margin-bottom: 8px; }
        .date { color: #7f8c8d; font-size: 0.9rem; margin-bottom: 30px; }
        .section { margin-bottom: 30px; }
        strong { color: #fff; }
        a { color: #3b82f6; text-decoration: none; }
        a:hover { text-decoration: underline; }
        .important { background: rgba(255, 193, 7, 0.1); padding: 15px; border-left: 4px solid #ffc107; border-radius: 5px; margin: 15px 0; color: #ffc107; }
        .highlight { background: rgba(59, 130, 246, 0.1); padding: 15px; border-left: 4px solid #3b82f6; border-radius: 5px; margin: 15px 0; }
    </style>
</head>
<body>
    <div class="container">
        <a href="/" class="nav-link">← Home</a>
        <h1>Terms of Service</h1>
        <p class="date">Last Updated: December 23, 2025</p>

        <div class="section">
            <h2>1. Acceptance of Terms</h2>
            <p>By using VideoSync, you agree to these Terms. If you don't agree, please don't use the Service.</p>
        </div>

        <div class="section">
            <h2>2. Service Description</h2>
            <p>VideoSync is an AI-powered video editing platform that enables you to:</p>
            <ul>
                <li>Edit videos using natural language commands</li>
                <li>Upload and manage YouTube videos</li>
                <li>Generate AI-powered content (voiceovers, music, thumbnails)</li>
                <li>Analyze YouTube channel performance</li>
                <li>Access stock media from Pexels</li>
            </ul>
        </div>

        <div class="section">
            <h2>3. User Accounts</h2>
            <h3>Account Creation</h3>
            <ul>
                <li>Provide accurate information</li>
                <li>Must be at least 13 years old</li>
                <li>Keep password secure</li>
                <li>One account per person</li>
            </ul>

            <h3>YouTube Connection</h3>
            <ul>
                <li>Optional - not required for video editing</li>
                <li>Connect multiple YouTube channels from different Google accounts</li>
                <li>Disconnect anytime</li>
                <li>We don't own your YouTube channels</li>
            </ul>
        </div>

        <div class="section">
            <h2>4. Acceptable Use</h2>
            
            <h3>You MAY:</h3>
            <ul>
                <li>✅ Edit your own videos</li>
                <li>✅ Upload to your YouTube channels</li>
                <li>✅ Use AI-generated content</li>
                <li>✅ Access stock media</li>
                <li>✅ Analyze your analytics</li>
            </ul>

            <h3>You MAY NOT:</h3>
            <ul>
                <li>❌ Upload copyrighted content without permission</li>
                <li>❌ Use for illegal activities</li>
                <li>❌ Harass or abuse others</li>
                <li>❌ Upload malware or viruses</li>
                <li>❌ Spam or send unsolicited messages</li>
                <li>❌ Download other users' YouTube videos</li>
            </ul>
        </div>

        <div class="section">
            <h2>5. Content Ownership</h2>
            <p><strong>Your Content:</strong> You retain full ownership of all videos you upload.</p>
            <p><strong>Our License:</strong> You grant us a limited license to process your content for editing purposes only.</p>
            <p><strong>AI-Generated Content:</strong> Provided "as-is" - you're responsible for ensuring it complies with laws.</p>
        </div>

        <div class="section">
            <h2>6. YouTube Integration</h2>
            <div class="highlight">
                <p>By using YouTube features, you agree to:</p>
                <ul style="margin-top: 10px;">
                    <li><a href="https://www.youtube.com/t/terms" target="_blank">YouTube Terms of Service</a></li>
                    <li><a href="https://developers.google.com/youtube/terms/api-services-terms-of-service" target="_blank">YouTube API Services Terms</a></li>
                    <li><a href="https://policies.google.com/privacy" target="_blank">Google Privacy Policy</a></li>
                </ul>
            </div>

            <h3>YouTube Permissions</h3>
            <p>We request:</p>
            <ul>
                <li><strong>youtube.upload</strong> - Upload videos to your channel</li>
                <li><strong>youtube.readonly</strong> - Read channel information</li>
                <li><strong>youtube.force-ssl</strong> - Modify videos, playlists, comments</li>
                <li><strong>yt-analytics.readonly</strong> - Access analytics</li>
            </ul>
            <p>You can revoke access anytime at <a href="https://myaccount.google.com/permissions" target="_blank">Google Account Permissions</a>.</p>
        </div>

        <div class="section">
            <h2>7. Service Limitations</h2>
            <ul>
                <li>Service provided "as-is" and "as available"</li>
                <li>File size limit: 500MB per video</li>
                <li>YouTube API has daily quota limits</li>
                <li>Scheduled maintenance may occur</li>
            </ul>
        </div>

        <div class="section">
            <h2>8. Prohibited Content</h2>
            <p>You may not upload content that:</p>
            <ul>
                <li>Violates copyright or intellectual property</li>
                <li>Contains hate speech or harassment</li>
                <li>Depicts violence or dangerous activities</li>
                <li>Contains sexually explicit material</li>
                <li>Violates YouTube Community Guidelines</li>
            </ul>
        </div>

        <div class="section">
            <h2>9. Disclaimers</h2>
            <div class="important">
                <p><strong>NO WARRANTIES:</strong> Service provided "as is" without guarantees.</p>
                <p><strong>LIMITED LIABILITY:</strong> Our liability is limited to $100 or amounts you paid in the past 12 months.</p>
            </div>
        </div>

        <div class="section">
            <h2>10. Account Termination</h2>
            <p><strong>By You:</strong> Delete account anytime from settings. All data deleted within 30 days.</p>
            <p><strong>By Us:</strong> We may suspend accounts that violate these Terms.</p>
        </div>

        <div class="section">
            <h2>11. Changes to Terms</h2>
            <p>We may update these Terms. Changes effective upon posting. Continued use = acceptance.</p>
        </div>

        <div class="section">
            <h2>12. Contact</h2>
            <p><strong>Email:</strong> support@yourapp.com</p>
            <p><strong>Legal inquiries:</strong> legal@yourapp.com</p>
        </div>

        <div style="text-align: center; margin-top: 40px; padding-top: 20px; border-top: 1px solid rgba(59, 130, 246, 0.3);">
            <a href="/" style="display: inline-block; background: linear-gradient(135deg, #3b82f6, #1d4ed8); color: white; padding: 12px 24px; border-radius: 25px; text-decoration: none; margin: 5px;">← Back to Home</a>
            <a href="/privacy" style="display: inline-block; background: #6c757d; color: white; padding: 12px 24px; border-radius: 25px; text-decoration: none; margin: 5px;">View Privacy Policy</a>
        </div>
    </div>

    <script>
        class DynamicBackgroundManager {
            constructor() { this.updateBackground(); setInterval(() => this.updateBackground(), 5*60*1000); }
            async updateBackground() {
                try {
                    const r = await fetch('/api/background/image');
                    if (r.ok) {
                        const blob = await r.blob();
                        const url = URL.createObjectURL(blob);
                        const o = document.createElement('div');
                        o.style.cssText = `position:fixed;top:0;left:0;width:100%;height:100%;background-image:url(${url});background-size:cover;background-position:center;opacity:0;transition:opacity 1s;z-index:-1;pointer-events:none`;
                        document.body.appendChild(o);
                        setTimeout(() => o.style.opacity = '0.3', 100);
                        setTimeout(() => {
                            const old = document.querySelectorAll('div[style*="background-image"]');
                            old.forEach((e, i) => { if (i < old.length - 1) e.remove(); });
                        }, 1100);
                    }
                } catch (e) { console.error(e); }
            }
        }
        new DynamicBackgroundManager();
    </script>
</body>
</html>
    "###;
    Html(html.to_string())
}
