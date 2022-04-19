use crate::{Task, TaskStatus, TaskType};
use crate::proto::*;
use prost::Message;
use crate::group::Group;
use crate::protocols::ProtocolType;
use crate::task::TaskResult;

const LAST_ROUND_KEYGEN: u16 = 6;
const LAST_ROUND_SIGN: u16 = 10;

struct Communicator {
    parties: usize,
    pub input: Vec<Vec<Option<Vec<u8>>>>, // TODO remove pub
    pub output: Vec<Vec<u8>>, // TODO remove pub
}

impl Communicator {
    pub fn new(parties: usize) -> Self {
        Communicator {
            parties,
            input: Vec::new(),
            output: Vec::new(),
        }
    }

    pub fn clear_input(&mut self) {
        self.input.clear();
        for _ in 0..self.parties {
            self.input.push(vec![None; self.parties]);
        }
    }

    pub fn broadcast(&mut self, message: Vec<u8>) {
        self.output.clear();

        for _ in 0..self.parties {
            self.output.push(message.clone());
        }
    }

    pub fn relay(&mut self) {
        self.output.clear();

        for i in 0..self.parties {
            let mut out: Vec<Vec<u8>> = Vec::new();
            for j in 0..self.parties {
                if i != j {
                    out.push(self.input[j][i].clone().unwrap());
                }
            }
            let message = Gg18Message { message: out };
            self.output.push(message.encode_to_vec());
        }
    }

    pub fn send_all<F>(&mut self, f: F) where F: Fn(usize) -> Vec<u8> {
        self.output.clear();

        for i in 0..self.parties {
            self.output.push(f(i));
        }
    }

    pub fn round_received(&self, ids: &[Vec<u8>]) -> bool {
        ids.into_iter()
            .zip((&self.input).into_iter())
            .filter(|(_a, b)| b.iter().all(|x| x.is_none()))
            .count() > 0
    }

    pub fn get_message(&self, idx: usize) -> Option<&Vec<u8>> {
        self.output.get(idx)
    }
}

pub struct GG18Group {
    name: String,
    threshold: u32,
    ids: Vec<Vec<u8>>,
    communicator: Communicator,
    result: Option<Group>,
    round: u16,
}

impl GG18Group {
    pub fn new(name: &str, ids: &[Vec<u8>], threshold: u32) -> Self {
        assert!(threshold <= ids.len() as u32);

        let mut ids = ids.clone().to_vec();
        ids.sort();

        let mut communicator = Communicator::new(ids.len());
        communicator.clear_input();
        let message = GroupRequest {
            device_ids: ids.iter().map(|x| x.to_vec()).collect(),
            name: String::from(name),
            threshold: Some(threshold),
            protocol: Some(0)
        };
        communicator.broadcast(message.encode_to_vec());

        GG18Group {
            name: name.into(),
            threshold,
            ids,
            communicator,
            result: None,
            round: 0,
        }
    }

    fn id_to_index(&self, device_id: &[u8]) -> Option<usize> {
        self.ids.iter().position(|x| x == device_id)
    }

    fn start_task(&mut self) {
        assert_eq!(self.round, 0);

        let parties = self.ids.len() as u32;
        let threshold = self.threshold;
        self.communicator.send_all(|idx| (Gg18KeyGenInit { index: idx as u32, parties, threshold }).encode_to_vec());
        self.communicator.clear_input();
    }

    fn advance_task(&mut self) {
        assert!((0..LAST_ROUND_KEYGEN).contains(&self.round));

        self.communicator.relay();
        self.communicator.clear_input();
    }

    fn finalize_task(&mut self) {
        assert!(self.round >= LAST_ROUND_KEYGEN);

        if self.round == LAST_ROUND_KEYGEN {
            self.result = Some(Group::new(
                self.communicator.input[0][1].clone().unwrap(),
                self.name.clone(),
                self.ids.clone(),
                self.threshold,
                ProtocolType::GG18
            ));
        }

        let group = self.result.as_ref().unwrap();

        let message = crate::proto::Group {
            id: group.identifier().to_vec(),
            name: group.name().to_string(),
            threshold: group.threshold(),
            device_ids: group.devices().iter().map(Vec::clone).collect()
        };

        self.communicator.broadcast(message.encode_to_vec());
        self.communicator.clear_input();
    }

    fn next_round(&mut self) {
        match self.round {
            0 => self.start_task(), // Created -> Running
            1..LAST_ROUND_KEYGEN => self.advance_task(), // Running -> Running
            LAST_ROUND_KEYGEN..=u16::MAX => self.finalize_task() // Running -> Finished
        }
        self.round += 1;
    }
}

impl Task for GG18Group {
    fn get_status(&self) -> TaskStatus {
        match self.round {
            0 => TaskStatus::Created,
            1..=LAST_ROUND_KEYGEN => TaskStatus::Running(self.round),
            _ => self.result.as_ref().map(|_| TaskStatus::Finished)
                .unwrap_or(TaskStatus::Failed(String::from("The group was not established."))),
        }
    }

    fn get_type(&self) -> TaskType {
        TaskType::Group
    }

    fn get_work(&self, device_id: Option<&[u8]>) -> Option<Vec<u8>> {
        if device_id.is_none() || !self.waiting_for(device_id.unwrap()) {
            return None
        }

        self.id_to_index(device_id.unwrap()).map(|idx| self.communicator.get_message(idx).unwrap().clone())
    }

    fn get_result(&self) -> Option<TaskResult> {
        self.result.as_ref().map(|x| TaskResult::GroupEstablished(x.clone()))
    }

    fn update(&mut self, device_id: &[u8], data: &[u8]) -> Result<TaskStatus, String> {
        if !self.waiting_for(device_id) {
            return Err("Wasn't waiting for a message from this ID.".to_string())
        }

        let data: Gg18Message = Message::decode(data).map_err(|_| String::from("Failed to parse data."))?;
        let mut data : Vec<Option<Vec<u8>>> = data.message.into_iter().map(Some).collect();

        let idx = self.id_to_index(device_id).ok_or("Device ID not found.".to_string())?;
        data.insert(idx, None);
        self.communicator.input[idx] = data;

        if self.communicator.round_received(&self.ids) {
            self.next_round();
            if self.round > LAST_ROUND_KEYGEN {
                return Ok(TaskStatus::Finished);
            }
        }
        Ok(TaskStatus::Running(self.round))
    }

    fn has_device(&self, device_id: &[u8]) -> bool {
        return self.ids.contains(&device_id.to_vec())
    }

    fn waiting_for(&self, device: &[u8]) -> bool {
        self.ids.iter()
            .zip((&self.communicator.input).into_iter())
            .filter(|(_a, b)| b.iter().all(|x| x.is_none()))
            .map(|(a, _b)| a.as_slice())
            .any(|x| x == device)
    }
}

pub struct GG18Sign {
    ids: Vec<Vec<u8>>,
    communicator: Communicator,
    data: Vec<u8>,
    result: Option<Vec<u8>>,
    indices: Vec<u32>,
    round: u16,
}

impl GG18Sign {
    pub fn new(group: Group, data: Vec<u8>) -> Self {
        // TODO add communication round in which signing ids are identified; currently assumes t=n
        let mut all_ids: Vec<Vec<u8>> = group.devices().iter().map(Vec::clone).collect();
        all_ids.sort();
        let signing_ids = all_ids.clone();

        let mut indices : Vec<u32> = Vec::new();
        for i in 0..signing_ids.len() {
            indices.push(all_ids.iter().position(|x| x == &signing_ids[i]).unwrap() as u32);
        }

        let mut communicator = Communicator::new(signing_ids.len());
        communicator.clear_input();
        let message = SignRequest {
            group_id: group.identifier().to_vec(),
            data: data.clone(),
        };
        communicator.broadcast(message.encode_to_vec());

        GG18Sign {
            ids: signing_ids,
            communicator,
            data,
            result: None,
            indices,
            round: 0,
        }
    }

    fn id_to_index(&self, device_id: &[u8]) -> Option<usize> {
        self.ids.iter().position(|x| x == device_id)
    }

    fn start_task(&mut self) {
        assert_eq!(self.round, 0);

        let indices = self.indices.clone();
        let hash = self.data.clone();
        self.communicator.send_all(|idx| (Gg18SignInit { indices: indices.clone(), index: idx as u32, hash: hash.clone() }).encode_to_vec());
        self.communicator.clear_input();
    }

    fn advance_task(&mut self) {
        assert!((0..LAST_ROUND_SIGN).contains(&self.round));

        self.communicator.relay();
        self.communicator.clear_input();
    }

    fn finalize_task(&mut self) {
        assert!(self.round >= LAST_ROUND_SIGN);

        if self.round == LAST_ROUND_SIGN {
            self.result = Some(self.communicator.input[0][1].as_ref().unwrap().clone());
        }

        self.communicator.broadcast(self.result.as_ref().unwrap().clone());
        self.communicator.clear_input();
    }

    fn next_round(&mut self) {
        match self.round {
            0 => self.start_task(), // Created -> Running
            1..LAST_ROUND_SIGN => self.advance_task(), // Running -> Running
            LAST_ROUND_SIGN..=u16::MAX => self.finalize_task() // Running -> Finished
        }
        self.round += 1;
    }
}

impl Task for GG18Sign {
    fn get_status(&self) -> TaskStatus {
        match self.round {
            0 => TaskStatus::Created,
            1..=LAST_ROUND_SIGN => TaskStatus::Running(self.round),
            u16::MAX => self.result.as_ref().map(|_| TaskStatus::Finished)
                .unwrap_or(TaskStatus::Failed(String::from("Server did not receive a signature."))),
            _ => unreachable!()
        }
    }

    fn get_type(&self) -> TaskType {
        TaskType::Sign
    }

    fn get_work(&self, device_id: Option<&[u8]>) -> Option<Vec<u8>> {
        if device_id.is_none() || !self.waiting_for(device_id.unwrap()) {
            return None
        }

        self.id_to_index(device_id.unwrap()).map(|idx| self.communicator.get_message(idx).unwrap().clone())
    }

    fn get_result(&self) -> Option<TaskResult> {
        self.result.as_ref().map(|x| TaskResult::Signed(x.clone()))
    }

    fn update(&mut self, device_id: &[u8], data: &[u8]) -> Result<TaskStatus, String> {
        if !self.waiting_for(device_id) {
            return Err("Wasn't waiting for a message from this ID.".to_string())
        }

        let data: Gg18Message = Message::decode(data).map_err(|_| "Failed to parse data.".to_string())?;
        let mut data : Vec<Option<Vec<u8>>> = data.message.into_iter().map(Some).collect();

        let idx = self.id_to_index(device_id).ok_or("Device ID not found.".to_string())?;
        data.insert(idx, None);
        self.communicator.input[idx] = data;

        if self.communicator.round_received(&self.ids) {
            self.next_round();
            if self.round > LAST_ROUND_SIGN {
                return Ok(TaskStatus::Finished);
            }
        }
        Ok(TaskStatus::Running(self.round))
    }

    fn has_device(&self, device_id: &[u8]) -> bool {
        return self.ids.contains(&device_id.to_vec())
    }

    fn waiting_for(&self, device: &[u8]) -> bool {
        self.ids.iter()
            .zip((&self.communicator.input).into_iter())
            .filter(|(_a, b)| b.iter().all(|x| x.is_none()))
            .map(|(a, _b)| a.as_slice())
            .any(|x| x == device)
    }
}
