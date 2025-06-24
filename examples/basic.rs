use std::sync::Arc;
use std::time::Duration;

use axum::{Router, routing::get};
use barnacle::{
    barnacle_layer, redis_store::RedisBarnacleStore, types::BarnacleConfig,
};
use redis::Client;

#[tokio::main]
async fn main() {
    // Create Redis client
    let redis_client =
        Arc::new(Client::open("redis://127.0.0.1/").expect("Failed to connect to Redis"));

    // Create rate limiter store
    let store = Arc::new(RedisBarnacleStore::new(redis_client));

    // Configure rate limiting
    let config = BarnacleConfig {
        max_requests: 10,
        window: Duration::from_secs(60),
        backoff: None,
        reset_on_success: false,
    };

    // Create the rate limiting layer
    let barnacle = barnacle_layer(store, config);

    // Build the application
    let app = Router::new()
        .route("/", get(hello_world))
        .layer(barnacle);

    // Run the server
    println!("Server running on http://localhost:3000");
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn hello_world() -> &'static str {
    "Hello, World!"
}
