use crate::device::Device;
use crate::get_timestamp;
use crate::group::Group;
use crate::proto::TaskType;
use crate::tasks::sign::SignTask;
use crate::tasks::{Task, TaskResult, TaskStatus};
use log::{error, info, warn};
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use tempfile::NamedTempFile;
use tonic::codegen::Arc;

pub struct SignPDFTask {
    sign_task: SignTask,
    result: Option<Result<Vec<u8>, String>>,
    pdfhelper: Option<Child>,
}

impl SignPDFTask {
    pub fn try_new(group: Group, name: String, data: Vec<u8>) -> Result<Self, String> {
        if data.len() > 8 * 1024 * 1024 || name.len() > 256 || name.chars().any(|x| x.is_control())
        {
            warn!("Invalid input name={} len={}", name, data.len());
            return Err("Invalid input".to_string());
        }

        let sign_task = SignTask::try_new(group, name, data)?;

        Ok(SignPDFTask {
            sign_task,
            result: None,
            pdfhelper: None,
        })
    }

    fn start_task(&mut self) {
        let file = NamedTempFile::new();
        if file.is_err() {
            error!("Could not create temporary file");
            self.result = Some(Err("Task failed (server error)".to_string()));
            return;
        }
        let mut file = file.unwrap();
        if file.write_all(&self.sign_task.data).is_err() {
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

        let hash = request_hash(
            &mut pdfhelper,
            self.sign_task.get_group().certificate().unwrap(),
        );
        if hash.is_empty() {
            self.result = Some(Err("Task failed (invalid PDF)".to_string()));
            return;
        }
        self.pdfhelper = Some(pdfhelper);
        self.sign_task.set_preprocessed(hash);
        self.sign_task.start_task();
    }

    fn advance_task(&mut self) {
        self.sign_task.advance_task();
    }

    fn finalize_task(&mut self) {
        self.sign_task.finalize_task();
        if let Some(TaskResult::Signed(signature)) = self.sign_task.get_result() {
            let signed = include_signature(self.pdfhelper.as_mut().unwrap(), &signature);
            self.pdfhelper = None;

            info!(
                "PDF signed by group_id={}",
                hex::encode(self.sign_task.get_group().identifier())
            );
            self.result = Some(Ok(signed));
        } else {
            self.result = Some(Err("Task failed (signature not output)".to_string()));
        }
    }

    fn next_round(&mut self) {
        if self.sign_task.protocol.round() == 0 {
            self.start_task();
        } else if self.sign_task.protocol.round() < self.sign_task.protocol.last_round() {
            self.advance_task()
        } else {
            self.finalize_task()
        }
    }
}

impl Task for SignPDFTask {
    fn get_status(&self) -> TaskStatus {
        self.sign_task.get_status()
    }

    fn get_type(&self) -> TaskType {
        TaskType::SignPdf
    }

    fn get_work(&self, device_id: Option<&[u8]>) -> Vec<Vec<u8>> {
        self.sign_task.get_work(device_id)
    }

    fn get_result(&self) -> Option<TaskResult> {
        if let Some(Ok(signature)) = &self.result {
            Some(TaskResult::SignedPdf(signature.clone()))
        } else {
            None
        }
    }

    fn get_decisions(&self) -> (u32, u32) {
        self.sign_task.get_decisions()
    }

    fn update(&mut self, device_id: &[u8], data: &Vec<Vec<u8>>) -> Result<bool, String> {
        let result = self.sign_task.update_internal(device_id, data);
        if let Ok(true) = result {
            self.next_round();
        };
        result
    }

    fn restart(&mut self) -> Result<bool, String> {
        self.sign_task.last_update = get_timestamp();
        if self.result.is_some() {
            return Ok(false);
        }

        if self.is_approved() {
            if let Some(pdfhelper) = self.pdfhelper.as_mut() {
                pdfhelper.kill().unwrap();
                self.pdfhelper = None;
            }
            self.sign_task.attempts += 1;
            self.start_task();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn last_update(&self) -> u64 {
        self.sign_task.last_update()
    }

    fn is_approved(&self) -> bool {
        self.sign_task.is_approved()
    }

    fn has_device(&self, device_id: &[u8]) -> bool {
        self.sign_task.has_device(device_id)
    }

    fn get_devices(&self) -> Vec<Arc<Device>> {
        self.sign_task.get_devices()
    }

    fn waiting_for(&self, device: &[u8]) -> bool {
        self.sign_task.waiting_for(device)
    }

    fn decide(&mut self, device_id: &[u8], decision: bool) -> Option<bool> {
        let result = self.sign_task.decide_internal(device_id, decision);
        if let Some(true) = result {
            self.next_round();
        };
        result
    }

    fn acknowledge(&mut self, device_id: &[u8]) {
        self.sign_task.acknowledge(device_id);
    }

    fn device_acknowledged(&self, device_id: &[u8]) -> bool {
        self.sign_task.device_acknowledged(device_id)
    }

    fn get_request(&self) -> &[u8] {
        self.sign_task.get_request()
    }

    fn get_attempts(&self) -> u32 {
        self.sign_task.get_attempts()
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
    out_buffer[..128].copy_from_slice(hex::encode(signature).as_bytes());
    out_buffer[128] = b'\n';

    process_stdin.write_all(&out_buffer).unwrap();

    let mut result = Vec::new();
    process_stdout.read_to_end(&mut result).unwrap();
    hex::decode(&result).unwrap()
}
