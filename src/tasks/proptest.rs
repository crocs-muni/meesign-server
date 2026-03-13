#![cfg(test)]

use proptest::prelude::*;
use proptest::range_subset;
use uuid::Uuid;

use std::collections::HashMap;

use super::{
    DeclinedTask, FailedTask, FinishedTask, RunningTaskContext, Task, TaskInfo, TaskResult,
    VotingTask,
};
use crate::persistence::{Device, DeviceKind, Group, KeyType, Participant, ProtocolType, TaskType};

prop_compose! {
    /// Proptest strategy to generate a valid device id
    pub fn valid_device_id()(bytes in any::<[u8; 32]>()) -> Vec<u8> {
        Vec::from(bytes)
    }
}

prop_compose! {
    /// Proptest strategy to generate a valid set of participants
    pub fn valid_participants(device_limit: usize, shares_limit: u32)(n_devices in 2..device_limit)(
        device_ids in prop::collection::hash_set(valid_device_id(), n_devices),
        device_shares in prop::collection::vec(1..shares_limit, n_devices),
    ) -> Vec<Participant> {
        device_ids
            .into_iter()
            .zip(device_shares.into_iter())
            .map(|(id, shares)| {
                Participant {
                    device: Device {
                        id,
                        name: "".to_string(),    // TODO: Randomize device name
                        kind: DeviceKind::User,  // TODO: Randomize device kind
                        certificate: Vec::new(), // TODO: Randomize certificate
                    },
                    shares,
                }
            })
            .collect()
    }
}

/// Proptest strategy to generate a valid "task triple":
/// a supported combination of task type, protocol type and key type
pub fn valid_task_triple() -> impl Strategy<Value = (TaskType, ProtocolType, KeyType)> {
    prop_oneof![
        Just((TaskType::Group, ProtocolType::Gg18, KeyType::SignPdf)),
        Just((TaskType::SignPdf, ProtocolType::Gg18, KeyType::SignPdf)),
        Just((TaskType::Group, ProtocolType::Gg18, KeyType::SignChallenge)),
        Just((
            TaskType::SignChallenge,
            ProtocolType::Gg18,
            KeyType::SignChallenge
        )),
        Just((TaskType::Group, ProtocolType::Frost, KeyType::SignChallenge)),
        Just((
            TaskType::SignChallenge,
            ProtocolType::Frost,
            KeyType::SignChallenge
        )),
        Just((
            TaskType::Group,
            ProtocolType::Musig2,
            KeyType::SignChallenge
        )),
        Just((
            TaskType::SignChallenge,
            ProtocolType::Musig2,
            KeyType::SignChallenge
        )),
        Just((TaskType::Group, ProtocolType::ElGamal, KeyType::Decrypt)),
        Just((TaskType::Decrypt, ProtocolType::ElGamal, KeyType::Decrypt)),
    ]
}

prop_compose! {
    /// Proptest strategy to generate a valid task info
    pub fn valid_task_info(device_limit: usize, shares_limit: u32)(
        name in ".*",
        (task_type, protocol_type, key_type) in valid_task_triple(),
        participants in valid_participants(device_limit, shares_limit),
        attempts in 0..5u32, // TODO: Review arbitrary bound on attempts
        // request in valid_task_request(),  // TODO: Generate valid requests
    ) -> TaskInfo {
        TaskInfo {
            id: Uuid::new_v4(),  // TODO: Should we randomize UUIDs?
            name,
            task_type,
            protocol_type,
            key_type,
            participants,
            attempts,
            request: Vec::new(),
        }
    }
}

fn valid_group(task_info: &TaskInfo, threshold: u32) -> impl Strategy<Value = Group> {
    let protocol_type = task_info.protocol_type;
    let key_type = task_info.key_type;
    let participant_ids_shares: Vec<(Vec<u8>, u32)> = task_info
        .participants
        .iter()
        .map(|participant| (participant.device.id.clone(), participant.shares))
        .collect();
    (".*", prop::option::of(".*")).prop_map(move |(name, note)| {
        Group {
            id: Vec::new(), // TODO: Generate valid group ID (public keys)
            name,
            threshold: threshold as i32,
            protocol: protocol_type,
            key_type,
            certificate: None, // TODO: Generate valid certificate
            note,
            participant_ids_shares: participant_ids_shares.clone(),
        }
    })
}

fn valid_running_task_context(
    task_info: &TaskInfo,
    accept_threshold: u32,
) -> BoxedStrategy<RunningTaskContext> {
    match task_info.task_type {
        TaskType::Group => prop::option::of(".*")
            .prop_map(move |note| RunningTaskContext::Group {
                threshold: accept_threshold,
                note,
            })
            .boxed(),
        TaskType::SignPdf => valid_group(task_info, accept_threshold)
            .prop_map(|group| {
                RunningTaskContext::SignPdf {
                    group,
                    data: Vec::new(), // TODO: Generate valid task data
                }
            })
            .boxed(),
        TaskType::SignChallenge => valid_group(task_info, accept_threshold)
            .prop_map(|group| {
                RunningTaskContext::SignChallenge {
                    group,
                    data: Vec::new(), // TODO: Generate valid task data
                }
            })
            .boxed(),
        TaskType::Decrypt => valid_group(task_info, accept_threshold)
            .prop_map(|group| {
                RunningTaskContext::Decrypt {
                    group,
                    data: Vec::new(), // TODO: Generate valid task data
                }
            })
            .boxed(),
    }
}

fn voting_task_decisions(
    task_info: &TaskInfo,
    accept_threshold: u32,
) -> impl Strategy<Value = HashMap<Vec<u8>, i8>> {
    // We shuffle the participants once to avoid choosing random subsets later
    let shuffled_participants = Just(task_info.participants.clone()).prop_shuffle();
    (shuffled_participants).prop_flat_map(move |shuffled_participants| {
        // The longest prefix of voters who can accept without change the task phase
        let potentially_accepting_voters: Vec<Participant> = shuffled_participants
            .iter()
            .cloned()
            .scan(0, |acc_shares, participant| {
                *acc_shares += participant.shares;
                if *acc_shares < accept_threshold {
                    Some(participant)
                } else {
                    None
                }
            })
            .collect();
        // We pick some of them to actually accept
        (0..=potentially_accepting_voters.len()).prop_flat_map(move |n_accepting_voters| {
            let mut shuffled_participants = shuffled_participants.clone();
            // These voters definitely accept
            let accepting_voters = &shuffled_participants[0..n_accepting_voters];
            let total_accepting_shares: u32 = accepting_voters
                .iter()
                .map(|participant| participant.shares)
                .sum();
            let accepting_decisions: HashMap<Vec<u8>, i8> = accepting_voters
                .iter()
                .map(|participant| (participant.device.id.clone(), participant.shares as i8))
                .collect();

            // The rest cannot accept
            let mut non_accepting_voters = shuffled_participants.split_off(n_accepting_voters);

            // These voters must stay undecided to ensure that the task can still be accepted,
            // i.e. it is not declined
            let definitely_undecided_voters: Vec<Participant> = non_accepting_voters
                .iter()
                .cloned()
                .scan(0, |acc_shares, participant| {
                    let participant_shares = participant.shares;
                    let res = if *acc_shares + total_accepting_shares < accept_threshold {
                        Some(participant)
                    } else {
                        None
                    };
                    *acc_shares += participant_shares;
                    res
                })
                .collect();

            // The rest may decide to either hold their vote or reject
            let non_accepting_voters =
                non_accepting_voters.split_off(definitely_undecided_voters.len());

            // We pick some of them to reject
            (0..=non_accepting_voters.len()).prop_map({
                let non_accepting_voters = non_accepting_voters;
                let accepting_decisions = accepting_decisions;
                move |n_rejecting_voters| {
                    let mut decisions = accepting_decisions.clone();
                    let rejecting_voters = &non_accepting_voters[0..n_rejecting_voters];
                    let rejecting_decisions = rejecting_voters.into_iter().map(|participant| {
                        (participant.device.id.clone(), -(participant.shares as i8))
                    });
                    decisions.extend(rejecting_decisions);
                    decisions
                }
            })
        })
    })
}

/// Proptest strategy to generate a valid voting task
pub fn valid_voting_task(
    device_limit: usize,
    shares_limit: u32,
) -> impl Strategy<Value = VotingTask> {
    valid_task_info(device_limit, shares_limit).prop_flat_map(move |task_info| {
        let total_shares = task_info.total_shares();
        let min_accept_threshold = match task_info.task_type {
            TaskType::Group => total_shares,
            _ => 1,
        };
        // Just like when creating a task in the client, we first specify
        // the accept threshold, which is implicitly `total_shares` for group tasks
        (min_accept_threshold..=total_shares).prop_flat_map({
            let task_info = task_info.clone();
            move |accept_threshold| {
                (
                    voting_task_decisions(&task_info, accept_threshold),
                    valid_running_task_context(&task_info, accept_threshold),
                )
                    .prop_map({
                        let task_info = task_info.clone();
                        move |(decisions, running_task_context)| VotingTask {
                            task_info: task_info.clone(),
                            decisions,
                            accept_threshold,
                            running_task_context,
                        }
                    })
            }
        })
    })
}

fn declined_task_accepts_rejects(
    task_info: &TaskInfo,
    accept_threshold: u32,
) -> impl Strategy<Value = (u32, u32)> {
    let shuffled_participants = Just(task_info.participants.clone()).prop_shuffle();
    (shuffled_participants).prop_flat_map(move |shuffled_participants| {
        // The longest prefix with total shares fewer than the accept threshold
        let potentially_non_rejecting_voters: Vec<Participant> = shuffled_participants
            .iter()
            .cloned()
            .scan(0, |acc_shares, participant| {
                *acc_shares += participant.shares;
                if *acc_shares < accept_threshold {
                    Some(participant)
                } else {
                    None
                }
            })
            .collect();

        // Pick some of them to actually not reject
        (0..=potentially_non_rejecting_voters.len()).prop_flat_map(move |n_non_rejecting_voters| {
            let mut shuffled_participants = shuffled_participants.clone();

            // The participants which need to reject for the task to be declined
            let rejecting_voters = &shuffled_participants[n_non_rejecting_voters..];
            let rejects = rejecting_voters
                .iter()
                .map(|participant| participant.shares)
                .sum();

            shuffled_participants.truncate(n_non_rejecting_voters);
            let non_rejecting_voters = shuffled_participants;

            // NOTE: This must hold because the total shares of non-rejecting voters are strictly
            //       fewer than the accept threshold
            assert!(rejects > 0);

            // Pick some of the non-rejecting voters to actually accept
            (0..=n_non_rejecting_voters).prop_map(move |n_accepting_voters| {
                let non_rejecting_voters = non_rejecting_voters.clone();
                let accepting_voters = &non_rejecting_voters[0..n_accepting_voters];
                let accepts = accepting_voters
                    .iter()
                    .map(|participant| participant.shares)
                    .sum();

                (accepts, rejects)
            })
        })
    })
}

/// Proptest strategy to generate a valid declined task
pub fn valid_declined_task(
    device_limit: usize,
    shares_limit: u32,
) -> impl Strategy<Value = DeclinedTask> {
    valid_task_info(device_limit, shares_limit).prop_flat_map(|task_info| {
        let total_shares = task_info.total_shares();
        (1..=total_shares).prop_flat_map(move |accept_threshold| {
            declined_task_accepts_rejects(&task_info, accept_threshold).prop_map({
                let task_info = task_info.clone();
                move |(accepts, rejects)| DeclinedTask {
                    task_info: task_info.clone(),
                    accepts,
                    rejects,
                }
            })
        })
    })
}

prop_compose! {
    /// Proptest strategy to generate a valid failed task
    pub fn valid_failed_task(device_limit: usize, shares_limit: u32)(
        task_info in valid_task_info(device_limit, shares_limit),
        // TODO: Generate random failure reason
    ) -> FailedTask {
        FailedTask { task_info, reason: "".to_string() }
    }
}

/// Proptest strategy to generate a valid finished task
pub fn valid_finished_task(
    device_limit: usize,
    shares_limit: u32,
) -> impl Strategy<Value = FinishedTask> {
    valid_task_info(device_limit, shares_limit).prop_flat_map(|task_info| {
        let n_participants = task_info.participants.len();
        let acknowledging_indices = range_subset::range_subset(
            0..n_participants,  // NOTE: From indices 0..n_participants
            0..=n_participants, //       we select 0 up to n_participants
        );
        let result = match task_info.task_type {
            TaskType::Group => (1..=task_info.total_shares())
                .prop_flat_map({
                    let task_info = task_info.clone();
                    move |threshold| {
                        valid_group(&task_info, threshold).prop_map(TaskResult::GroupEstablished)
                    }
                })
                .boxed(),
            TaskType::SignChallenge => {
                Just(TaskResult::Signed(Vec::new())).boxed() // TODO: Generate payload
            }
            TaskType::SignPdf => {
                Just(TaskResult::SignedPdf(Vec::new())).boxed() // TODO: Generate payload
            }
            TaskType::Decrypt => {
                Just(TaskResult::Decrypted(Vec::new())).boxed() // TODO: Generate payload
            }
        };
        (acknowledging_indices, result).prop_map({
            let task_info = task_info.clone();
            move |(acknowledging_indices, result)| {
                let acknowledgements = acknowledging_indices
                    .into_iter()
                    .map(|idx| task_info.participants[idx].device.id.clone())
                    .collect();
                FinishedTask {
                    task_info: task_info.clone(),
                    result,
                    acknowledgements,
                }
            }
        })
    })
}

/// Proptest strategy to generate a valid non-voting task
pub fn valid_nonvoting_task(device_limit: usize, shares_limit: u32) -> impl Strategy<Value = Task> {
    prop_oneof![
        valid_declined_task(device_limit, shares_limit).prop_map(Task::Declined),
        valid_failed_task(device_limit, shares_limit).prop_map(Task::Failed),
        valid_finished_task(device_limit, shares_limit).prop_map(Task::Finished),
        // TODO: Add running task
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    proptest! {
        #[test]
        fn voting_task_is_valid(task in valid_voting_task(5, 3)) {
            let (accept_shares, reject_shares) = VotingTask::accepts_rejects(&task.decisions);
            let undecided_shares: u32 = task
                .task_info
                .participants
                .iter()
                .filter(|participant| !task.decisions.contains_key(&participant.device.id))
                .map(|participant| participant.shares)
                .sum();

            assert!(accept_shares < task.accept_threshold);
            assert!(accept_shares + undecided_shares >= task.accept_threshold);
            assert!(accept_shares + reject_shares + undecided_shares == task.task_info.total_shares());

            let participant_id_set: HashSet<_> = task
                .task_info
                .participants
                .into_iter()
                .map(|participant| participant.device.id)
                .collect();

            for (participant_id, _) in &task.decisions {
                assert!(participant_id_set.contains(participant_id));
            }
        }
    }

    proptest! {
        #[test]
        fn declined_task_is_valid(task in valid_declined_task(5, 3)) {
            assert!(task.rejects >= 1);
            assert!(task.accepts + task.rejects <= task.task_info.total_shares());

            // TODO: Check that accepts and rejects are sums of shares of disjoint subsets?
        }
    }
}
