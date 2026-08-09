// src/handlers/mod.rs
pub mod admin;
pub mod api_access; // 💳 USDC subscription page (x402 paywall on /api-access)
pub mod auth;
pub mod background;
pub mod background_routes;
pub mod campaigns;
pub mod chat;
pub mod clipping; // 📹 YouTube clipping feature
pub mod crypto_payments; // 💳 USDC on Base payments (x402) for studio offers
pub mod gig_templates; // 💼 Fiverr/PPH gig templates + sample generation
pub mod health; // 🏥 Health check and monitoring endpoints
pub mod jobs; // 🆕 Job control endpoints
pub mod manual_clipping; // ✂️ Manual clipping — paste URL, get download links
pub mod output;
pub mod paypal;
pub mod prospects; // 🎯 Admin prospect finder
pub mod referrals; // 🔗 Referral codes + commissions for content machine users

pub mod social_publish;
pub mod subscribe;
pub mod tools; // 🎬 On-demand FFmpeg tool endpoints
pub mod ui;
pub mod upload;
pub mod website_video; // 🌐 Website-URL→Video credits service ($50/10, $100/30 bundles)
pub mod youtube; // 📺 YouTube integration // 💳 Regular-user $15/mo USDC subscription (post 7-day trial)
