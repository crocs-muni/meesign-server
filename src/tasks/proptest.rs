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
    // If the accumulated shares of accepting participants exceed the accept threshold,
    // the task is accepted and thus not voting. We need to limit the accepting participants.
    // To avoid `prop_filter`, we randomly shuffle the participants, and keep only
    // the longest prefix whose total shares cannot exceed the threshold,
    // even if all of them accept.
    let shuffled_participants = Just(task_info.participants.clone()).prop_shuffle();
    (shuffled_participants).prop_flat_map(move |shuffled_participants| {
        // A set of participants with fewer total shares than the accept threshold
        let potential_voters: Vec<Participant> = shuffled_participants
            .into_iter()
            .scan(0, |acc_shares, participant| {
                *acc_shares += participant.shares;
                if *acc_shares < accept_threshold {
                    Some(participant)
                } else {
                    None
                }
            })
            .collect();

        // Generate a subset of the potential voters
        let voter_indices = range_subset::range_subset(
            0..potential_voters.len(),  // NOTE: From indices 0..n_potential_voters
            0..=potential_voters.len(), //       we select 0 up to n_potential_voters
        );
        // Generate arbitrary decisions for the voters
        let votes =
            prop::collection::vec(prop_oneof![Just(true), Just(false)], potential_voters.len());
        (voter_indices, votes).prop_map(move |(voter_indices, votes)| {
            voter_indices
                .into_iter()
                .map(|voter_idx| {
                    let participant = &potential_voters[voter_idx];
                    let accepted = votes[voter_idx];
                    let vote = if accepted {
                        participant.shares as i8
                    } else {
                        -(participant.shares as i8)
                    };
                    (participant.device.id.clone(), vote)
                })
                .collect()
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

fn declined_task_decisions(
    task_info: &TaskInfo,
    accept_threshold: u32,
) -> impl Strategy<Value = HashMap<Vec<u8>, i8>> {
    // The task is declined if it becomes impossible to accept the task even if all undecided
    // participants accepted. We calculate the minimum amount of rejects and let the others
    // decide randomly.
    // To avoid `prop_filter`, we randomly shuffle the participants, and find the longest
    // prefix whose total shares cannot exceed the threshold, even if all of them accept.
    // The rest must reject, but this prefix can decide arbitrarily.
    let shuffled_participants = Just(task_info.participants.clone()).prop_shuffle();
    (shuffled_participants).prop_flat_map(move |shuffled_participants| {
        // A set of participants with fewer total shares than the accept threshold
        let arbitrary_voters: Vec<Participant> = shuffled_participants
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

        // The participants which need to reject for the task to be declined
        let rejecting_voters = &shuffled_participants[arbitrary_voters.len()..];

        let rejecting_decisions: HashMap<_, _> = rejecting_voters
            .into_iter()
            .map(|participant| (participant.device.id.clone(), -(participant.shares as i8)))
            .collect();

        // Generate a subset of the arbitrary voters, representing those who actually vote
        let voter_indices = range_subset::range_subset(
            0..arbitrary_voters.len(),  // NOTE: From indices 0..n_arbitrary_voters
            0..=arbitrary_voters.len(), //       we select 0 up to n_arbitrary_voters
        );
        // Generate arbitrary decisions for the voters
        let votes =
            prop::collection::vec(prop_oneof![Just(true), Just(false)], arbitrary_voters.len());
        (voter_indices, votes).prop_map({
            let rejecting_decisions = rejecting_decisions.clone();
            move |(voter_indices, votes)| {
                let mut rejecting_decisions = rejecting_decisions.clone();
                let arbitrary_votes = voter_indices.into_iter().map(|voter_idx| {
                    let participant = &arbitrary_voters[voter_idx];
                    let accepted = votes[voter_idx];
                    let vote = if accepted {
                        participant.shares as i8
                    } else {
                        -(participant.shares as i8)
                    };
                    (participant.device.id.clone(), vote)
                });
                rejecting_decisions.extend(arbitrary_votes);
                rejecting_decisions
            }
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
            declined_task_decisions(&task_info, accept_threshold).prop_map({
                let task_info = task_info.clone();
                move |decisions| {
                    let accepts = decisions.values().filter(|&vote| *vote > 0).count() as u32;
                    let rejects = decisions.values().filter(|&vote| *vote < 0).count() as u32;
                    DeclinedTask {
                        task_info: task_info.clone(),
                        accepts,
                        rejects,
                    }
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
