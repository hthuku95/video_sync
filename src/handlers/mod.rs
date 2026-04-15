// src/handlers/mod.rs
pub mod auth;
pub mod chat;
pub mod upload;
pub mod ui;
pub mod admin;
pub mod background;
pub mod background_routes;
pub mod output;
pub mod jobs; // 🆕 Job control endpoints
pub mod youtube; // 📺 YouTube integration
pub mod clipping; // 📹 YouTube clipping feature
pub mod health; // 🏥 Health check and monitoring endpoints
pub mod tools; // 🎬 On-demand FFmpeg tool endpoints
pub mod gig_templates; // 💼 Fiverr/PPH gig templates + sample generation
pub mod manual_clipping; // ✂️ Manual clipping — paste URL, get download links
pub mod prospects; // 🎯 Admin prospect finder
pub mod api_access; // 💳 USDC subscription page (x402 paywall on /api-access)
