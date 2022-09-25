use std::fmt::Write;

use crate::state::State;

use axum::response::Html;
use axum::{routing::get, Router, Server};
use tokio::sync::Mutex;
use tonic::codegen::Arc;

pub async fn run_web(state: Arc<Mutex<State>>, addr: &str, port: u16) -> Result<(), String> {
    let app = Router::new().route("/", get(|| root(state)));

    let addr = format!("{}:{}", addr, port)
        .parse()
        .map_err(|_| String::from("Unable to parse web server address"))?;

    Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .map_err(|_| String::from("Unable to run web server"))?;

    Ok(())
}

async fn root(state: Arc<Mutex<State>>) -> Html<String> {
    let mut response = "<h1>MeeSign server</h1>\n".to_string();
    write!(response, "<ul>").unwrap();
    let state = state.lock().await;
    write!(
        response,
        "<li>{} = {}</li>\n",
        "devices count",
        state.get_devices().len()
    )
    .unwrap();
    write!(
        response,
        "<li>{} = {}</li>\n",
        "groups count",
        state.get_groups().len()
    )
    .unwrap();
    write!(
        response,
        "<li>{} = {}</li>\n",
        "tasks count",
        state.get_tasks().len()
    )
    .unwrap();
    write!(response, "</ul>\n").unwrap();
    Html(response)
}
