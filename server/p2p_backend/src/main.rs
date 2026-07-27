use axum::{
    routing::{get, post},
    Router,
    Json,
};
use std::net::SocketAddr;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct BlockRequest {
    hash: String,
}

#[derive(Serialize, Deserialize)]
struct BlockResponse {
    data: Vec<u8>,
    signature: String,
}

#[derive(Serialize, Deserialize)]
struct SignatureVerifyRequest {
    message: String,
    signature: String,
    public_key: String,
}

#[derive(Serialize, Deserialize)]
struct SignatureVerifyResponse {
    valid: bool,
}

async fn get_block(Json(_payload): Json<BlockRequest>) -> Json<BlockResponse> {
    // Mock BitTorrent-style block sharing
    Json(BlockResponse {
        data: vec![0, 1, 2, 3], // Mock data
        signature: "mock_signature".to_string(),
    })
}

async fn verify_signature(Json(_payload): Json<SignatureVerifyRequest>) -> Json<SignatureVerifyResponse> {
    // Mock cryptographic signature verification (normally using ed25519-dalek)
    Json(SignatureVerifyResponse {
        valid: true,
    })
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/block", post(get_block))
        .route("/verify", post(verify_signature))
        .route("/health", get(|| async { "OK" }));

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    println!("Listening on {}", addr);
    
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
