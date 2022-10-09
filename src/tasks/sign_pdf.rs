use crate::communicator::Communicator;
use crate::device::Device;
use crate::get_timestamp;
use crate::group::Group;
use crate::proto::{Gg18Message, SignRequest};
use crate::protocols::gg18::GG18Sign;
use crate::protocols::Protocol;
use crate::tasks::{Task, TaskResult, TaskStatus, TaskType};
use log::{error, info, warn};
use prost::Message;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use tempfile::NamedTempFile;
use tonic::codegen::Arc;

pub struct SignPDFTask {
    group: Group,
    communicator: Communicator,
    result: Option<Result<Vec<u8>, String>>,
    document: Vec<u8>,
    pdfhelper: Option<Child>,
    protocol: Box<dyn Protocol + Send + Sync>,
    request: Vec<u8>,
    last_update: u64,
    attempts: u32,
}

impl SignPDFTask {
    pub fn try_new(group: Group, name: String, data: Vec<u8>) -> Result<Self, String> {
        if data.len() > 8 * 1024 * 1024 || name.len() > 256 || name.chars().any(|x| x.is_control())
        {
            warn!("Invalid input name={} len={}", name, data.len());
            return Err("Invalid input".to_string());
        }

        let mut devices: Vec<Arc<Device>> = group.devices().to_vec();
        devices.sort_by_key(|x| x.identifier().to_vec());

        let communicator = Communicator::new(&devices, group.threshold());

        let request = (SignRequest {
            group_id: group.identifier().to_vec(),
            name,
            data: data.clone(),
        })
        .encode_to_vec();

        Ok(SignPDFTask {
            group,
            communicator,
            result: None,
            document: data,
            pdfhelper: None,
            protocol: Box::new(GG18Sign::new()),
            request,
            last_update: get_timestamp(),
            attempts: 0,
        })
    }

    fn start_task(&mut self) {
        assert!(self.communicator.accept_count() >= self.group.threshold());
        let file = NamedTempFile::new();
        if file.is_err() {
            error!("Could not create temporary file");
            self.result = Some(Err("Task failed (server error)".to_string()));
            return;
        }
        let mut file = file.unwrap();
        if file.write_all(&self.document).is_err() {
            error!("Could not write in temporary file");
            self.result = Some(Err("Task failed (server error)".to_string()));
            return;
        }

        let pdfhelper = Command::new("java")
            .arg("-jar")
            .arg("MeeSignHelper.jar")
            .arg("sign")
            .arg(file.path().to_str().unwrap())
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn();

        if pdfhelper.is_err() {
            error!("Could not start PDFHelper");
            self.result = Some(Err("Task failed (server error)".to_string()));
            return;
        }
        let mut pdfhelper = pdfhelper.unwrap();

        let hash = request_hash(&mut pdfhelper, self.group.certificate().unwrap());
        if hash.is_empty() {
            self.result = Some(Err("Task failed (invalid PDF)".to_string()));
            return;
        }
        self.pdfhelper = Some(pdfhelper);
        self.protocol.initialize(&mut self.communicator, &hash);
    }

    fn advance_task(&mut self) {
        self.protocol.advance(&mut self.communicator)
    }

    fn finalize_task(&mut self) {
        let signature = self.protocol.finalize(&mut self.communicator);
        if signature.is_none() {
            self.result = Some(Err("Task failed (signature not output)".to_string()));
            return;
        }
        let signature = signature.unwrap();
        let signed = include_signature(self.pdfhelper.as_mut().unwrap(), &signature);
        self.pdfhelper = None;

        info!(
            "PDF signed by group_id={}",
            hex::encode(self.group.identifier())
        );

        self.result = Some(Ok(signed));

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
        match &self.result {
            Some(Err(e)) => TaskStatus::Failed(e.clone()),
            Some(Ok(_)) => TaskStatus::Finished,
            None => {
                if self.protocol.round() == 0 {
                    TaskStatus::Created
                } else {
                    TaskStatus::Running(self.protocol.round())
                }
            }
        }
    }

    fn get_type(&self) -> TaskType {
        TaskType::Sign
    }

    fn get_work(&self, device_id: Option<&[u8]>) -> Option<Vec<u8>> {
        if device_id.is_none() || !self.waiting_for(device_id.unwrap()) {
            return None;
        }

        self.communicator.get_message(device_id.unwrap())
    }

    fn get_result(&self) -> Option<TaskResult> {
        if let Some(Ok(signature)) = &self.result {
            Some(TaskResult::Signed(signature.clone()))
        } else {
            None
        }
    }

    fn get_decisions(&self) -> (u32, u32) {
        (
            self.communicator.accept_count(),
            self.communicator.reject_count(),
        )
    }

    fn update(&mut self, device_id: &[u8], data: &[u8]) -> Result<bool, String> {
        if self.communicator.accept_count() < self.group.threshold() {
            return Err("Not enough agreements to proceed with the protocol.".to_string());
        }

        if !self.waiting_for(device_id) {
            return Err("Wasn't waiting for a message from this ID.".to_string());
        }

        let data: Gg18Message =
            Message::decode(data).map_err(|_| String::from("Expected GG18Message."))?;
        self.communicator.receive_messages(device_id, data.message);
        self.last_update = get_timestamp();

        if self.communicator.round_received() && self.protocol.round() <= self.protocol.last_round()
        {
            self.next_round();
            return Ok(true);
        }
        Ok(false)
    }

    fn restart(&mut self) -> Result<bool, String> {
        self.last_update = get_timestamp();
        if self.result.is_some() {
            return Ok(false);
        }

        if self.is_approved() {
            if let Some(pdfhelper) = self.pdfhelper.as_mut() {
                pdfhelper.kill().unwrap();
                self.pdfhelper = None;
            }
            self.attempts += 1;
            self.start_task();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn last_update(&self) -> u64 {
        self.last_update
    }

    fn is_approved(&self) -> bool {
        self.communicator.accept_count() >= self.group.threshold()
    }

    fn has_device(&self, device_id: &[u8]) -> bool {
        self.group.contains(device_id)
    }

    fn get_devices(&self) -> Vec<Arc<Device>> {
        self.group.devices().to_vec()
    }

    fn waiting_for(&self, device: &[u8]) -> bool {
        if self.protocol.round() == 0 {
            return !self.communicator.device_decided(device);
        } else if self.protocol.round() >= self.protocol.last_round() {
            return !self.communicator.device_acknowledged(device);
        }

        self.communicator.waiting_for(device)
    }

    fn decide(&mut self, device_id: &[u8], decision: bool) -> bool {
        self.communicator.decide(device_id, decision);
        self.last_update = get_timestamp();
        if self.result.is_none() && self.protocol.round() == 0 {
            if self.communicator.reject_count() >= self.group.reject_threshold() {
                self.result = Some(Err("Task declined".to_string()));
                return true;
            } else if self.communicator.accept_count() >= self.group.threshold() {
                self.next_round();
                return true;
            }
        }
        false
    }

    fn acknowledge(&mut self, device_id: &[u8]) {
        self.communicator.acknowledge(device_id);
    }

    fn device_acknowledged(&self, device_id: &[u8]) -> bool {
        self.communicator.device_acknowledged(device_id)
    }

    fn get_request(&self) -> &[u8] {
        &self.request
    }

    fn get_attempts(&self) -> u32 {
        self.attempts
    }
}

fn request_hash(process: &mut Child, certificate: &[u8]) -> Vec<u8> {
    let process_stdin = process.stdin.as_mut().unwrap();
    let process_stdout = process.stdout.as_mut().unwrap();

    process_stdin.write_all(certificate).unwrap();
    process_stdin.flush().unwrap();

    let mut in_buffer = [0u8; 65]; // \n
    if let Ok(()) = process_stdout.read_exact(&mut in_buffer) {
        hex::decode(String::from_utf8(Vec::from(&in_buffer[..64])).unwrap()).unwrap()
    } else {
        Vec::new()
    }
}

fn include_signature(process: &mut Child, signature: &[u8]) -> Vec<u8> {
    let process_stdin = process.stdin.as_mut().unwrap();
    let process_stdout = process.stdout.as_mut().unwrap();

    let mut out_buffer = [0u8; 129];
    out_buffer[..128].copy_from_slice(hex::encode(&signature).as_bytes());
    out_buffer[128] = b'\n';

    process_stdin.write_all(&out_buffer).unwrap();

    let mut result = Vec::new();
    process_stdout.read_to_end(&mut result).unwrap();
    hex::decode(&result).unwrap()
}
