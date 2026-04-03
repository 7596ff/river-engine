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

    axum::serve(listener, app).await?;

    Ok(())
}
