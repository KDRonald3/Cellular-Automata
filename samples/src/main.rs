mod server;

#[tokio::main]
async fn main() {
    let runs_dir = std::path::PathBuf::from("runs");
    let app = server::build_router(runs_dir);

    let addr = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind to 127.0.0.1:3000");

    println!("Serving at http://{addr}");

    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let _ = open::that(format!("http://{addr}"));
    });

    axum::serve(listener, app).await.expect("server error");
}
