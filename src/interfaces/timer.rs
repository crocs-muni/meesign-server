use crate::get_timestamp;
use crate::state::State;
use crate::tasks::TaskStatus;

use log::debug;
use tokio::sync::MutexGuard;
use tokio::{sync::Mutex, time};
use tonic::codegen::Arc;

pub async fn run_timer(state: Arc<Mutex<State>>) -> Result<(), String> {
    let mut interval = time::interval(time::Duration::from_secs(1));
    loop {
        interval.tick().await;
        let mut state = state.lock().await;
        check_tasks(&mut state);
        check_subscribers(&mut state);
    }
}

fn check_tasks(state: &mut MutexGuard<State>) {
    let mut restarts = Vec::new();
    let timestamp = get_timestamp();
    for (task_id, task) in state.get_tasks() {
        if task.get_status() != TaskStatus::Finished
            && task.is_approved()
            && timestamp - task.last_update() > 30
        {
            debug!("Stale task detected task_id={:?}", hex::encode(task_id));
            restarts.push(*task_id);
        }
    }
    for task_id in restarts {
        state.restart_task(&task_id);
    }
}

fn check_subscribers(state: &mut MutexGuard<State>) {
    let mut remove = Vec::new();
    for (device_id, tx) in state.get_subscribers() {
        if tx.is_closed() {
            debug!(
                "Closed channel detected device_id={:?}",
                hex::encode(device_id)
            );
            remove.push(device_id.clone());
        } else {
            state.device_activated(device_id);
        }
    }
    for device_id in remove {
        state.remove_subscriber(&device_id);
    }
}
