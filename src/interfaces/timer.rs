use crate::error::Error;
use crate::state::State;
use crate::utils;

use log::debug;
use tokio::sync::MutexGuard;
use tokio::{sync::Mutex, time};
use tonic::codegen::Arc;

pub async fn run_timer(state: Arc<Mutex<State>>) -> Result<(), String> {
    let mut interval = time::interval(time::Duration::from_secs(1));
    loop {
        interval.tick().await;
        let mut state = state.lock().await;
        check_tasks(&mut state).await.unwrap();
        check_subscribers(&mut state).await;
    }
}

async fn check_tasks(state: &mut MutexGuard<'_, State>) -> Result<(), Error> {
    for task_id in state.get_tasks_for_restart().await? {
        state.restart_task(&task_id).await?;
    }
    Ok(())
}

async fn check_subscribers(state: &mut MutexGuard<'_, State>) {
    let mut remove = Vec::new();
    for subscriber in state.get_subscribers().iter() {
        let (device_id, tx) = subscriber.pair();
        if tx.is_closed() {
            debug!(
                "Closed channel detected device_id={:?}",
                utils::hextrunc(device_id)
            );
            remove.push(device_id.clone());
        } else {
            state.activate_device(&device_id);
        }
    }
    for device_id in remove {
        state.remove_subscriber(&device_id);
    }
}
