use crate::get_timestamp;
use crate::state::State;
use crate::tasks::TaskStatus;

use log::info;
use tokio::{sync::Mutex, time};
use tonic::codegen::Arc;

pub async fn run_timer(state: Arc<Mutex<State>>) -> Result<(), String> {
    let mut interval = time::interval(time::Duration::from_secs(1));
    loop {
        interval.tick().await;
        let mut state = state.lock().await;
        let mut restarts = Vec::new();
        let timestamp = get_timestamp();
        for (task_id, task) in state.get_tasks() {
            if task.get_status() != TaskStatus::Finished
                && task.is_approved()
                && timestamp - task.last_update() > 30
            {
                info!("Stale task detected {:?} – queueing for restart", task_id);
                restarts.push(task_id.clone());
            }
        }
        for task_id in restarts {
            state.restart_task(&task_id);
        }
    }
}
