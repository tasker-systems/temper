//! temper-cloud — Vercel serverless adapter for temper-api.
//!
//! Wraps [`temper_api::create_app`] with the official `vercel_runtime` v2
//! VercelLayer to serve the axum Router as a Vercel serverless function.
//! No migrations at runtime — connect, serve, done.
