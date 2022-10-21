use crate::state::State;

use crate::get_timestamp;
use axum::response::Html;
use axum::{routing::get, Router, Server};
use lazy_static::lazy_static;
use tera::{Context, Tera};
use tokio::sync::Mutex;
use tonic::codegen::Arc;

lazy_static! {
    pub static ref TEMPLATES: Tera = {
        let mut tera = match Tera::new("src/interfaces/web/*") {
            Ok(t) => t,
            Err(e) => {
                println!("Parsing error(s): {}", e);
                ::std::process::exit(1);
            }
        };
        tera.autoescape_on(vec![".html", ".sql"]);
        tera
    };
}

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
    let mut context = Context::new();
    let timestamp = get_timestamp();
    let state = state.lock().await;
    let devices = state
        .get_devices()
        .values()
        .map(|device| {
            format!(
                "{} [{}] (active before {}s)",
                device.name(),
                hex::encode(device.identifier()),
                timestamp.saturating_sub(device.last_active())
            )
        })
        .collect::<Vec<String>>();
    context.insert("devices", &devices);

    let groups = state
        .get_groups()
        .values()
        .map(|device| format!("{} [{}]", device.name(), hex::encode(device.identifier())))
        .collect::<Vec<String>>();
    context.insert("groups", &groups);

    let tasks = state
        .get_tasks()
        .iter()
        .map(|(task_id, task)| format!("[{}] ({:?})", task_id, task.get_status()))
        .collect::<Vec<_>>();
    context.insert("tasks", &tasks);

    Html(TEMPLATES.render("dashboard.html", &context).unwrap())
}
