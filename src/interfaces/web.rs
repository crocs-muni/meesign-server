use axum::{routing::get, Router, Server};

pub async fn run_web(addr: &str, port: u16) -> Result<(), String> {
    let app = Router::new().route("/", get(|| async { "MeeSign Web Server" }));

    let addr = format!("{}:{}", addr, port)
        .parse()
        .map_err(|_| String::from("Unable to parse web server address"))?;

    Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .map_err(|_| String::from("Unable to run web server"))?;

    Ok(())
}
