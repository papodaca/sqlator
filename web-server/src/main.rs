mod handlers;
mod state;
mod ws_query;

use axum::{
    routing::{get, post},
    Router,
};
use clap::Parser;
use state::{parse_config_file, AppState};
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

    /// Single-database config file. When provided, the UI skips the connection
    /// manager and immediately opens an admin interface for that one database.
    /// File may be JSON {"url":"..."}, YAML with `url:` field, or a bare URL.
    #[arg(short = 'c', long = "config", value_name = "FILE")]
    config: Option<PathBuf>,

    /// Path to the compiled SvelteKit SPA (output of `pnpm build`)
    #[arg(long, default_value = "./build")]
    static_dir: PathBuf,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let state: Arc<AppState> = if let Some(config_path) = &cli.config {
        let (url, name) = match parse_config_file(config_path) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        };
        println!("Single-DB mode: connecting to '{}'", name);
        match AppState::new_with_single_db(url, name).await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                eprintln!("Failed to connect to database: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        match AppState::new() {
            Ok(s) => Arc::new(s),
            Err(e) => {
                eprintln!("Failed to initialize app state: {}", e);
                std::process::exit(1);
            }
        }
    };

    let serve_dir = ServeDir::new(&cli.static_dir)
        .not_found_service(ServeFile::new(cli.static_dir.join("index.html")));

    let app = Router::new()
        .route("/api/query", get(ws_query::handler))
        .route("/api/export-file", get(handlers::export_file))
        .route("/api/:command", post(handlers::dispatch))
        .fallback_service(serve_dir)
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", cli.host, cli.port)
        .parse()
        .expect("invalid bind address");

    println!("SQLator web server listening on http://{}", addr);
    if cli.host == "127.0.0.1" {
        println!("Open your browser at http://localhost:{}", cli.port);
    }

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind");

    axum::serve(listener, app).await.expect("server error");
}
