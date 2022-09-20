use crate::communicator::Communicator;
use crate::device::Device;
use crate::group::Group;
use crate::proto::{Gg18Message, SignRequest, TaskAcknowledgement};
use crate::protocols::gg18::GG18Sign;
use crate::protocols::Protocol;
use crate::tasks::{Task, TaskResult, TaskStatus, TaskType};
use prost::Message;
use std::fs::File;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};

pub struct SignPDFTask {
    group: Group,
    participant_ids: Option<Vec<Vec<u8>>>,
    communicator: Communicator,
    result: Option<Vec<u8>>,
    round: u16,
    document: Vec<u8>,
    pdfhelper: Option<Child>,
    failed: Option<String>,
    protocol: Box<dyn Protocol + Send + Sync>,
    request: Vec<u8>,
}

impl SignPDFTask {
    pub fn new(group: Group, name: String, data: Vec<u8>) -> Self {
        let mut devices: Vec<Device> = group.devices().values().map(Device::clone).collect();
        devices.sort_by_key(|x| x.identifier().to_vec());

        let communicator = Communicator::new(&devices, group.threshold());

        let request = (SignRequest {
            group_id: group.identifier().to_vec(),
            name: name.clone(),
            data: data.clone(),
        })
        .encode_to_vec();

        SignPDFTask {
            group,
            participant_ids: None,
            communicator,
            result: None,
            round: 0,
            document: data.clone(),
            pdfhelper: None,
            failed: None,
            protocol: Box::new(GG18Sign::new()),
            request,
        }
    }

    fn id_to_index(&self, device_id: &[u8]) -> Option<usize> {
        self.participant_ids
            .as_ref()
            .and_then(|x| x.iter().position(|x| x == device_id))
    }

    fn start_task(&mut self) {
        assert_eq!(self.round, 0);
        {
            let mut file = File::create("document.pdf").unwrap();
            file.write_all(&self.document).unwrap();
        }

        let mut pdfhelper = Command::new("java")
            .arg("-jar")
            .arg("MeeSignHelper.jar")
            .arg("sign")
            .arg("document.pdf")
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();

        let hash = request_hash(&mut pdfhelper, self.group.certificate());
        self.pdfhelper = Some(pdfhelper);
        std::fs::remove_file("document.pdf").unwrap();
        self.protocol.initialize(&mut self.communicator, &hash);
    }

    fn advance_task(&mut self) {
        self.protocol.advance(&mut self.communicator)
    }

    fn finalize_task(&mut self) {
        let signature = self.protocol.finalize(&mut self.communicator);
        let signed = include_signature(self.pdfhelper.as_mut().unwrap(), &signature);
        self.pdfhelper = None;
        self.result = Some(signed);

        self.communicator.clear_input();
    }

    fn next_round(&mut self) {
        if self.protocol.round() == 0 {
            self.start_task();
        } else if self.protocol.round() < self.protocol.last_round() {
            self.advance_task()
        } else {
            self.finalize_task()
        }
    }
}

impl Task for SignPDFTask {
    fn get_status(&self) -> TaskStatus {
        if self.failed.is_some() {
            return TaskStatus::Failed(self.failed.clone().unwrap());
        }

        if self.protocol.round() == 0 {
            TaskStatus::Created
        } else if self.protocol.round() <= self.protocol.last_round() {
            TaskStatus::Running(self.protocol.round())
        } else {
            self.result
                .as_ref()
                .map(|_| TaskStatus::Finished)
                .unwrap_or(TaskStatus::Failed(String::from(
                    "Server did not receive a signature",
                )))
        }
    }

    fn get_type(&self) -> TaskType {
        TaskType::Sign
    }

    fn get_work(&self, device_id: Option<&[u8]>) -> Option<Vec<u8>> {
        if device_id.is_none() || !self.waiting_for(device_id.unwrap()) {
            return None;
        }

        self.id_to_index(device_id.unwrap())
            .and_then(|idx| self.communicator.get_message(idx).cloned())
    }

    fn get_result(&self) -> Option<TaskResult> {
        self.result.as_ref().map(|x| TaskResult::Signed(x.clone()))
    }

    fn get_confirmations(&self) -> (u32, u32) {
        (
            self.communicator.accept_count(),
            self.communicator.reject_count(),
        )
    }

    fn update(&mut self, device_id: &[u8], data: &[u8]) -> Result<(), String> {
        if self.communicator.accept_count() != self.communicator.threshold as u32 {
            return Err("Not enough agreements to proceed with the protocol.".to_string());
        }

        if !self.waiting_for(device_id) {
            return Err("Wasn't waiting for a message from this ID.".to_string());
        }

        let idx = self
            .id_to_index(device_id)
            .ok_or("Device ID not found.".to_string())?;

        if 0 < self.protocol.round() && self.protocol.round() <= self.protocol.last_round() {
            let data: Gg18Message =
                Message::decode(data).map_err(|_| String::from("Expected GG18Message."))?;
            self.communicator.input[idx] = data.message.into_iter().map(Some).collect();
            self.communicator.input[idx].insert(idx, None);
        } else {
            let _data: TaskAcknowledgement =
                Message::decode(data).map_err(|_| String::from("Expected TaskAcknowledgement."))?;
            self.communicator.input[idx] =
                vec![Some(vec![]); (self.communicator.threshold - 1) as usize];
            self.communicator.input[idx].insert(idx, None);
        }

        if self.participant_ids.is_some()
            && self
                .communicator
                .round_received(&self.participant_ids.as_ref().unwrap())
            && self.protocol.round() <= self.protocol.last_round()
        {
            self.next_round();
        }
        Ok(())
    }

    fn has_device(&self, device_id: &[u8]) -> bool {
        return self.group.contains(device_id);
    }

    fn waiting_for(&self, device: &[u8]) -> bool {
        if self.round == 0 {
            return !self.communicator.device_confirmed(device);
        }

        self.participant_ids
            .as_ref()
            .unwrap()
            .iter()
            .zip((&self.communicator.input).into_iter())
            .filter(|(_a, b)| b.iter().all(|x| x.is_none()))
            .map(|(a, _b)| a.as_slice())
            .any(|x| x == device)
    }

    fn confirmation(&mut self, device_id: &[u8], accept: bool) {
        self.communicator.confirmation(device_id, accept);
        if self.round == 0 {
            if self.communicator.reject_count() >= self.group.reject_threshold() {
                self.failed = Some("Too many rejections.".to_string());
            } else if self.communicator.accept_count() >= self.group.threshold() {
                self.next_round();
            }
        }
    }

    fn get_request(&self) -> &[u8] {
        &self.request
    }
}

fn request_hash(process: &mut Child, certificate: &[u8]) -> Vec<u8> {
    let process_stdin = process.stdin.as_mut().unwrap();
    let process_stdout = process.stdout.as_mut().unwrap();

    process_stdin.write(certificate).unwrap();
    process_stdin.flush().unwrap();

    let mut in_buffer = [0u8; 65]; // \n
    process_stdout.read_exact(&mut in_buffer).unwrap();

    hex::decode(String::from_utf8(Vec::from(&in_buffer[..64])).unwrap()).unwrap()
}

fn include_signature(process: &mut Child, signature: &[u8]) -> Vec<u8> {
    let process_stdin = process.stdin.as_mut().unwrap();
    let process_stdout = process.stdout.as_mut().unwrap();

    let mut out_buffer = [0u8; 129];
    out_buffer[..128].copy_from_slice(hex::encode(&signature).as_bytes());
    out_buffer[128] = '\n' as u8;

    process_stdin.write(&out_buffer).unwrap();

    let mut result = Vec::new();
    process_stdout.read_to_end(&mut result).unwrap();
    hex::decode(&result).unwrap()
}
