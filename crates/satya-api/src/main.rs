use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "satya_api=info,tower_http=warn".into()),
        )
        .init();

    let addr = std::env::var("SATYA_BIND").unwrap_or_else(|_| "127.0.0.1:8000".into());
    satya_api::run(&addr).await
}
