use crate::{Task, TaskStatus, TaskType};
use crate::proto::*;
use prost::Message;
use prost::DecodeError;
use crate::group::Group;
use crate::protocols::ProtocolType;
use crate::task::TaskResult;

const LAST_ROUND_KEYGEN: u16 = 6;
const LAST_ROUND_SIGN: u16 = 10;

pub struct GG18Group {
    name: String,
    threshold: u32,
    ids: Vec<Vec<u8>>,
    messages_in: Vec<Vec<Option<Vec<u8>>>>,
    messages_out: Vec<Vec<u8>>,
    result: Option<Group>,
    round: u16,
}

impl GG18Group {
    pub fn new(name: &str, ids: &[Vec<u8>], threshold: u32) -> Self {
        assert!(threshold <= ids.len() as u32);

        let mut ids = ids.clone().to_vec();
        ids.sort();

        let mut messages_in : Vec<Vec<Option<Vec<u8>>>> = Vec::new();
        let mut messages_out : Vec<Vec<u8>> = Vec::new();

        let m = GroupRequest {
            device_ids: ids.iter().map(|x| x.to_vec()).collect(),
            name: String::from(name),
            threshold: Some(threshold),
            protocol: Some(0)
        };

        for _ in 0..ids.len() {
            messages_in.push(vec![None; ids.len()]);
            messages_out.push(m.encode_to_vec());
        }

        GG18Group {
            name: name.into(),
            threshold,
            ids,
            messages_in,
            messages_out,
            result: None,
            round: 0,
        }
    }

    fn id_to_index(&self, device_id: &[u8]) -> Option<usize> {
        self.ids.iter().position(|x| x == device_id)
    }

    fn clear_receive(&mut self) {
        self.messages_in.clear();
        for _ in 0..self.ids.len() {
            self.messages_in.push(vec![None; self.ids.len()]);
        }
    }

    fn broadcast(&mut self, message: Vec<u8>) {
        self.messages_out.clear();

        for _ in 0..self.ids.len() {
            self.messages_out.push(message.clone());
        }
    }

    fn relay(&mut self) {
        self.messages_out.clear();

        for i in 0..self.ids.len() {
            let mut message_out: Vec<Vec<u8>> = Vec::new();
            for j in 0..self.ids.len() {
                if i != j {
                    message_out.push(self.messages_in[j][i].clone().unwrap());
                }
            }
            let m = Gg18Message { message: message_out };
            self.messages_out.push(m.encode_to_vec());
        }
    }

    fn send_all<F>(&mut self, f: F) where F: Fn(usize) -> Vec<u8> {
        self.messages_out.clear();

        for i in 0..self.ids.len() {
            self.messages_out.push(f(i));
        }
    }

    fn start_task(&mut self) {
        assert_eq!(self.round, 0);

        let parties = self.ids.len() as u32;
        let threshold = self.threshold;
        self.send_all(|idx| (Gg18KeyGenInit { index: idx as u32, parties, threshold }).encode_to_vec());
        self.clear_receive();
    }

    fn advance_task(&mut self) {
        assert!((0..LAST_ROUND_KEYGEN).contains(&self.round));

        self.relay();
        self.clear_receive();
    }

    fn finalize_task(&mut self) {
        assert!(self.round >= LAST_ROUND_KEYGEN);

        if self.round == LAST_ROUND_KEYGEN {
            self.result = Some(Group::new(
                self.messages_in[0][1].clone().unwrap(),
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

        self.broadcast(message.encode_to_vec());
        self.clear_receive();
    }

    fn next_round(&mut self) {
        match self.round {
            0 => self.start_task(), // Created -> Running
            1..LAST_ROUND_KEYGEN => self.advance_task(), // Running -> Running
            LAST_ROUND_KEYGEN..=u16::MAX => self.finalize_task() // Running -> Finished
        }
        self.round += 1;
    }

    fn round_received(&self) -> bool {
        (&self.ids)
            .into_iter()
            .zip((&self.messages_in).into_iter())
            .filter(|(_a, b)| b.iter().all(|x| x.is_none()))
            .count() > 0
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

        self.id_to_index(device_id.unwrap()).map(|idx| self.messages_out[idx].clone())
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
        self.messages_in[idx] = data;

        if self.round_received() {
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
        (&self.ids)
            .into_iter()
            .zip((&self.messages_in).into_iter())
            .filter(|(_a, b)| b.iter().all(|x| x.is_none()))
            .map(|(a, _b)| a.as_slice())
            .any(|x| x == device)
    }
}

pub struct GG18Sign {
    sorted_ids: Vec<Vec<u8>>,
    messages_in: Vec<Vec<Option<Vec<u8>>>>,
    messages_out: Vec<Vec<u8>>,
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

        let mut messages_in : Vec<Vec<Option<Vec<u8>>>> = Vec::new();

        for _ in 0..signing_ids.len() {
            messages_in.push(vec![None; signing_ids.len()]);
        }

        let mut indices : Vec<u32> = Vec::new();
        for i in 0..signing_ids.len() {
            indices.push(all_ids.iter().position(|x| x == &signing_ids[i]).unwrap() as u32);
        }

        let mut messages_out : Vec<Vec<u8>> = Vec::new();
        let m = SignRequest {
            group_id: group.identifier().to_vec(),
            data: data.clone(),
        };

        for _ in 0..signing_ids.len() {
            messages_out.push(m.encode_to_vec())
        }

        GG18Sign {
            sorted_ids: signing_ids,
            messages_in,
            messages_out,
            data,
            result: None,
            indices,
            round: 0,
        }
    }

    fn id_to_index(&self, device_id: &[u8]) -> Option<usize> {
        self.sorted_ids.iter().position(|x| x == device_id)
    }

    fn next_round(&mut self) {
        match self.round {
            0 => {
                self.messages_in.clear();
                self.messages_out.clear();
                for i in 0..self.sorted_ids.len() {
                    self.messages_in.push(vec![None; self.sorted_ids.len()]);
                    let m = Gg18SignInit { indices: self.indices.clone(), index: i as u32, hash: self.data.clone() };
                    self.messages_out.push(m.encode_to_vec());
                }
                self.round += 1;
            },
            1..LAST_ROUND_SIGN => {
                let mut messages_out: Vec<Vec<u8>> = Vec::new();

                for i in 0..self.sorted_ids.len() {
                    let mut message_out: Vec<Vec<u8>> = Vec::new();
                    for j in 0..self.sorted_ids.len() {
                        if i != j {
                            message_out.push(self.messages_in[j][i].clone().unwrap());
                        }
                    }
                    let m = Gg18Message { message: message_out };
                    messages_out.push(m.encode_to_vec());
                }

                let mut messages_in = Vec::new();
                for _ in 0..self.sorted_ids.len() {
                    messages_in.push(vec![None; self.sorted_ids.len()]);
                }

                self.messages_in = messages_in;
                self.messages_out = messages_out;
                self.round += 1;
            },
            LAST_ROUND_SIGN..=u16::MAX => {
                if self.round == LAST_ROUND_SIGN {
                    let signature = self.messages_in[0][1].as_ref().unwrap().clone();
                    self.result = Some(signature.clone());
                }

                self.messages_in.clear();
                self.messages_out.clear();

                for _ in 0..self.sorted_ids.len() {
                    self.messages_in.push(vec![None; self.sorted_ids.len()]);
                    self.messages_out.push(self.result.as_ref().unwrap().clone());
                }
            }
        }
    }

    fn round_received(&self) -> bool {
        (&self.sorted_ids)
            .into_iter()
            .zip((&self.messages_in).into_iter())
            .filter(|(_a, b)| b.iter().all(|x| x.is_none()))
            .count() > 0
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

        self.id_to_index(device_id.unwrap()).map(|idx| self.messages_out[idx].clone())
    }

    fn get_result(&self) -> Option<TaskResult> {
        self.result.as_ref().map(|x| TaskResult::Signed(x.clone()))
    }

    fn update(&mut self, device_id: &[u8], data: &[u8]) -> Result<TaskStatus, String> {
        if !self.waiting_for(device_id) {
            return Err("Wasn't waiting for a message from this ID.".to_string())
        }

        let m: Result<Gg18Message, DecodeError> = Message::decode(data);

        if m.is_err() {
            return Err("Failed to parse data.".to_string())
        }

        let mut data : Vec<Option<Vec<u8>>> = m.unwrap().message.into_iter()
            .map(|x| Some(x.to_vec()))
            .collect();

        match self.id_to_index(device_id) {
            Some(i) => {
                data.insert(i, None);
                self.messages_in[i] = data
            },
            None => return Err("Device ID not found.".to_string())
        }

        if self.round_received() {
            self.next_round();
            if self.round > LAST_ROUND_KEYGEN {
                return Ok(TaskStatus::Finished);
            }
        }
        Ok(TaskStatus::Running(self.round))
    }

    fn has_device(&self, device_id: &[u8]) -> bool {
        return self.sorted_ids.contains(&device_id.to_vec())
    }

    fn waiting_for(&self, device: &[u8]) -> bool {
        (&self.sorted_ids)
            .into_iter()
            .zip((&self.messages_in).into_iter())
            .filter(|(_a, b)| b.iter().all(|x| x.is_none()))
            .map(|(a, _b)| a.as_slice())
            .any(|x| x == device)
    }
}
