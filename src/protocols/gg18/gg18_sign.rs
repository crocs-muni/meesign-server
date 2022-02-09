use crate::{Task, TaskStatus, TaskType};


const LAST_ROUND: u16 = 10;
pub struct GG18Sign {
    sorted_ids: Vec<Vec<u8>>,
    messages_in: Vec<Vec<Option<Vec<u8>>>>,
    messages_out: Vec<Vec<u8>>,
    signature: Option<Vec<u8>>,
    round: u16,
}

impl GG18Sign {
    pub fn new(mut all_ids: Vec<Vec<u8>>, mut signing_ids: Vec<Vec<u8>>, message_hash: Vec<u8>) -> Self {
        all_ids.sort();
        signing_ids.sort();
        let mut messages_in : Vec<Vec<Option<Vec<u8>>>> = Vec::new();

        for _ in 0..signing_ids.len() {
            messages_in.push(vec![None; signing_ids.len()]);
        }

        let mut indices : Vec<u16> = Vec::new();
        for i in 0..signing_ids.len() {
            indices.push(all_ids.iter().position(|x| x == &signing_ids[i]).unwrap() as u16);
        }

        let mut messages_out : Vec<Vec<u8>> = Vec::new();
        for i in 0..signing_ids.len() {
            messages_out.push(serde_json::to_vec(&(indices.clone(), i as u16, message_hash.clone())).unwrap());
        }

        GG18Sign {
            sorted_ids: signing_ids,
            messages_in,
            messages_out,
            signature: None,
            round: 1,
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
            messages_out[i] = serde_json::to_vec(&message_out).unwrap();
        }

        let mut messages_in : Vec<Vec<Option<Vec<u8>>>> = Vec::new();
        for _ in 0..self.sorted_ids.len() {
            messages_in.push(vec![None; self.sorted_ids.len()]);
        }

        self.messages_in = messages_in;
        self.messages_out = messages_out;
    }
}

impl Task for GG18Sign {
    fn get_status(&self) -> (TaskType, TaskStatus) {
        if self.signature.is_none() {
            return (TaskType::Sign, TaskStatus::Waiting(self.waiting_for()))
        }

        match self.signature.clone() {
            Some(pk) => (TaskType::Sign, TaskStatus::KeysGenerated(pk)),
            None => (TaskType::Sign,
                     TaskStatus::Failed("Served did not recieve a signature.".as_bytes().to_vec()))
        }
    }

    fn update(&mut self, device_id: &[u8], data: &[u8]) -> Result<TaskStatus, String> {
        let waiting_for = self.waiting_for();
        if !waiting_for.contains(&device_id.to_vec()) {
            return Err("Wasn't waiting for a message from this ID.".to_string())
        }
        let data  = serde_json::from_slice::<Vec<Vec<u8>>>(data);
        if data.is_err() {
            return Err("Failed to parse data.".to_string())
        }
        let mut data: Vec<Option<Vec<u8>>> = data.unwrap().into_iter().map(|x| Some(x)).collect();

        match self.id_to_index(device_id) {
            Some(i) => {data.insert(i, None);
                        self.messages_in[i] = data},
            None => return Err("Device ID not found.".to_string())
        }

        if self.waiting_for().is_empty() {
            if self.round == LAST_ROUND {
                if self.messages_in[0][1].is_none() {
                    return Err("Failed to recieve a signature.".to_string())
                }
                return Ok(TaskStatus::Signed(self.messages_in[0][1].clone().unwrap()))
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
}
