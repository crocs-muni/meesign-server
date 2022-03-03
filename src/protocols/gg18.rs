use crate::{Task, TaskStatus, TaskType};
use crate::proto::*;
use prost::Message;
use prost::DecodeError;

const LAST_ROUND_KEYGEN: u16 = 6;
const LAST_ROUND_SIGN: u16 = 10;

pub struct GG18 {
    name: Option<String>,
    sorted_ids: Vec<Vec<u8>>,
    messages_in: Vec<Vec<Option<Vec<u8>>>>,
    messages_out: Vec<Vec<u8>>,
    result: Option<Vec<u8>>,
    round: u16,
    task_type: TaskType,
    last_round: u16,
}

impl GG18 {
    pub fn new_group(name: &str, mut ids: &[Vec<u8>], threshold: u32) -> Self {
        assert!(threshold <= ids.len() as u32);

        let mut ids = ids.clone().to_vec();
        ids.sort();
        let mut messages_in : Vec<Vec<Option<Vec<u8>>>> = Vec::new();
        let mut messages_out : Vec<Vec<u8>> = Vec::new();

        for i in 0..ids.len() {
            messages_in.push(vec![None; ids.len()]);

            let m = Gg18KeyGenInit {index: i as u32, parties: ids.len() as u32, threshold};
            messages_out.push(m.encode_to_vec());
        }

        GG18 {
            name: Some(name.into()),
            sorted_ids: ids,
            messages_in,
            messages_out,
            result: None,
            round: 1,
            task_type: TaskType::GG18Group,
            last_round: LAST_ROUND_KEYGEN,
        }
    }

    pub fn new_sign(mut all_ids: Vec<Vec<u8>>, mut signing_ids: Vec<Vec<u8>>, message_hash: Vec<u8>) -> Self {
        all_ids.sort();
        signing_ids.sort();
        let mut messages_in : Vec<Vec<Option<Vec<u8>>>> = Vec::new();

        for _ in 0..signing_ids.len() {
            messages_in.push(vec![None; signing_ids.len()]);
        }

        let mut indices : Vec<u32> = Vec::new();
        for i in 0..signing_ids.len() {
            indices.push(all_ids.iter().position(|x| x == &signing_ids[i]).unwrap() as u32);
        }

        let mut messages_out : Vec<Vec<u8>> = Vec::new();
        for i in 0..signing_ids.len() {
            let m = Gg18SignInit{indices:indices.clone(), index: i as u32, hash: message_hash.clone()};
            messages_out.push(m.encode_to_vec())
        }

        GG18 {
            name: None,
            sorted_ids: signing_ids,
            messages_in,
            messages_out,
            result: None,
            round: 1,
            task_type: TaskType::Sign,
            last_round: LAST_ROUND_SIGN,
        }
    }

    fn waiting_for(&self) -> Vec<Vec<u8>> {
        (&self.sorted_ids)
            .into_iter()
            .zip((&self.messages_in).into_iter())
            .filter(|(_a, b)| b.iter().all(|x| x.is_none()))
            .map(|(a, _b)| a.clone())
            .collect()
    }

    fn id_to_index(&self, device_id: &[u8]) -> Option<usize> {
        self.sorted_ids.iter().position(|x| x == device_id)
    }

    fn new_round(&mut self) {
        let mut messages_out : Vec<Vec<u8>> = Vec::new();
        
        for i in 0..self.sorted_ids.len() {
            let mut message_out : Vec<Vec<u8>> = Vec::new();
            for j in 0..self.sorted_ids.len() {
                if i != j {
                    message_out.push(self.messages_in[j][i].clone().unwrap());
                }
            }
            let m = Gg18Message {message: message_out};
            messages_out[i] = m.encode_to_vec();
        }

        let mut messages_in : Vec<Vec<Option<Vec<u8>>>> = Vec::new();
        for _ in 0..self.sorted_ids.len() {
            messages_in.push(vec![None; self.sorted_ids.len()]);
        }

        self.messages_in = messages_in;
        self.messages_out = messages_out;
    }
}

impl Task for GG18 {

    fn get_status(&self) -> (TaskType, TaskStatus) {
        if self.result.is_none() {
            return (self.task_type.clone(), TaskStatus::Waiting(self.waiting_for()))
        }

        if self.task_type == TaskType::GG18Group {
            match self.result.clone() {
                Some(pk) => (self.task_type.clone(), TaskStatus::KeysGenerated(pk)),
                None => (self.task_type.clone(),
                         TaskStatus::Failed("Served did not receive a public key.".as_bytes().to_vec()))
            }
        } else {
            match self.result.clone() {
                Some(pk) => (TaskType::Sign, TaskStatus::KeysGenerated(pk)),
                None => (TaskType::Sign,
                         TaskStatus::Failed("Served did not receive a signature.".as_bytes().to_vec()))
            }

        }
    }

    fn update(&mut self, device_id: &[u8], data: &[u8]) -> Result<TaskStatus, String> {
        let waiting_for = self.waiting_for();
        if !waiting_for.contains(&device_id.to_vec()) {
            return Err("Wasn't waiting for a message from this ID.".to_string())
        }

        let m : Result<Gg18Message, DecodeError> = Message::decode(data);

        if m.is_err() {
            return Err("Failed to parse data.".to_string())
        }

        let mut data : Vec<Option<Vec<u8>>> = m.unwrap().message.into_iter()
            .map(|x| Some(x.as_slice().to_vec()))
            .collect();

        match self.id_to_index(device_id) {
            Some(i) => {
                data.insert(i, None);
                self.messages_in[i] = data
            },
            None => return Err("Device ID not found.".to_string())
        }

        if self.waiting_for().is_empty() {
            if self.round == self.last_round {
                if self.messages_in[0][1].is_none() {
                    return Err("Failed to receive a last message.".to_string())
                }

                return Ok(if self.task_type == TaskType::GG18Group {
                    TaskStatus::KeysGenerated(self.messages_in[0][1].clone().unwrap())
                } else {
                    TaskStatus::Signed(self.messages_in[0][1].clone().unwrap())
                })
            }
            self.new_round();
            self.round += 1;
        }
        Ok(TaskStatus::Waiting(self.waiting_for()))
    }

    fn get_work(&self, device_id: &[u8]) -> Option<Vec<u8>> {
        let waiting_for = self.waiting_for();
        if !waiting_for.contains(&device_id.to_vec()) {
            return None
        }

        match self.id_to_index(device_id) {
            Some(x) => Some(self.messages_out[x].clone()),
            None => None
        }
    }

    fn has_device(&self, device_id: &[u8]) -> bool {
        return self.sorted_ids.contains(&device_id.to_vec())
    }
}
