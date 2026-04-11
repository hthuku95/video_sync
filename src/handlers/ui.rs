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
        .route("/clipping/manage", get(clipping_management_page))
        .route("/help", get(help_guide_page))
        .route("/privacy", get(privacy_policy_page))
        .route("/terms", get(terms_of_service_page))
        .route("/chat", get(chat_interface))
        .route("/chat/:session_id", get(chat_interface_with_session))
        .route("/app", get(chat_interface)) // Alternative route
        .route("/video-tools", get(video_tools_page))
        .route("/manual-clipping", get(manual_clipping_page))
        .route("/signup/clipper", get(clipper_signup_page))
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
            <h2>320-Tool FFmpeg Toolkit — Fully AI-Accessible</h2>
            <p style="text-align:center;color:#666;margin-bottom:2rem;">Every tool is available via natural language. Just describe what you want.</p>
            <div class="tools-grid">
                <div class="tool-category">
                    <h3>🎬 Core Editing</h3>
                    <ul class="tool-list">
                        <li>Trim, Cut, Merge, Split</li>
                        <li>Deshake / Stabilize (vid.stab)</li>
                        <li>Reverse, Loop, Concatenate</li>
                        <li>Scene Detection & Analysis</li>
                        <li>Segment & Chapter Split</li>
                    </ul>
                </div>
                <div class="tool-category">
                    <h3>🎨 Visual Effects (100+)</h3>
                    <ul class="tool-list">
                        <li>Color Grading & LUT3D</li>
                        <li>Cinematic Film Grain & Vignette</li>
                        <li>Chroma Key / Green Screen</li>
                        <li>Motion Blur, Glow, Bloom</li>
                        <li>Edge Detect, Posterize, Solarize</li>
                        <li>Vintage Curves, Vibrance, HSV</li>
                    </ul>
                </div>
                <div class="tool-category">
                    <h3>🔊 Audio Processing (80+)</h3>
                    <ul class="tool-list">
                        <li>Loudnorm (EBU R128 / LUFS)</li>
                        <li>RNN Denoise & De-esser</li>
                        <li>Equalizer, Compressor, Limiter</li>
                        <li>CQT & Spectrum Visualization</li>
                        <li>Stereo Widener, Surround Mix</li>
                        <li>Pitch Shift, Time Stretch</li>
                    </ul>
                </div>
                <div class="tool-category">
                    <h3>📊 Analysis & Review (40+)</h3>
                    <ul class="tool-list">
                        <li>VMAF / SSIM / PSNR Quality</li>
                        <li>Loudness & Silence Detection</li>
                        <li>Scene Change Detection</li>
                        <li>Black Frame & Freeze Detect</li>
                        <li>Bitrate & Metadata Extraction</li>
                    </ul>
                </div>
                <div class="tool-category">
                    <h3>📤 Platform Export</h3>
                    <ul class="tool-list">
                        <li>YouTube / TikTok / Instagram</li>
                        <li>Format Conversion (20+ formats)</li>
                        <li>H.264 / H.265 / VP9 / AV1</li>
                        <li>GIF with Palette Optimization</li>
                        <li>HDR to SDR Tone Mapping</li>
                    </ul>
                </div>
                <div class="tool-category">
                    <h3>⚡ Workflow Recipes</h3>
                    <ul class="tool-list">
                        <li>YouTube-Ready Export</li>
                        <li>Podcast Audio Cleanup</li>
                        <li>Cinematic Grade</li>
                        <li>Talking Head Cleanup</li>
                        <li>GIF Creator</li>
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
            const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
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
                        // Clippers land on manual clipping, everyone else on dashboard
                        const dest = data.user && data.user.is_clipper ? '/manual-clipping' : '/dashboard';
                        window.location.href = dest;
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
                        const dest = data.user && data.user.is_clipper ? '/manual-clipping' : '/dashboard';
                        window.location.href = dest;
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
                <!-- YouTube Clipping Card (only for admins/whitelisted users) -->
                <a href="/clipping/manage" class="action-card" id="clipping-action-card" style="display: none;">
                    <h3>✂️ YouTube Clipping</h3>
                    <p>Auto-generate viral clips from popular channels and post to your channel</p>
                </a>
                <a href="/video-tools" class="action-card">
                    <h3>🛠️ Video Tools</h3>
                    <p>Stabilize, convert formats, visualize audio, and run workflow recipes directly</p>
                </a>
                <a href="/gig-templates" class="action-card">
                    <h3>💼 Gig Templates</h3>
                    <p>Fiverr & PPH gig info with pricing tiers, copy-paste descriptions, and AI sample video generation</p>
                </a>
                <a href="/manual-clipping" class="action-card">
                    <h3>✂️ Manual Clipping</h3>
                    <p>Paste any YouTube or Twitch URL to extract viral clips with download links — no destination channel needed</p>
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
        const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
        if (!authToken) {
            window.location.href = '/login';
        }

        // Clippers should use manual clipping dashboard, not this page
        const user = JSON.parse(localStorage.getItem('user') || '{}');
        if (user.is_clipper) {
            window.location.href = '/manual-clipping';
        }

        // Set user welcome message
        if (user.username) {
            document.getElementById('userWelcome').textContent = `Welcome back, ${user.username}!`;
        }

        // Show/hide YouTube clipping card based on permissions
        // Check access via API to include whitelisted users
        const clippingCard = document.getElementById('clipping-action-card');
        if (clippingCard) {
            const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
            if (authToken) {
                fetch('/api/clipping/access-check', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                })
                .then(response => {
                    if (response.ok) {
                        clippingCard.style.display = 'block';
                    }
                })
                .catch(err => console.debug('Clipping access check failed:', err));
            }
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
                const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
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
                const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
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
        let jobPollTimer = null;
        let seenJobIds = new Set();
        // Session ID passed from server (either specific session or null for new)
        let providedSessionId = SESSION_ID_PLACEHOLDER;

        console.log('📝 Provided Session ID:', providedSessionId);

        let sessionUuid = providedSessionId || generateUUID();

        console.log('🆔 Final Session UUID:', sessionUuid);

        // Initialize the application
        document.addEventListener('DOMContentLoaded', function() {
            console.log('🚀 DOMContentLoaded event fired - initializing chat interface');
            console.log('Auth token present:', !!localStorage.getItem('auth_token') || localStorage.getItem('authToken'));

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
                const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
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
                    stopJobPolling(); // WS live — polling not needed
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
                            seenJobIds.add('ws-' + Date.now());
                            addMessage('assistant', jsonData.content);
                            break;
                        case 'thinking':
                            // Update typing indicator with agent thinking/tool calling details
                            console.log('Thinking update:', jsonData.content);
                            updateThinkingIndicator(jsonData.content);
                            break;
                        case 'background_job_status':
                            // Shown on reconnect when a task is still running in the background
                            hideTypingIndicator();
                            addMessage('assistant', jsonData.content, true /* isStatus */);
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

                // Start polling for completed background jobs while WS is down
                startJobPolling();

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

        // ─── Background job polling (SSR fallback when WS is disconnected) ──────
        function startJobPolling() {
            if (jobPollTimer) return;
            jobPollTimer = setInterval(async function() {
                const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
                if (!authToken) return;
                try {
                    const res = await fetch(`/api/chat/sessions/${sessionUuid}/jobs`, {
                        headers: { 'Authorization': 'Bearer ' + authToken }
                    });
                    if (!res.ok) return;
                    const data = await res.json();
                    const jobs = data.jobs || [];
                    for (const job of jobs) {
                        if (job.status === 'completed' && job.result) {
                            const msgId = 'job-' + job.id;
                            if (!seenJobIds.has(msgId)) {
                                seenJobIds.add(msgId);
                                hideTypingIndicator();
                                addMessage('assistant', job.result);
                            }
                        } else if (job.status === 'failed' && job.error) {
                            const msgId = 'job-err-' + job.id;
                            if (!seenJobIds.has(msgId)) {
                                seenJobIds.add(msgId);
                                hideTypingIndicator();
                                addMessage('assistant', '❌ Task failed: ' + job.error);
                            }
                        }
                    }
                } catch(e) {
                    console.warn('Job polling error:', e);
                }
            }, 10000); // every 10s
        }

        function stopJobPolling() {
            if (jobPollTimer) {
                clearInterval(jobPollTimer);
                jobPollTimer = null;
            }
        }
        // ─────────────────────────────────────────────────────────────────────

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
                const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
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
                            <button onclick="connectYouTubeChannel()" class="btn">Connect YouTube Channel</button>
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

        async function connectYouTubeChannel() {
            try {
                const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
                if (!authToken) {
                    alert('Please log in first');
                    window.location.href = '/login';
                    return;
                }

                // Fetch OAuth URL from backend with Authorization header
                const response = await fetch('/youtube/connect?redirect_to=' + encodeURIComponent(window.location.pathname), {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });

                if (!response.ok) {
                    const error = await response.json();
                    alert('Failed to connect: ' + (error.message || 'Unknown error'));
                    return;
                }

                const data = await response.json();
                if (data.success && data.auth_url) {
                    // Redirect to Google OAuth page
                    window.location.href = data.auth_url;
                } else {
                    alert('Failed to get authorization URL');
                }
            } catch (error) {
                console.error('Error connecting YouTube:', error);
                alert('Failed to connect YouTube channel. Please try again.');
            }
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
                const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
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
            const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
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
            const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
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
            <a href="#edit">Video Editing (320 Tools)</a>
            <a href="#workflows">Workflow Recipes</a>
            <a href="#video-tools">Video Tools Page</a>
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
            <h2>3. Video Editing — 320 Tools</h2>
            <h3>Core Editing</h3>
            <ul>
                <li>Trim, Merge, Split, Resize, Crop, Rotate, Reverse, Loop</li>
                <li>2-pass video stabilization (vid.stab)</li>
                <li>Scene detection, black frame & silence detection</li>
            </ul>
            <h3>Visual Effects (100+ tools)</h3>
            <ul>
                <li>Color grading: LUT3D, curves, vibrance, HSV, hue/saturation</li>
                <li>Cinematic: film grain, vignette, vintage curves, telecine</li>
                <li>Keying: chroma key, luma key, color hold</li>
                <li>Spatial: motion blur, glow, bloom, sharpen, denoise</li>
                <li>Artistic: posterize (geq), solarize, CLAHE (histeq), banding fix</li>
            </ul>
            <h3>Audio Processing (80+ tools)</h3>
            <ul>
                <li>Loudness: EBU R128 / LUFS normalization, limiter (alimiter)</li>
                <li>Cleanup: RNN denoiser, de-esser (deesser), speech normalizer</li>
                <li>EQ: graphiceq, parametric eq, high/low shelf, bandpass</li>
                <li>Dynamics: compressor, expander, gate, sidechaining</li>
                <li>Visualize: CQT, spectrum analyzer, waveform video</li>
            </ul>
            <h3>Analysis (40+ tools)</h3>
            <ul>
                <li>Quality: VMAF, SSIM, PSNR</li>
                <li>Inspection: bitrate, metadata, scene change, freeze frames</li>
            </ul>
            <div class="example-box">
"Stabilize this shaky footage"<br>
"Apply film grain and a vignette for a cinematic look"<br>
"Normalize audio to -14 LUFS for YouTube"<br>
"Remove background noise and de-ess the sibilance"<br>
"Generate a CQT audio visualization"<br>
"Convert to MKV"
            </div>
        </div>

        <div class="section" id="workflows">
            <h2>4. Workflow Recipes</h2>
            <p>Named multi-step chains that apply several tools in sequence with a single command:</p>
            <ul>
                <li><strong>YouTube Ready Export</strong> — stabilize → normalize color → loudnorm −14 LUFS → yuv420p</li>
                <li><strong>Podcast Cleanup</strong> — denoise → de-ess sibilance → limit peaks → loudnorm −16 LUFS</li>
                <li><strong>Cinematic Grade</strong> — vintage curves → vibrance → vignette → film grain</li>
                <li><strong>Talking Head Cleanup</strong> — stabilize → denoise speech → de-ess → loudnorm −16 LUFS</li>
                <li><strong>GIF Creator</strong> — trim segment → scale → optimize palette</li>
            </ul>
            <div class="example-box">
"Run the YouTube ready export workflow on my video"<br>
"Apply podcast cleanup to this recording"<br>
"Give it a cinematic grade"<br>
"Create a GIF from seconds 10 to 15"
            </div>
        </div>

        <div class="section" id="video-tools">
            <h2>5. Video Tools Page</h2>
            <p>For direct tool access without the AI chat, visit <a href="/video-tools" style="color:#3b82f6;">/video-tools</a>. It provides four interactive panels:</p>
            <ul>
                <li><strong>Stabilize</strong> — shakiness/smoothing/zoom sliders, runs 2-pass vid.stab</li>
                <li><strong>Convert Format</strong> — dropdown of 11 output formats (mp4, mkv, webm, mov, wav, …)</li>
                <li><strong>Audio Visualizer</strong> — waveform / spectrum / CQT video output</li>
                <li><strong>Workflows</strong> — pick a recipe, enter file path, get a download link</li>
            </ul>
            <p>Enter the file path relative to <code>uploads/</code> (the filename shown after upload). A download link appears when processing completes.</p>
        </div>

        <div class="section" id="youtube">
            <h2>6. YouTube Integration</h2>
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
            <h2>7. AI Tools</h2>
            <ul>
                <li><strong>Stock Media:</strong> Free videos/photos from Pexels</li>
                <li><strong>TTS:</strong> 17+ natural voices (75ms latency)</li>
                <li><strong>Music:</strong> Studio-quality background tracks</li>
                <li><strong>Sound FX:</strong> Custom sound effects</li>
            </ul>
        </div>

        <div class="section" id="trouble">
            <h2>8. Troubleshooting</h2>
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
// YouTube Clipping Management Page
// ============================================================================

pub async fn clipping_management_page() -> Html<String> {
    let html = r###"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>YouTube Clipping Manager - VideoSync</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            min-height: 100vh;
            color: #e8e8e8;
        }
        .container { max-width: 1400px; margin: 0 auto; padding: 2rem; }
        .header {
            background: rgba(30, 30, 52, 0.8);
            backdrop-filter: blur(20px);
            border-bottom: 1px solid rgba(59, 130, 246, 0.3);
            padding: 1.5rem 2rem;
            margin-bottom: 2rem;
            border-radius: 15px;
        }
        .header h1 { font-size: 2rem; margin-bottom: 0.5rem; }
        .header p { color: #cbd5e1; font-size: 1rem; }
        .btn {
            padding: 0.75rem 1.5rem;
            background: linear-gradient(135deg, #3b82f6, #1d4ed8);
            color: white;
            border: none;
            border-radius: 10px;
            font-weight: 600;
            cursor: pointer;
            transition: all 0.3s;
            text-decoration: none;
            display: inline-block;
        }
        .btn:hover {
            background: linear-gradient(135deg, #2563eb, #1e40af);
            transform: translateY(-2px);
            box-shadow: 0 4px 12px rgba(59, 130, 246, 0.4);
        }
        .btn-secondary { background: linear-gradient(135deg, #64748b, #475569); }
        .btn-secondary:hover { background: linear-gradient(135deg, #475569, #334155); }
        .btn-danger { background: linear-gradient(135deg, #ef4444, #dc2626); }
        .btn-danger:hover { background: linear-gradient(135deg, #dc2626, #b91c1c); }
        .btn-small { padding: 0.5rem 1rem; font-size: 0.9rem; }

        .tabs {
            display: flex;
            gap: 1rem;
            margin-bottom: 2rem;
            border-bottom: 2px solid rgba(59, 130, 246, 0.2);
        }
        .tab-button {
            padding: 1rem 2rem;
            background: none;
            border: none;
            color: #cbd5e1;
            cursor: pointer;
            border-bottom: 3px solid transparent;
            transition: all 0.3s;
            font-size: 1rem;
            font-weight: 500;
        }
        .tab-button.active {
            color: #3b82f6;
            border-bottom-color: #3b82f6;
        }
        .tab-button:hover { color: #60a5fa; }

        .tab-content { display: none; }
        .tab-content.active { display: block; }

        .card {
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            border-radius: 15px;
            padding: 1.5rem;
            margin-bottom: 1rem;
        }
        .grid {
            display: grid;
            grid-template-columns: repeat(auto-fill, minmax(350px, 1fr));
            gap: 1.5rem;
        }

        .channel-card {
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            border-radius: 15px;
            padding: 1.5rem;
            transition: all 0.3s;
        }
        .channel-card:hover {
            transform: translateY(-5px);
            box-shadow: 0 8px 20px rgba(59, 130, 246, 0.3);
            border-color: rgba(59, 130, 246, 0.5);
        }
        .channel-header {
            display: flex;
            gap: 1rem;
            margin-bottom: 1rem;
        }
        .channel-thumbnail {
            width: 60px;
            height: 60px;
            border-radius: 50%;
            object-fit: cover;
        }
        .channel-info h3 { font-size: 1.2rem; margin-bottom: 0.25rem; }
        .channel-info p { color: #94a3b8; font-size: 0.9rem; }

        .linkage-card {
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            border-radius: 15px;
            padding: 1.5rem;
        }
        .linkage-flow {
            display: flex;
            align-items: center;
            gap: 1rem;
            margin-bottom: 1rem;
        }
        .linkage-arrow { font-size: 1.5rem; color: #3b82f6; }

        .job-card {
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            border-radius: 15px;
            padding: 1.5rem;
            margin-bottom: 1rem;
        }
        .progress-bar {
            width: 100%;
            height: 8px;
            background: rgba(59, 130, 246, 0.2);
            border-radius: 10px;
            overflow: hidden;
            margin: 0.5rem 0;
        }
        .progress-fill {
            height: 100%;
            background: linear-gradient(90deg, #3b82f6, #60a5fa);
            transition: width 0.3s;
        }

        .status-badge {
            padding: 0.25rem 0.75rem;
            border-radius: 20px;
            font-size: 0.85rem;
            font-weight: 600;
        }
        .status-pending { background: rgba(251, 191, 36, 0.2); color: #fbbf24; }
        .status-running { background: rgba(59, 130, 246, 0.2); color: #3b82f6; }
        .status-completed { background: rgba(34, 197, 94, 0.2); color: #22c55e; }
        .status-failed { background: rgba(239, 68, 68, 0.2); color: #ef4444; }

        .modal {
            display: none;
            position: fixed;
            top: 0;
            left: 0;
            width: 100%;
            height: 100%;
            background: rgba(0, 0, 0, 0.7);
            z-index: 1000;
            align-items: center;
            justify-content: center;
        }
        .modal.active { display: flex; }
        .modal-content {
            background: rgba(30, 30, 52, 0.95);
            border: 1px solid rgba(59, 130, 246, 0.3);
            border-radius: 20px;
            padding: 2rem;
            max-width: 600px;
            width: 90%;
            max-height: 80vh;
            overflow-y: auto;
        }
        .modal-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
            margin-bottom: 1.5rem;
        }
        .modal-close {
            background: none;
            border: none;
            color: #e8e8e8;
            font-size: 1.5rem;
            cursor: pointer;
        }

        .form-group {
            margin-bottom: 1.5rem;
        }
        .form-group label {
            display: block;
            margin-bottom: 0.5rem;
            color: #cbd5e1;
            font-weight: 500;
        }
        .form-group input,
        .form-group select {
            width: 100%;
            padding: 0.75rem;
            background: rgba(30, 30, 52, 0.8);
            border: 1px solid rgba(59, 130, 246, 0.3);
            border-radius: 10px;
            color: #e8e8e8;
            font-size: 1rem;
        }
        .form-group input:focus,
        .form-group select:focus {
            outline: none;
            border-color: #3b82f6;
        }

        .search-results {
            max-height: 400px;
            overflow-y: auto;
            margin-top: 1rem;
        }
        .search-result-item {
            display: flex;
            gap: 1rem;
            padding: 1rem;
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            border-radius: 10px;
            margin-bottom: 0.5rem;
            cursor: pointer;
            transition: all 0.3s;
        }
        .search-result-item:hover {
            background: rgba(30, 30, 52, 0.8);
            border-color: rgba(59, 130, 246, 0.5);
        }

        .loading {
            text-align: center;
            padding: 3rem;
            color: #94a3b8;
        }
        .empty-state {
            text-align: center;
            padding: 3rem;
            color: #94a3b8;
        }
        .empty-state-icon { font-size: 3rem; margin-bottom: 1rem; }

        .clip-card {
            background: rgba(30, 30, 52, 0.6);
            border: 1px solid rgba(59, 130, 246, 0.2);
            border-radius: 15px;
            padding: 1rem;
        }
        .clip-thumbnail {
            width: 100%;
            height: 200px;
            background: rgba(15, 20, 25, 0.8);
            border-radius: 10px;
            margin-bottom: 1rem;
            display: flex;
            align-items: center;
            justify-content: center;
            color: #64748b;
        }
        .viral-factors {
            display: flex;
            flex-wrap: wrap;
            gap: 0.5rem;
            margin: 0.5rem 0;
        }
        .viral-factor-badge {
            padding: 0.25rem 0.5rem;
            background: rgba(139, 92, 246, 0.2);
            color: #a78bfa;
            border-radius: 5px;
            font-size: 0.75rem;
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
            <div style="display: flex; justify-content: space-between; align-items: center;">
                <div>
                    <h1>✂️ YouTube Clipping Manager</h1>
                    <p>Monitor channels, auto-clip viral moments, post to your channels</p>
                </div>
                <div>
                    <a href="/dashboard" class="btn btn-secondary">← Back to Dashboard</a>
                </div>
            </div>
        </div>

        <div class="tabs">
            <button class="tab-button active" onclick="switchTab('sources')">📺 Source Channels</button>
            <button class="tab-button" onclick="switchTab('linkages')">🔗 Channel Linkages</button>
            <button class="tab-button" onclick="switchTab('jobs')">⚙️ Active Jobs</button>
            <button class="tab-button" onclick="switchTab('clips')">🎬 Generated Clips</button>
            <button class="tab-button" onclick="switchTab('review')">🔍 Review Queue</button>
        </div>

        <!-- Tab 1: Source Channels -->
        <div id="sourcesTab" class="tab-content active">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.5rem;">
                <h2>Source Channels to Monitor</h2>
                <button onclick="openAddChannelModal()" class="btn">+ Add Source Channel</button>
            </div>
            <div id="sourceChannelsContainer" class="loading">Loading source channels...</div>
        </div>

        <!-- Tab 2: Channel Linkages -->
        <div id="linkagesTab" class="tab-content">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1.5rem;">
                <h2>Channel Linkages</h2>
                <button onclick="openCreateLinkageModal()" class="btn">+ Create Linkage</button>
            </div>
            <div id="linkagesContainer" class="loading">Loading linkages...</div>
        </div>

        <!-- Tab 3: Clipping Jobs -->
        <div id="jobsTab" class="tab-content">
            <h2 style="margin-bottom: 1.5rem;">Active Clipping Jobs</h2>
            <div id="jobsContainer" class="loading">Loading jobs...</div>
        </div>

        <!-- Tab 4: Generated Clips -->
        <div id="clipsTab" class="tab-content">
            <h2 style="margin-bottom: 1.5rem;">Generated Clips Gallery</h2>
            <div id="clipsContainer" class="loading">Loading clips...</div>
        </div>

        <!-- Tab 5: Review Queue + Content Management -->
        <div id="reviewTab" class="tab-content">
            <h2 style="margin-bottom: 1.5rem;">Review Queue</h2>
            <p style="color: #94a3b8; margin-bottom: 1.5rem;">
                Clips awaiting human approval before upload. Enable per-linkage approval in the Linkages tab.
            </p>
            <div id="reviewContainer" class="loading">Loading pending clips...</div>

            <!-- Content Management Agent -->
            <div class="card" style="margin-top: 2rem;">
                <h3 style="margin-bottom: 1rem;">🤖 Content Management Agent</h3>
                <p style="color: #94a3b8; margin-bottom: 1rem; font-size: 0.9rem;">
                    Type a natural-language instruction to manage your published clips (update metadata, delete, repost).
                </p>
                <div class="form-group">
                    <label>Select Destination Channel</label>
                    <select id="cmChannelSelect" style="width:100%; padding:0.75rem; background:rgba(30,30,52,0.8); border:1px solid rgba(59,130,246,0.3); border-radius:10px; color:#e8e8e8; font-size:1rem;">
                        <option value="">Loading channels...</option>
                    </select>
                </div>
                <div class="form-group">
                    <label>Instruction</label>
                    <textarea id="cmInstruction" rows="3"
                        placeholder="e.g. Take down clip 3 and repost it with title: Better Title"
                        style="width:100%; padding:0.75rem; background:rgba(30,30,52,0.8); border:1px solid rgba(59,130,246,0.3); border-radius:10px; color:#e8e8e8; font-size:1rem; resize:vertical;"></textarea>
                </div>
                <button onclick="startContentManagement()" class="btn">Run Agent</button>
                <div id="cmStatus" style="margin-top:1rem; display:none;"></div>
            </div>
        </div>
    </div>

    <!-- Modal: Add Channel -->
    <div id="addChannelModal" class="modal">
        <div class="modal-content">
            <div class="modal-header">
                <h2>Add Source Channel</h2>
                <button class="modal-close" onclick="closeAddChannelModal()">×</button>
            </div>
            <div class="form-group">
                <label>Search for YouTube Channel</label>
                <input type="text" id="channelSearchInput" placeholder="e.g., MrBeast, PewDiePie..." oninput="searchChannels(this.value)">
            </div>
            <div id="channelSearchResults" class="search-results"></div>
        </div>
    </div>

    <!-- Modal: Create Linkage -->
    <div id="createLinkageModal" class="modal">
        <div class="modal-content">
            <div class="modal-header">
                <h2>Create Channel Linkage</h2>
                <button class="modal-close" onclick="closeCreateLinkageModal()">×</button>
            </div>
            <div class="form-group">
                <label>Source Channel (to clip from)</label>
                <select id="linkageSourceChannel"></select>
            </div>
            <div class="form-group">
                <label>Destination Channel (your channel)</label>
                <select id="linkageDestChannel"></select>
            </div>
            <div class="form-group">
                <label>Clips per Video (1-4)</label>
                <input type="number" id="linkageClipsPerVideo" min="1" max="4" value="2">
            </div>
            <div class="form-group">
                <label>Min Clip Duration (seconds)</label>
                <input type="number" id="linkageMinDuration" min="30" max="300" value="60">
            </div>
            <div class="form-group">
                <label>Max Clip Duration (seconds)</label>
                <input type="number" id="linkageMaxDuration" min="60" max="300" value="120">
            </div>
            <button onclick="createLinkage()" class="btn">Create Linkage</button>
        </div>
    </div>

    <script>
        const authToken = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
        if (!authToken) {
            window.location.href = '/login';
        }

        let searchTimeout = null;

        // Per-job WebSocket connections (job_id → WebSocket)
        const jobSockets = {};

        function openJobWebSocket(jobId) {
            if (jobSockets[jobId]) return;
            const proto = location.protocol === 'https:' ? 'wss' : 'ws';
            const ws = new WebSocket(proto + '://' + location.host + '/ws/clipping-jobs/' + jobId);
            jobSockets[jobId] = ws;

            ws.onmessage = function(event) {
                try { updateJobCard(jobId, JSON.parse(event.data)); } catch(e) {}
            };

            ws.onclose = function() {
                delete jobSockets[jobId];
                const card = document.getElementById('job-card-' + jobId);
                if (card && card.dataset.status === 'processing') {
                    setTimeout(function() { openJobWebSocket(jobId); }, 3000);
                }
            };
        }

        function updateJobCard(jobId, update) {
            const s = update.status;
            if (!s) return;
            const status = s.status;
            const step = s.current_step || '';
            const pct = s.progress_percent != null ? s.progress_percent : 0;
            const detail = s.current_action_detail || '';

            const stepEl = document.getElementById('job-step-' + jobId);
            if (stepEl) stepEl.textContent = step || 'Processing...';

            const detailEl = document.getElementById('job-detail-' + jobId);
            if (detailEl) detailEl.textContent = detail;

            const barEl = document.getElementById('job-progress-bar-' + jobId);
            const pctEl = document.getElementById('job-progress-pct-' + jobId);
            if (barEl) barEl.style.width = pct + '%';
            if (pctEl) pctEl.textContent = Math.round(pct) + '%';

            const statusEl = document.getElementById('job-status-' + jobId);
            if (statusEl) {
                const classes = { running: 'status-running', completed: 'status-completed',
                                  failed: 'status-failed', queued: 'status-pending' };
                statusEl.className = 'status-badge ' + (classes[status] || 'status-running');
                statusEl.textContent = status;
            }

            const card = document.getElementById('job-card-' + jobId);
            if (card) card.dataset.status = status;

            if (status === 'completed' || status === 'failed') {
                if (jobSockets[jobId]) { jobSockets[jobId].close(); delete jobSockets[jobId]; }
                setTimeout(loadJobs, 2000);
            }
        }

        // Tab switching
        function switchTab(tabName) {
            // Close any open job WebSocket connections when leaving jobs tab
            if (tabName !== 'jobs') {
                Object.values(jobSockets).forEach(function(ws) { ws.close(); });
                Object.keys(jobSockets).forEach(function(k) { delete jobSockets[k]; });
            }

            document.querySelectorAll('.tab-button').forEach(btn => btn.classList.remove('active'));
            document.querySelectorAll('.tab-content').forEach(content => content.classList.remove('active'));

            event.target.classList.add('active');
            document.getElementById(tabName + 'Tab').classList.add('active');

            // Load data for the active tab
            if (tabName === 'sources') loadSourceChannels();
            else if (tabName === 'linkages') loadLinkages();
            else if (tabName === 'jobs') loadJobs();
            else if (tabName === 'clips') loadClips();
            else if (tabName === 'review') { loadPendingReview(); loadCMChannels(); }
        }

        // Load pending review clips
        async function loadPendingReview() {
            const container = document.getElementById('reviewContainer');
            container.className = 'loading';
            container.innerHTML = 'Loading pending clips...';

            try {
                const response = await fetch('/api/clipping/clips/pending-review', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();
                const clips = data.clips || [];

                if (clips.length === 0) {
                    container.className = 'empty-state';
                    container.innerHTML = '<div class="empty-state-icon">✅</div><h3>No clips pending review</h3><p>All clips have been reviewed or auto-published</p>';
                    return;
                }

                container.className = 'grid';
                container.innerHTML = clips.map(clip => `
                    <div class="clip-card" id="review-clip-${clip.id}">
                        <div class="clip-thumbnail">🎬 ${(clip.duration_seconds || 0).toFixed(1)}s</div>
                        <p style="color:#94a3b8; font-size:0.8rem; margin-bottom:0.5rem;">${clip.source_video_title || 'Unknown source'}</p>
                        <div class="form-group" style="margin-bottom:0.5rem;">
                            <label style="font-size:0.85rem;">Title</label>
                            <input type="text" id="review-title-${clip.id}" value="${(clip.proposed_title || clip.ai_title || '').replace(/"/g, '&quot;')}"
                                   style="width:100%; padding:0.5rem; background:rgba(30,30,52,0.8); border:1px solid rgba(59,130,246,0.3); border-radius:8px; color:#e8e8e8; font-size:0.9rem;">
                        </div>
                        <div class="form-group" style="margin-bottom:0.75rem;">
                            <label style="font-size:0.85rem;">Description</label>
                            <textarea id="review-desc-${clip.id}" rows="2"
                                      style="width:100%; padding:0.5rem; background:rgba(30,30,52,0.8); border:1px solid rgba(59,130,246,0.3); border-radius:8px; color:#e8e8e8; font-size:0.9rem; resize:vertical;">${clip.proposed_description || ''}</textarea>
                        </div>
                        <div style="display:flex; gap:0.5rem;">
                            <button onclick="saveAndApproveClip(${clip.id})" class="btn btn-small" style="background:linear-gradient(135deg,#22c55e,#16a34a);">Approve & Upload</button>
                            <button onclick="rejectReviewClip(${clip.id})" class="btn btn-danger btn-small">Reject</button>
                        </div>
                    </div>
                `).join('');
            } catch (error) {
                container.className = 'empty-state';
                container.innerHTML = '<p style="color:#ef4444;">Error: ' + error.message + '</p>';
            }
        }

        async function saveAndApproveClip(clipId) {
            const title = document.getElementById('review-title-' + clipId).value;
            const desc = document.getElementById('review-desc-' + clipId).value;

            // Save edits first
            if (title || desc) {
                await fetch('/api/clipping/clips/' + clipId + '/propose-edit', {
                    method: 'PUT',
                    headers: { 'Authorization': 'Bearer ' + authToken, 'Content-Type': 'application/json' },
                    body: JSON.stringify({ proposed_title: title, proposed_description: desc })
                });
            }

            // Approve (triggers upload)
            try {
                const r = await fetch('/api/clipping/clips/' + clipId + '/approve', {
                    method: 'PUT',
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await r.json();
                if (data.success) {
                    const card = document.getElementById('review-clip-' + clipId);
                    if (card) card.remove();
                    alert('Clip approved and uploading to YouTube!');
                } else {
                    alert('Approval failed. Check server logs.');
                }
            } catch (e) {
                alert('Error: ' + e.message);
            }
        }

        async function rejectReviewClip(clipId) {
            const reason = prompt('Reason for rejection (optional):');
            try {
                const r = await fetch('/api/clipping/clips/' + clipId + '/reject', {
                    method: 'PUT',
                    headers: { 'Authorization': 'Bearer ' + authToken, 'Content-Type': 'application/json' },
                    body: JSON.stringify({ reason })
                });
                if (r.ok) {
                    const card = document.getElementById('review-clip-' + clipId);
                    if (card) card.remove();
                } else {
                    alert('Rejection failed.');
                }
            } catch (e) {
                alert('Error: ' + e.message);
            }
        }

        // Load channels for content management agent dropdown
        async function loadCMChannels() {
            try {
                const r = await fetch('/api/youtube/channels', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await r.json();
                const channels = data.channels || [];
                document.getElementById('cmChannelSelect').innerHTML = channels.map(c =>
                    '<option value="' + c.id + '">' + c.channel_name + '</option>'
                ).join('') || '<option value="">No channels connected</option>';
            } catch (e) {}
        }

        // Start content management session
        let cmSessionId = null;
        let cmPollInterval = null;

        async function startContentManagement() {
            const instruction = document.getElementById('cmInstruction').value.trim();
            const channelId = parseInt(document.getElementById('cmChannelSelect').value);
            if (!instruction) { alert('Please enter an instruction.'); return; }
            if (!channelId) { alert('Please select a channel.'); return; }

            const statusDiv = document.getElementById('cmStatus');
            statusDiv.style.display = 'block';
            statusDiv.innerHTML = '<div class="loading">Starting agent...</div>';

            try {
                const r = await fetch('/api/clipping/manage-content', {
                    method: 'POST',
                    headers: { 'Authorization': 'Bearer ' + authToken, 'Content-Type': 'application/json' },
                    body: JSON.stringify({ instruction, destination_channel_id: channelId })
                });
                const data = await r.json();
                cmSessionId = data.session_id;
                statusDiv.innerHTML = '<p>Session #' + cmSessionId + ' started. Polling for updates...</p><div id="cmStatusBody"></div>';
                if (cmPollInterval) clearInterval(cmPollInterval);
                cmPollInterval = setInterval(pollCMSession, 3000);
            } catch (e) {
                statusDiv.innerHTML = '<p style="color:#ef4444;">Error: ' + e.message + '</p>';
            }
        }

        async function pollCMSession() {
            if (!cmSessionId) return;
            try {
                const r = await fetch('/api/clipping/manage-content/' + cmSessionId, {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await r.json();
                const session = data.session;
                const body = document.getElementById('cmStatusBody');
                if (!body) return;

                let html = '<p><strong>Status:</strong> <span style="color:' +
                    (session.status === 'completed' ? '#22c55e' : session.status === 'failed' ? '#ef4444' : '#3b82f6') +
                    '">' + session.status + '</span></p>';

                if (session.status === 'awaiting_confirmation' && session.confirmation_required) {
                    const cr = session.confirmation_required;
                    html += '<div style="background:rgba(239,68,68,0.15); border:1px solid rgba(239,68,68,0.4); border-radius:10px; padding:1rem; margin-top:0.75rem;">';
                    html += '<strong>AI wants to:</strong> ' + (cr.action_summary || '') + '<br>';
                    html += '<div style="margin-top:0.75rem; display:flex; gap:0.5rem;">';
                    html += '<button onclick="confirmCMAction(true)" class="btn btn-small" style="background:linear-gradient(135deg,#22c55e,#16a34a);">Confirm</button>';
                    html += '<button onclick="confirmCMAction(false)" class="btn btn-danger btn-small">Cancel</button>';
                    html += '</div></div>';
                }

                if (session.result_summary) {
                    html += '<div style="margin-top:0.75rem; background:rgba(59,130,246,0.1); padding:0.75rem; border-radius:8px;"><strong>Result:</strong> ' + session.result_summary + '</div>';
                }

                body.innerHTML = html;

                if (session.status === 'completed' || session.status === 'failed') {
                    clearInterval(cmPollInterval);
                    cmPollInterval = null;
                }
            } catch (e) {}
        }

        async function confirmCMAction(confirmed) {
            if (!cmSessionId) return;
            await fetch('/api/clipping/manage-content/' + cmSessionId + '/confirm', {
                method: 'POST',
                headers: { 'Authorization': 'Bearer ' + authToken, 'Content-Type': 'application/json' },
                body: JSON.stringify({ confirmed })
            });
        }

        // Load source channels
        async function loadSourceChannels() {
            const container = document.getElementById('sourceChannelsContainer');
            container.className = 'loading';
            container.innerHTML = 'Loading source channels...';

            try {
                const response = await fetch('/api/clipping/source-channels', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();
                const channels = data.channels || data || [];

                if (channels.length === 0) {
                    container.className = 'empty-state';
                    container.innerHTML = `
                        <div class="empty-state-icon">📺</div>
                        <h3>No Source Channels Yet</h3>
                        <p>Add channels to start monitoring for viral content</p>
                    `;
                    return;
                }

                container.className = 'grid';
                container.innerHTML = channels.map(channel => `
                    <div class="channel-card">
                        <div class="channel-header">
                            <img src="${channel.channel_thumbnail_url || '/placeholder.png'}"
                                 alt="${channel.channel_name}"
                                 class="channel-thumbnail">
                            <div class="channel-info">
                                <h3>${channel.channel_name}</h3>
                                <p>${channel.subscriber_count ? (channel.subscriber_count/1000000).toFixed(1) + 'M subscribers' : 'Unknown'}</p>
                            </div>
                        </div>
                        <p style="color: #94a3b8; font-size: 0.9rem; margin-bottom: 1rem;">
                            Polling every ${channel.polling_interval_minutes} minutes
                        </p>
                        <div style="display: flex; gap: 0.5rem;">
                            <button onclick="deleteSourceChannel(${channel.id})" class="btn btn-danger btn-small">Delete</button>
                            <button onclick="toggleChannelActive(${channel.id}, ${!channel.is_active})"
                                    class="btn btn-small ${channel.is_active ? 'btn-secondary' : ''}">
                                ${channel.is_active ? 'Deactivate' : 'Activate'}
                            </button>
                        </div>
                    </div>
                `).join('');
            } catch (error) {
                container.className = 'empty-state';
                container.innerHTML = `<p style="color: #ef4444;">Error loading channels: ${error.message}</p>`;
            }
        }

        // Load linkages
        async function loadLinkages() {
            const container = document.getElementById('linkagesContainer');
            container.className = 'loading';
            container.innerHTML = 'Loading linkages...';

            try {
                const response = await fetch('/api/clipping/linkages', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();
                const linkages = data.linkages || data || [];

                if (linkages.length === 0) {
                    container.className = 'empty-state';
                    container.innerHTML = `
                        <div class="empty-state-icon">🔗</div>
                        <h3>No Linkages Yet</h3>
                        <p>Create linkages to start auto-clipping</p>
                    `;
                    return;
                }

                container.innerHTML = linkages.map(linkage => `
                    <div class="linkage-card">
                        <div class="linkage-flow">
                            <div>
                                <strong>${linkage.source_channel_name}</strong>
                                <p style="color: #94a3b8; font-size: 0.85rem;">Source</p>
                            </div>
                            <div class="linkage-arrow">→</div>
                            <div>
                                <strong>${linkage.destination_channel_name}</strong>
                                <p style="color: #94a3b8; font-size: 0.85rem;">Destination</p>
                            </div>
                        </div>
                        <div style="background: rgba(59, 130, 246, 0.1); padding: 1rem; border-radius: 10px; margin-bottom: 1rem;">
                            <p><strong>Config:</strong> ${linkage.clips_per_video} clips/video, ${linkage.min_clip_duration_seconds}-${linkage.max_clip_duration_seconds}s duration</p>
                            <p><strong>Stats:</strong> ${linkage.total_clips_generated} generated, ${linkage.total_clips_posted} posted</p>
                        </div>
                        <div style="display: flex; gap: 0.5rem;">
                            <button onclick="deleteLinkage(${linkage.id})" class="btn btn-danger btn-small">Delete</button>
                            <button onclick="toggleLinkageActive(${linkage.id}, ${!linkage.is_active})"
                                    class="btn btn-small ${linkage.is_active ? 'btn-secondary' : ''}">
                                ${linkage.is_active ? 'Deactivate' : 'Activate'}
                            </button>
                        </div>
                    </div>
                `).join('');
            } catch (error) {
                container.className = 'empty-state';
                container.innerHTML = `<p style="color: #ef4444;">Error loading linkages: ${error.message}</p>`;
            }
        }

        // Load jobs
        async function loadJobs() {
            const container = document.getElementById('jobsContainer');

            try {
                const response = await fetch('/api/clipping/jobs', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();
                const jobs = data.jobs || data || [];

                if (jobs.length === 0) {
                    container.className = 'empty-state';
                    container.innerHTML = `
                        <div class="empty-state-icon">⚙️</div>
                        <h3>No Jobs Running</h3>
                        <p>Jobs will appear here when clipping starts</p>
                    `;
                    return;
                }

                container.innerHTML = jobs.map(job => {
                    const statusClass = job.status === 'completed' ? 'status-completed' :
                                       job.status === 'failed' ? 'status-failed' :
                                       job.status === 'pending' ? 'status-pending' : 'status-running';
                    const pct = job.progress_percent || 0;

                    return `
                        <div class="job-card" id="job-card-${job.id}" data-status="${job.status}">
                            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1rem;">
                                <div>
                                    <h3>${job.source_video_title}</h3>
                                    <p id="job-step-${job.id}" style="color: #94a3b8; font-size: 0.9rem;">${job.current_step || 'Initializing'}</p>
                                </div>
                                <span id="job-status-${job.id}" class="status-badge ${statusClass}">${job.status}</span>
                            </div>
                            <div id="job-progress-wrap-${job.id}">
                                <div class="progress-bar">
                                    <div id="job-progress-bar-${job.id}" class="progress-fill" style="width: ${pct}%"></div>
                                </div>
                                <p id="job-progress-pct-${job.id}" style="text-align: right; color: #94a3b8; font-size: 0.85rem; margin-top: 0.25rem;">
                                    ${Math.round(pct)}%
                                </p>
                            </div>
                            ${job.error_message ? `<p style="color: #ef4444; margin-top: 0.5rem;">${job.error_message}</p>` : ''}
                            <p id="job-detail-${job.id}" style="color: #64748b; font-size: 0.8rem; margin-top: 0.25rem;"></p>
                        </div>
                    `;
                }).join('');

                // Open WebSocket for each active job
                jobs.filter(j => j.status === 'processing' || j.status === 'pending').forEach(job => {
                    openJobWebSocket(job.id);
                });
            } catch (error) {
                container.className = 'empty-state';
                container.innerHTML = `<p style="color: #ef4444;">Error loading jobs: ${error.message}</p>`;
            }
        }

        // Load clips
        async function loadClips() {
            const container = document.getElementById('clipsContainer');
            container.className = 'loading';
            container.innerHTML = 'Loading clips...';

            try {
                const response = await fetch('/api/clipping/clips', {
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                const data = await response.json();
                const clips = data.clips || data || [];

                if (clips.length === 0) {
                    container.className = 'empty-state';
                    container.innerHTML = `
                        <div class="empty-state-icon">🎬</div>
                        <h3>No Clips Yet</h3>
                        <p>Generated clips will appear here</p>
                    `;
                    return;
                }

                container.className = 'grid';
                container.innerHTML = clips.map(clip => {
                    const statusClass = clip.upload_status === 'published' ? 'status-completed' :
                                       clip.upload_status === 'failed' ? 'status-failed' :
                                       clip.upload_status === 'uploading' ? 'status-running' : 'status-pending';

                    return `
                        <div class="clip-card">
                            <div class="clip-thumbnail">
                                🎬 ${clip.duration_seconds}s
                            </div>
                            <h3 style="margin-bottom: 0.5rem;">${clip.ai_title}</h3>
                            <span class="status-badge ${statusClass}">${clip.upload_status}</span>
                            ${clip.viral_factors ? `
                                <div class="viral-factors">
                                    ${JSON.parse(clip.viral_factors).map(factor =>
                                        `<span class="viral-factor-badge">${factor}</span>`
                                    ).join('')}
                                </div>
                            ` : ''}
                            ${clip.youtube_url ? `
                                <a href="${clip.youtube_url}" target="_blank" class="btn btn-small" style="margin-top: 0.5rem; width: 100%;">
                                    View on YouTube
                                </a>
                            ` : ''}
                        </div>
                    `;
                }).join('');
            } catch (error) {
                container.className = 'empty-state';
                container.innerHTML = `<p style="color: #ef4444;">Error loading clips: ${error.message}</p>`;
            }
        }

        // Channel search
        async function searchChannels(query) {
            if (!query || query.length < 2) {
                document.getElementById('channelSearchResults').innerHTML = '';
                return;
            }

            clearTimeout(searchTimeout);
            searchTimeout = setTimeout(async () => {
                try {
                    const response = await fetch(`/api/youtube/search-channels?q=${encodeURIComponent(query)}`, {
                        headers: { 'Authorization': 'Bearer ' + authToken }
                    });
                    const data = await response.json();

                    const resultsHtml = data.channels.map(channel => `
                        <div class="search-result-item" onclick="selectChannel('${channel.channel_id}', '${channel.channel_name.replace(/'/g, "\\'")}')">
                            <img src="${channel.thumbnail_url || '/placeholder.png'}"
                                 alt="${channel.channel_name}"
                                 style="width: 50px; height: 50px; border-radius: 50%;">
                            <div>
                                <strong>${channel.channel_name}</strong>
                                <p style="color: #94a3b8; font-size: 0.9rem;">${channel.description || 'No description'}</p>
                            </div>
                        </div>
                    `).join('');

                    document.getElementById('channelSearchResults').innerHTML = resultsHtml || '<p style="color: #94a3b8;">No channels found</p>';
                } catch (error) {
                    document.getElementById('channelSearchResults').innerHTML = `<p style="color: #ef4444;">Error: ${error.message}</p>`;
                }
            }, 500);
        }

        async function selectChannel(channelId, channelName) {
            try {
                const response = await fetch('/api/clipping/source-channels', {
                    method: 'POST',
                    headers: {
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify({
                        channel_id: channelId,
                        polling_interval_minutes: 30
                    })
                });

                if (response.ok) {
                    closeAddChannelModal();
                    loadSourceChannels();
                } else {
                    const error = await response.json();
                    alert('Error adding channel: ' + error.message);
                }
            } catch (error) {
                alert('Error: ' + error.message);
            }
        }

        // Linkage creation
        async function createLinkage() {
            const sourceChannelId = document.getElementById('linkageSourceChannel').value;
            const destChannelId = document.getElementById('linkageDestChannel').value;
            const clipsPerVideo = document.getElementById('linkageClipsPerVideo').value;
            const minDuration = document.getElementById('linkageMinDuration').value;
            const maxDuration = document.getElementById('linkageMaxDuration').value;

            try {
                const response = await fetch('/api/clipping/linkages', {
                    method: 'POST',
                    headers: {
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify({
                        source_channel_id: parseInt(sourceChannelId),
                        destination_channel_id: parseInt(destChannelId),
                        clips_per_video: parseInt(clipsPerVideo),
                        min_clip_duration_seconds: parseInt(minDuration),
                        max_clip_duration_seconds: parseInt(maxDuration)
                    })
                });

                if (response.ok) {
                    closeCreateLinkageModal();
                    loadLinkages();
                } else {
                    const error = await response.json();
                    alert('Error creating linkage: ' + error.message);
                }
            } catch (error) {
                alert('Error: ' + error.message);
            }
        }

        // Modal functions
        function openAddChannelModal() {
            document.getElementById('addChannelModal').classList.add('active');
        }

        function closeAddChannelModal() {
            document.getElementById('addChannelModal').classList.remove('active');
            document.getElementById('channelSearchInput').value = '';
            document.getElementById('channelSearchResults').innerHTML = '';
        }

        async function openCreateLinkageModal() {
            // Load source channels
            const sourcesResponse = await fetch('/api/clipping/source-channels', {
                headers: { 'Authorization': 'Bearer ' + authToken }
            });
            const sourcesData = await sourcesResponse.json();
            const sources = sourcesData.channels || sourcesData || [];

            // Load user's YouTube channels
            const channelsResponse = await fetch('/api/youtube/channels', {
                headers: { 'Authorization': 'Bearer ' + authToken }
            });
            const channelsData = await channelsResponse.json();
            const channels = channelsData.channels || channelsData || [];

            document.getElementById('linkageSourceChannel').innerHTML = sources.map(s =>
                `<option value="${s.id}">${s.channel_name}</option>`
            ).join('');

            document.getElementById('linkageDestChannel').innerHTML = channels.map(c =>
                `<option value="${c.id}">${c.channel_name}</option>`
            ).join('');

            document.getElementById('createLinkageModal').classList.add('active');
        }

        function closeCreateLinkageModal() {
            document.getElementById('createLinkageModal').classList.remove('active');
        }

        // Delete functions
        async function deleteSourceChannel(id) {
            if (!confirm('Are you sure you want to delete this source channel?')) return;

            try {
                await fetch(`/api/clipping/source-channels/${id}`, {
                    method: 'DELETE',
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                loadSourceChannels();
            } catch (error) {
                alert('Error: ' + error.message);
            }
        }

        async function deleteLinkage(id) {
            if (!confirm('Are you sure you want to delete this linkage?')) return;

            try {
                await fetch(`/api/clipping/linkages/${id}`, {
                    method: 'DELETE',
                    headers: { 'Authorization': 'Bearer ' + authToken }
                });
                loadLinkages();
            } catch (error) {
                alert('Error: ' + error.message);
            }
        }

        // Toggle active status
        async function toggleChannelActive(id, isActive) {
            try {
                await fetch(`/api/clipping/source-channels/${id}`, {
                    method: 'PATCH',
                    headers: {
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify({ is_active: isActive })
                });
                loadSourceChannels();
            } catch (error) {
                alert('Error: ' + error.message);
            }
        }

        async function toggleLinkageActive(id, isActive) {
            try {
                await fetch(`/api/clipping/linkages/${id}`, {
                    method: 'PATCH',
                    headers: {
                        'Authorization': 'Bearer ' + authToken,
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify({ is_active: isActive })
                });
                loadLinkages();
            } catch (error) {
                alert('Error: ' + error.message);
            }
        }

        // Auto-refresh jobs every 5 seconds
        setInterval(() => {
            if (document.getElementById('jobsTab').classList.contains('active')) {
                loadJobs();
            }
        }, 5000);

        // Initial load
        loadSourceChannels();
    </script>
</body>
</html>
    "###;
    Html(html.to_string())
}

// ============================================================================
// Video Tools Page — SSR interactive tool UI (stabilize, convert, visualize, workflows)
// ============================================================================

pub async fn video_tools_page() -> Html<String> {
    let html = r###"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Video Tools — VideoSync</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f1419 100%);
            background-attachment: fixed;
            min-height: 100vh;
            color: #e8e8e8;
            padding: 20px;
        }
        .topbar {
            max-width: 900px; margin: 0 auto 20px;
            display: flex; justify-content: space-between; align-items: center;
        }
        .topbar a { color: #3b82f6; text-decoration: none; font-weight: 600; }
        h1 { color: #fff; font-size: 2rem; }
        .subtitle { color: #9ca3af; margin-bottom: 24px; }
        .container { max-width: 900px; margin: 0 auto; }
        .tabs { display: flex; gap: 8px; margin-bottom: 24px; flex-wrap: wrap; }
        .tab-btn {
            padding: 8px 20px; border-radius: 20px; border: 1px solid rgba(59,130,246,0.4);
            background: rgba(59,130,246,0.1); color: #93c5fd; cursor: pointer;
            font-size: 0.875rem; transition: all 0.2s;
        }
        .tab-btn.active, .tab-btn:hover {
            background: rgba(59,130,246,0.3); border-color: #3b82f6; color: #fff;
        }
        .panel { display: none; }
        .panel.active { display: block; }
        .card {
            background: rgba(26,26,46,0.95); border-radius: 12px;
            border: 1px solid rgba(59,130,246,0.2); padding: 28px;
        }
        .card p { color: #9ca3af; margin-bottom: 20px; font-size: 0.9rem; line-height: 1.6; }
        .form-group { margin-bottom: 18px; }
        label { display: block; margin-bottom: 6px; font-size: 0.875rem; color: #d1d5db; }
        input[type=text], input[type=number], select {
            width: 100%; padding: 10px 14px; background: rgba(255,255,255,0.05);
            border: 1px solid rgba(255,255,255,0.15); border-radius: 8px;
            color: #e8e8e8; font-size: 0.9rem; outline: none;
        }
        input:focus, select:focus { border-color: #3b82f6; }
        select option { background: #1e293b; }
        .slider-row { display: flex; align-items: center; gap: 12px; }
        .slider-row input[type=range] { flex: 1; }
        .slider-val { min-width: 30px; text-align: right; color: #3b82f6; font-weight: 600; }
        .two-col { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
        .btn {
            padding: 12px 28px; background: #3b82f6; color: #fff;
            border: none; border-radius: 8px; cursor: pointer; font-size: 0.9rem;
            font-weight: 600; transition: background 0.2s; width: 100%;
        }
        .btn:hover { background: #2563eb; }
        .btn:disabled { background: #374151; color: #6b7280; cursor: not-allowed; }
        .result { margin-top: 18px; padding: 14px; border-radius: 8px; font-size: 0.875rem; }
        .result.ok { background: rgba(34,197,94,0.1); border: 1px solid rgba(34,197,94,0.3); color: #86efac; }
        .result.err { background: rgba(239,68,68,0.1); border: 1px solid rgba(239,68,68,0.3); color: #fca5a5; }
        .download-link { display: inline-block; margin-top: 10px; color: #3b82f6; font-weight: 600; }
        .workflow-info { background: rgba(59,130,246,0.08); border: 1px solid rgba(59,130,246,0.2);
            border-radius: 6px; padding: 10px 14px; margin: 10px 0; font-size: 0.8rem; color: #93c5fd; }
        .gif-opts { margin-top: 14px; padding-top: 14px; border-top: 1px solid rgba(255,255,255,0.1); }
        code { font-size: 0.85em; background: rgba(255,255,255,0.08); padding: 2px 5px; border-radius: 3px; }
    </style>
</head>
<body>
    <div class="container">
        <div class="topbar">
            <a href="/dashboard">← Dashboard</a>
            <a href="/chat">💬 AI Chat</a>
        </div>
        <h1>🛠️ Video Tools</h1>
        <p class="subtitle">
            Direct FFmpeg tool access. Enter a file path relative to <code>uploads/</code>
            (e.g. <code>abc123_file.mp4</code>) — the name shown after upload in the chat.
        </p>

        <div class="tabs">
            <button class="tab-btn active" onclick="showTab('stabilize',this)">🎥 Stabilize</button>
            <button class="tab-btn" onclick="showTab('convert',this)">🔄 Convert Format</button>
            <button class="tab-btn" onclick="showTab('visualize',this)">🎵 Audio Visualizer</button>
            <button class="tab-btn" onclick="showTab('workflow',this)">⚡ Workflows</button>
        </div>

        <!-- ── Stabilize ───────────────────────────────────────────────── -->
        <div id="panel-stabilize" class="panel active">
            <div class="card">
                <p>Remove camera shake using 2-pass vid.stab analysis. Higher shakiness detects more motion; higher smoothing = more stable but more border crop.</p>
                <div class="form-group">
                    <label>Input file path</label>
                    <input type="text" id="stab-input" placeholder="uploads/my_video.mp4">
                </div>
                <div class="form-group">
                    <label>Shakiness: <span id="stab-shake-val">5</span></label>
                    <div class="slider-row">
                        <input type="range" id="stab-shake" min="1" max="10" value="5"
                            oninput="document.getElementById('stab-shake-val').textContent=this.value">
                        <span class="slider-val">1–10</span>
                    </div>
                </div>
                <div class="form-group">
                    <label>Smoothing: <span id="stab-smooth-val">10</span></label>
                    <div class="slider-row">
                        <input type="range" id="stab-smooth" min="1" max="50" value="10"
                            oninput="document.getElementById('stab-smooth-val').textContent=this.value">
                        <span class="slider-val">1–50</span>
                    </div>
                </div>
                <div class="form-group">
                    <label>Zoom: <span id="stab-zoom-val">0</span>%</label>
                    <div class="slider-row">
                        <input type="range" id="stab-zoom" min="0" max="20" value="0"
                            oninput="document.getElementById('stab-zoom-val').textContent=this.value">
                        <span class="slider-val">0–20%</span>
                    </div>
                </div>
                <button class="btn" id="stab-btn" onclick="runStabilize()">Stabilize Video</button>
                <div id="stab-result"></div>
            </div>
        </div>

        <!-- ── Convert Format ─────────────────────────────────────────── -->
        <div id="panel-convert" class="panel">
            <div class="card">
                <p>Convert to a different container format. Audio-only formats (mp3, wav, flac) extract the audio track.</p>
                <div class="form-group">
                    <label>Input file path</label>
                    <input type="text" id="conv-input" placeholder="uploads/my_video.mp4">
                </div>
                <div class="form-group">
                    <label>Target format</label>
                    <select id="conv-format">
                        <option value="mp4">MP4 (H.264)</option>
                        <option value="mkv">MKV (Matroska)</option>
                        <option value="webm">WebM (VP8/Vorbis)</option>
                        <option value="mov">MOV (QuickTime)</option>
                        <option value="avi">AVI</option>
                        <option value="ts">MPEG-TS</option>
                        <option value="mp3">MP3 (audio only)</option>
                        <option value="aac">AAC (audio only)</option>
                        <option value="flac">FLAC (lossless audio)</option>
                        <option value="wav">WAV (uncompressed)</option>
                        <option value="m4a">M4A (AAC in MP4)</option>
                    </select>
                </div>
                <button class="btn" id="conv-btn" onclick="runConvert()">Convert Format</button>
                <div id="conv-result"></div>
            </div>
        </div>

        <!-- ── Audio Visualizer ───────────────────────────────────────── -->
        <div id="panel-visualize" class="panel">
            <div class="card">
                <p>Generate an audio visualization video from any audio or video file. Result is an MP4 you can download.</p>
                <div class="form-group">
                    <label>Input file path</label>
                    <input type="text" id="viz-input" placeholder="uploads/my_audio.wav">
                </div>
                <div class="form-group">
                    <label>Visualization mode</label>
                    <select id="viz-mode">
                        <option value="waveform">Waveform (amplitude over time)</option>
                        <option value="spectrum">Spectrum (frequency intensity)</option>
                        <option value="cqt">CQT (musical frequency bands)</option>
                    </select>
                </div>
                <div class="two-col">
                    <div class="form-group">
                        <label>Width (px)</label>
                        <input type="number" id="viz-width" value="1280" min="320" max="3840" step="160">
                    </div>
                    <div class="form-group">
                        <label>Height (px)</label>
                        <input type="number" id="viz-height" value="400" min="100" max="1080" step="100">
                    </div>
                </div>
                <button class="btn" id="viz-btn" onclick="runVisualize()">Generate Visualization</button>
                <div id="viz-result"></div>
            </div>
        </div>

        <!-- ── Workflows ──────────────────────────────────────────────── -->
        <div id="panel-workflow" class="panel">
            <div class="card">
                <p>Named multi-step workflow chains that apply several FFmpeg tools in sequence.</p>
                <div class="form-group">
                    <label>Input file path</label>
                    <input type="text" id="wf-input" placeholder="uploads/my_video.mp4">
                </div>
                <div class="form-group">
                    <label>Workflow</label>
                    <select id="wf-select" onchange="updateWorkflowInfo()">
                        <option value="youtube_ready">YouTube Ready Export</option>
                        <option value="podcast_cleanup">Podcast Cleanup</option>
                        <option value="cinematic_grade">Cinematic Grade</option>
                        <option value="talking_head_cleanup">Talking Head Cleanup</option>
                        <option value="create_gif">Create GIF</option>
                    </select>
                </div>
                <div class="workflow-info" id="wf-info">
                    Stabilize → normalize color → loudnorm −14 LUFS → yuv420p
                </div>
                <div class="gif-opts" id="gif-opts" style="display:none;">
                    <div class="two-col">
                        <div class="form-group">
                            <label>Start (s)</label>
                            <input type="number" id="gif-start" value="0" min="0" step="1">
                        </div>
                        <div class="form-group">
                            <label>Duration (s)</label>
                            <input type="number" id="gif-dur" value="5" min="1" max="60" step="1">
                        </div>
                    </div>
                    <div class="two-col">
                        <div class="form-group">
                            <label>Width (px)</label>
                            <input type="number" id="gif-w" value="480" min="120" max="960" step="40">
                        </div>
                        <div class="form-group">
                            <label>FPS</label>
                            <input type="number" id="gif-fps" value="15" min="5" max="30" step="1">
                        </div>
                    </div>
                </div>
                <button class="btn" id="wf-btn" onclick="runWorkflow()">Run Workflow</button>
                <div id="wf-result"></div>
            </div>
        </div>
    </div>

    <script>
        const WORKFLOW_INFO = {
            youtube_ready: 'Stabilize → normalize color → loudnorm −14 LUFS → yuv420p',
            podcast_cleanup: 'Denoise → de-ess sibilance → limit peaks → loudnorm −16 LUFS',
            cinematic_grade: 'Vintage curves → vibrance → vignette → film grain',
            talking_head_cleanup: 'Stabilize → denoise speech → de-ess → loudnorm −16 LUFS',
            create_gif: 'Trim segment → scale → optimize palette (GIF output)',
        };

        function showTab(name, btn) {
            document.querySelectorAll('.panel').forEach(p => p.classList.remove('active'));
            document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
            document.getElementById('panel-' + name).classList.add('active');
            btn.classList.add('active');
        }

        function updateWorkflowInfo() {
            const wf = document.getElementById('wf-select').value;
            document.getElementById('wf-info').textContent = WORKFLOW_INFO[wf] || '';
            document.getElementById('gif-opts').style.display = wf === 'create_gif' ? 'block' : 'none';
        }

        function getAuthHeader() {
            const token = localStorage.getItem('auth_token') || localStorage.getItem('authToken');
            return token ? { 'Authorization': 'Bearer ' + token } : {};
        }

        function showResult(id, result) {
            const el = document.getElementById(id);
            if (result.success) {
                el.className = 'result ok';
                el.innerHTML = escHtml(result.message)
                    + (result.download_url
                        ? '<br><a class="download-link" href="' + escHtml(result.download_url) + '" target="_blank">⬇ Download result</a>'
                        : '');
            } else {
                el.className = 'result err';
                el.textContent = result.message;
            }
        }

        function escHtml(s) {
            return String(s || '').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
        }

        async function postTool(endpoint, body, btnId, resultId) {
            const btn = document.getElementById(btnId);
            btn.disabled = true;
            btn.textContent = 'Processing…';
            document.getElementById(resultId).className = '';
            document.getElementById(resultId).textContent = '';
            try {
                const res = await fetch(endpoint, {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json', ...getAuthHeader() },
                    body: JSON.stringify(body),
                });
                const data = await res.json();
                showResult(resultId, data);
            } catch (e) {
                showResult(resultId, { success: false, message: 'Request failed: ' + e.message });
            } finally {
                btn.disabled = false;
                btn.textContent = btn.getAttribute('data-label') || 'Run';
            }
        }

        // Set default labels
        document.getElementById('stab-btn').setAttribute('data-label', 'Stabilize Video');
        document.getElementById('conv-btn').setAttribute('data-label', 'Convert Format');
        document.getElementById('viz-btn').setAttribute('data-label', 'Generate Visualization');
        document.getElementById('wf-btn').setAttribute('data-label', 'Run Workflow');

        function runStabilize() {
            const input = document.getElementById('stab-input').value.trim();
            if (!input) return alert('Enter an input file path');
            postTool('/api/tools/stabilize', {
                input_file: input,
                shakiness: +document.getElementById('stab-shake').value,
                smoothing: +document.getElementById('stab-smooth').value,
                zoom: +document.getElementById('stab-zoom').value,
            }, 'stab-btn', 'stab-result');
        }

        function runConvert() {
            const input = document.getElementById('conv-input').value.trim();
            if (!input) return alert('Enter an input file path');
            postTool('/api/tools/convert', {
                input_file: input,
                format: document.getElementById('conv-format').value,
            }, 'conv-btn', 'conv-result');
        }

        function runVisualize() {
            const input = document.getElementById('viz-input').value.trim();
            if (!input) return alert('Enter an input file path');
            postTool('/api/tools/visualize-audio', {
                input_file: input,
                mode: document.getElementById('viz-mode').value,
                width: +document.getElementById('viz-width').value,
                height: +document.getElementById('viz-height').value,
            }, 'viz-btn', 'viz-result');
        }

        function runWorkflow() {
            const input = document.getElementById('wf-input').value.trim();
            if (!input) return alert('Enter an input file path');
            const wf = document.getElementById('wf-select').value;
            const body = { input_file: input, workflow: wf };
            if (wf === 'create_gif') {
                body.start_seconds = +document.getElementById('gif-start').value;
                body.duration_seconds = +document.getElementById('gif-dur').value;
                body.gif_width = +document.getElementById('gif-w').value;
                body.gif_fps = +document.getElementById('gif-fps').value;
            }
            postTool('/api/tools/workflow', body, 'wf-btn', 'wf-result');
        }
    </script>
</body>
</html>"###;
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


// ============================================================================
// Manual Clipping Dashboard
// ============================================================================

pub async fn manual_clipping_page() -> Html<String> {
    Html(MANUAL_CLIPPING_HTML.to_string())
}

const MANUAL_CLIPPING_HTML: &str = r#####"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Manual Clipping — VideoSync</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:'Inter',system-ui,sans-serif;background:#1a1a2e;color:#e0e0e0;min-height:100vh}
.header{background:#16213e;border-bottom:1px solid #0f3460;padding:14px 24px;display:flex;align-items:center;justify-content:space-between}
.header h1{font-size:1.1rem;color:#dbd8e3}
.back{color:#5c5470;text-decoration:none;font-size:0.9rem}.back:hover{color:#dbd8e3}
.container{max-width:900px;margin:0 auto;padding:24px}
.card{background:#16213e;border:1px solid #0f3460;border-radius:12px;padding:24px;margin-bottom:20px}
.card h2{font-size:1rem;color:#dbd8e3;margin-bottom:16px}
.url-row{display:flex;gap:12px;margin-bottom:12px}
.url-input{flex:1;padding:10px 14px;background:#0f3460;border:1px solid #1e3a5f;border-radius:8px;color:#e0e0e0;font-size:0.95rem}
.url-input:focus{outline:none;border-color:#5c5470}
.cfg-row{display:grid;grid-template-columns:repeat(3,1fr);gap:12px;margin-bottom:14px}
label{font-size:0.8rem;color:#9ca3af;display:block;margin-bottom:4px}
input[type=number]{width:100%;padding:8px 12px;background:#0f3460;border:1px solid #1e3a5f;border-radius:6px;color:#e0e0e0;font-size:0.9rem}
.btn{padding:10px 22px;border:none;border-radius:8px;cursor:pointer;font-size:0.9rem;font-weight:600}
.btn-primary{background:#5c5470;color:#fff}.btn-primary:hover{background:#7a6e8a}
.btn-sm{padding:5px 12px;font-size:0.8rem;border-radius:5px}
.btn-download{background:#065f46;color:#6ee7b7}.btn-download:hover{background:#047857}
.btn-cancel{background:#7f1d1d;color:#fca5a5}
.job-row{display:flex;align-items:flex-start;gap:12px;padding:14px;border-bottom:1px solid #0f3460}
.job-row:last-child{border-bottom:none}
.job-meta{flex:1}
.job-url{color:#dbd8e3;font-size:0.9rem;margin-bottom:4px;word-break:break-all}
.job-status{font-size:0.8rem;color:#9ca3af}
.status-dot{display:inline-block;width:8px;height:8px;border-radius:50%;margin-right:6px}
.status-pending,.status-analyzing,.status-downloading,.status-extracting,.status-uploading{background:#f59e0b}
.status-completed{background:#10b981}
.status-failed,.status-cancelled{background:#ef4444}
.clips-grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(190px,1fr));gap:12px;margin-top:12px}
.clip-card{background:#0f3460;border:1px solid #1e3a5f;border-radius:8px;overflow:hidden}
.clip-thumb{width:100%;aspect-ratio:16/9;object-fit:cover}
.clip-thumb-ph{width:100%;aspect-ratio:16/9;background:#1a1a2e;display:flex;align-items:center;justify-content:center;color:#5c5470;font-size:1.5rem}
.clip-info{padding:8px 10px}
.clip-title{font-size:0.82rem;color:#dbd8e3;margin-bottom:4px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.clip-meta{font-size:0.75rem;color:#9ca3af;margin-bottom:8px}
.progress-bar{height:4px;background:#0f3460;border-radius:2px;overflow:hidden;margin-top:6px}
.progress-fill{height:100%;background:#5c5470;transition:width 0.3s}
.msg{padding:10px 16px;border-radius:8px;margin-bottom:12px;font-size:0.9rem}
.msg-success{background:#065f46;color:#6ee7b7}
.msg-error{background:#7f1d1d;color:#fca5a5}
.empty{text-align:center;padding:40px;color:#5c5470}
.platform-badge{font-size:0.72rem;padding:2px 7px;border-radius:4px;font-weight:600;margin-left:8px}
.yt-badge{background:#7f1d1d;color:#fca5a5}
.tw-badge{background:#2d1b69;color:#c4b5fd}
</style>
</head>
<body>
<div class="header">
  <div style="display:flex;align-items:center;gap:16px">
    <a href="/dashboard" id="back-link" class="back">Back</a>
    <h1>Manual Clipping</h1>
  </div>
  <span id="user-name" style="color:#9ca3af;font-size:0.85rem"></span>
</div>
<div class="container">
  <div id="msg"></div>
  <div class="card">
    <h2>Clip a Video</h2>
    <div class="url-row">
      <input class="url-input" id="video_url" placeholder="Paste YouTube or Twitch URL here" oninput="detectPlatform()">
      <button class="btn btn-primary" onclick="submitJob()">Clip It</button>
    </div>
    <div id="platform-hint" style="font-size:0.82rem;color:#9ca3af;margin-bottom:12px"></div>
    <div class="cfg-row">
      <div><label>Clips (1-5)</label><input type="number" id="clips_count" value="3" min="1" max="5"></div>
      <div><label>Min length (s)</label><input type="number" id="min_duration" value="30" min="10" max="300"></div>
      <div><label>Max length (s)</label><input type="number" id="max_duration" value="120" min="30" max="600"></div>
    </div>
    <p style="font-size:0.78rem;color:#5c5470">AI finds the best moments and generates download-ready clips. No YouTube channel required.</p>
  </div>
  <div class="card">
    <h2>Your Jobs</h2>
    <div id="jobs-area"><div class="empty">Loading...</div></div>
  </div>
</div>
<script>
const token = localStorage.getItem('auth_token') || localStorage.getItem('authToken') || localStorage.getItem('auth_token') || localStorage.getItem('admin_token');
if (!token) window.location.href = '/login';
function parseJwt(t){try{return JSON.parse(atob(t.split('.')[1]));}catch(e){return{};}}
const claims = parseJwt(token);
if(claims.username) document.getElementById('user-name').textContent = claims.username;
if(!claims.is_clipper) document.getElementById('back-link').href='/dashboard';
function showMsg(text,ok=true){const el=document.getElementById('msg');el.innerHTML=`<div class="msg ${ok?'msg-success':'msg-error'}">${text}</div>`;setTimeout(()=>el.innerHTML='',4000);}
function detectPlatform(){const url=document.getElementById('video_url').value;const hint=document.getElementById('platform-hint');if(!url){hint.textContent='';return;}if(url.includes('twitch.tv'))hint.textContent='Twitch VOD detected';else if(url.includes('youtube.com')||url.includes('youtu.be'))hint.textContent='YouTube video detected';else hint.textContent='';}
async function submitJob(){const video_url=document.getElementById('video_url').value.trim();if(!video_url){showMsg('Please paste a URL',false);return;}const payload={video_url,clips_count:parseInt(document.getElementById('clips_count').value)||3,min_duration:parseInt(document.getElementById('min_duration').value)||30,max_duration:parseInt(document.getElementById('max_duration').value)||120};const res=await fetch('/api/manual-clipping/jobs',{method:'POST',headers:{'Authorization':'Bearer '+token,'Content-Type':'application/json'},body:JSON.stringify(payload)});const data=await res.json();if(data.success){showMsg('Job created! AI is analyzing...');document.getElementById('video_url').value='';loadJobs();}else showMsg(data.message||'Failed',false);}
let activeRefresh=null;
async function loadJobs(){const res=await fetch('/api/manual-clipping/jobs',{headers:{'Authorization':'Bearer '+token}});const data=await res.json();if(!data.success){document.getElementById('jobs-area').innerHTML='<div class="empty">Failed to load</div>';return;}const jobs=data.jobs||[];if(!jobs.length){document.getElementById('jobs-area').innerHTML='<div class="empty">No jobs yet. Paste a URL above to get started.</div>';return;}let html='';let hasActive=false;for(const j of jobs){const active=['pending','analyzing','downloading','extracting','uploading'].includes(j.status);if(active)hasActive=true;const platBadge=j.video_platform==='twitch'?'<span class="platform-badge tw-badge">TWITCH</span>':'<span class="platform-badge yt-badge">YOUTUBE</span>';html+=`<div class="job-row" id="job-${j.id}"><div class="job-meta"><div class="job-url">${j.video_title||j.video_url}${platBadge}</div><div class="job-status"><span class="status-dot status-${j.status}"></span>${j.status.toUpperCase()}${active?` - ${j.progress_percent}%`:''}${j.error_message?` <span style="color:#ef4444">${j.error_message}</span>`:''}</div>${active?`<div class="progress-bar"><div class="progress-fill" style="width:${j.progress_percent}%"></div></div>`:''} ${j.status==='completed'?`<div id="clips-${j.id}" style="margin-top:12px"></div>`:''}</div>${j.status==='completed'?`<button class="btn btn-sm btn-download" onclick="loadClips('${j.id}')">Load Clips</button>`:''}${active?`<button class="btn btn-sm btn-cancel" onclick="cancelJob('${j.id}')">Cancel</button>`:''}</div>`;}
document.getElementById('jobs-area').innerHTML=html;if(activeRefresh)clearTimeout(activeRefresh);if(hasActive)activeRefresh=setTimeout(loadJobs,5000);}
async function loadClips(jobId){const res=await fetch(`/api/manual-clipping/jobs/${jobId}`,{headers:{'Authorization':'Bearer '+token}});const data=await res.json();if(!data.success)return;const clips=data.clips||[];if(!clips.length)return;let html='<div class="clips-grid">';for(const c of clips){const dur=c.duration_seconds?Math.round(c.duration_seconds)+'s':'';html+=`<div class="clip-card">${c.thumbnail_url?`<img class="clip-thumb" src="${c.thumbnail_url}" loading="lazy">`:'<div class="clip-thumb-ph">🎬</div>'}<div class="clip-info"><div class="clip-title">${c.title||'Clip '+c.clip_number}</div><div class="clip-meta">${dur}</div>${c.download_url?`<a href="${c.download_url}" download class="btn btn-download" style="display:block;text-align:center;text-decoration:none;font-size:0.82rem;padding:7px">Download</a>`:'<span style="color:#9ca3af;font-size:0.8rem">Link expired</span>'}</div></div>`;}html+='</div>';const el=document.getElementById(`clips-${jobId}`);if(el)el.innerHTML=html;}
async function cancelJob(id){const res=await fetch(`/api/manual-clipping/jobs/${id}`,{method:'DELETE',headers:{'Authorization':'Bearer '+token}});const data=await res.json();if(data.success)loadJobs();else showMsg('Could not cancel',false);}
loadJobs();
</script>
</body>
</html>"#####;

// ============================================================================
// Clipper Invite Signup Page
// ============================================================================

pub async fn clipper_signup_page() -> Html<String> {
    Html(CLIPPER_SIGNUP_HTML.to_string())
}

const CLIPPER_SIGNUP_HTML: &str = r#####"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Join as Clipper — VideoSync</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:'Inter',system-ui,sans-serif;background:#1a1a2e;color:#e0e0e0;min-height:100vh;display:flex;align-items:center;justify-content:center}
.card{background:#16213e;border:1px solid #0f3460;border-radius:16px;padding:40px;width:100%;max-width:420px;margin:20px}
.logo{font-size:1.4rem;font-weight:700;color:#dbd8e3;margin-bottom:6px}
.subtitle{color:#9ca3af;font-size:0.9rem;margin-bottom:24px}
label{font-size:0.82rem;color:#9ca3af;display:block;margin-bottom:4px}
input{width:100%;padding:10px 14px;background:#0f3460;border:1px solid #1e3a5f;border-radius:8px;color:#e0e0e0;font-size:0.9rem;margin-bottom:14px}
input:focus{outline:none;border-color:#5c5470}
.btn{width:100%;padding:12px;background:#5c5470;color:#fff;border:none;border-radius:8px;cursor:pointer;font-size:0.95rem;font-weight:600;margin-top:4px}
.btn:hover{background:#7a6e8a}
.msg{padding:10px 14px;border-radius:8px;margin-bottom:14px;font-size:0.85rem}
.msg-success{background:#065f46;color:#6ee7b7}
.msg-error{background:#7f1d1d;color:#fca5a5}
.note{font-size:0.78rem;color:#5c5470;margin-top:12px;text-align:center}
</style>
</head>
<body>
<div class="card">
  <div class="logo">VideoSync</div>
  <div class="subtitle">Create your Clipper account</div>
  <div id="msg"></div>
  <form onsubmit="register(event)">
    <label>Invite Token</label>
    <input id="token" placeholder="Your invite token" required>
    <label>Email</label>
    <input type="email" id="email" placeholder="you@example.com" required>
    <label>Username</label>
    <input id="username" placeholder="clipper_name" required>
    <label>Password</label>
    <input type="password" id="password" placeholder="Min 6 characters" required>
    <label>Confirm Password</label>
    <input type="password" id="confirm_password" required>
    <button type="submit" class="btn">Create Account</button>
  </form>
  <p class="note">Already have an account? <a href="/login" style="color:#dbd8e3">Sign in</a></p>
</div>
<script>
const params=new URLSearchParams(window.location.search);
if(params.get('token'))document.getElementById('token').value=params.get('token');
function showMsg(text,ok=true){document.getElementById('msg').innerHTML=`<div class="msg ${ok?'msg-success':'msg-error'}">${text}</div>`;}
async function register(e){e.preventDefault();const payload={token:document.getElementById('token').value.trim(),email:document.getElementById('email').value.trim(),username:document.getElementById('username').value.trim(),password:document.getElementById('password').value,confirm_password:document.getElementById('confirm_password').value};const res=await fetch('/api/auth/register/clipper',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(payload)});const data=await res.json();if(data.success){localStorage.setItem('authToken',data.token);localStorage.setItem('user',JSON.stringify(data.user));showMsg('Account created! Redirecting...');setTimeout(()=>window.location.href='/manual-clipping',1200);}else showMsg(data.message||'Registration failed',false);}
</script>
</body>
</html>"#####;
