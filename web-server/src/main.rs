mod handlers;
mod state;
mod ws_query;

use axum::{
    routing::{get, post},
    Router,
};
use clap::Parser;
use state::AppState;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

#[derive(Parser)]
#[command(name = "sqlator-web", about = "SQLator web server")]
struct Cli {
    /// Port to listen on
    #[arg(short, long, default_value = "3000")]
    port: u16,

    /// Host address to bind (127.0.0.1 by default for local-only access)
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Path to the compiled SvelteKit SPA (output of `pnpm build`)
    #[arg(long, default_value = "./build")]
    static_dir: PathBuf,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let state = match AppState::new() {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("Failed to initialize app state: {}", e);
            std::process::exit(1);
        }
    };

    // SPA fallback: serve index.html for any unknown route (SvelteKit client-side routing)
    let serve_dir = ServeDir::new(&cli.static_dir)
        .not_found_service(ServeFile::new(cli.static_dir.join("index.html")));

    let app = Router::new()
        // WebSocket endpoint for streaming query results
        .route("/api/query", get(ws_query::handler))
        // File download for exported connections
        .route("/api/export-file", get(handlers::export_file))
        // All other commands via POST /api/:command
        .route("/api/:command", post(handlers::dispatch))
        // Serve the compiled Svelte SPA from the static dir
        .fallback_service(serve_dir)
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", cli.host, cli.port)
        .parse()
        .expect("invalid bind address");

    println!("SQLator web server listening on http://{}", addr);
    println!("Open your browser at http://localhost:{}", cli.port);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind");

    axum::serve(listener, app).await.expect("server error");
}
