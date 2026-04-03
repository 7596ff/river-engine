//! River Snowflake server binary.

use std::sync::Arc;

use clap::Parser;
use river_snowflake::{server, GeneratorCache};
use tokio::net::TcpListener;

#[derive(Parser)]
#[command(name = "river-snowflake")]
#[command(about = "Snowflake ID generation server")]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value = "4001")]
    port: u16,

    /// Host to bind to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let state = Arc::new(server::AppState {
        cache: GeneratorCache::new(),
    });

    let app = server::router(state);

    let addr = format!("{}:{}", args.host, args.port);
    let listener = TcpListener::bind(&addr).await?;

    eprintln!("Snowflake server listening on {}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    eprintln!("Snowflake server shut down gracefully");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            eprintln!("\nReceived Ctrl+C, shutting down...");
        }
        _ = terminate => {
            eprintln!("\nReceived SIGTERM, shutting down...");
        }
    }
}
