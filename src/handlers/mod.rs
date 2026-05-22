// src/handlers/mod.rs
pub mod admin;
pub mod api_access; // 💳 USDC subscription page (x402 paywall on /api-access)
pub mod auth;
pub mod background;
pub mod background_routes;
pub mod chat;
pub mod clipping; // 📹 YouTube clipping feature
pub mod gig_templates; // 💼 Fiverr/PPH gig templates + sample generation
pub mod health; // 🏥 Health check and monitoring endpoints
pub mod jobs; // 🆕 Job control endpoints
pub mod manual_clipping; // ✂️ Manual clipping — paste URL, get download links
pub mod output;
pub mod prospects; // 🎯 Admin prospect finder
pub mod service_catalog;
pub mod subscribe;
pub mod tools; // 🎬 On-demand FFmpeg tool endpoints
pub mod ui;
pub mod upload;
pub mod youtube; // 📺 YouTube integration // 💳 Regular-user $15/mo USDC subscription (post 7-day trial)
