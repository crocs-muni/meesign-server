use crate::error::Error;
use crate::state::State;
use crate::utils;

use log::debug;
use tokio::time;
use tonic::codegen::Arc;

pub async fn run_timer(state: Arc<State>) -> Result<(), String> {
    let mut interval = time::interval(time::Duration::from_secs(1));
    loop {
        interval.tick().await;
        check_tasks(&state).await.unwrap();
        check_subscribers(&state).await;
    }
}

async fn check_tasks(state: &State) -> Result<(), Error> {
    state.restart_stale_tasks().await
}

async fn check_subscribers(state: &State) {
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
            state.activate_device(device_id);
        }
    }
    for device_id in remove {
        state.remove_subscriber(&device_id);
    }
}
