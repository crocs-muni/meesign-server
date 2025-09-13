use crate::communicator::Communicator;
use crate::error::Error;
use crate::group::Group;
use crate::persistence::Task as TaskModel;
use crate::tasks::sign::SignTask;
use crate::tasks::{FailedTask, RoundUpdate, RunningTask, TaskInfo, TaskResult};
use lazy_static::lazy_static;
use log::{error, info, warn};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tempfile::NamedTempFile;
use uuid::Uuid;

lazy_static! {
    static ref PDF_HELPERS: Mutex<HashMap<Uuid, Child>> = Mutex::new(HashMap::new());
}

pub struct SignPDFTask {
    sign_task: SignTask,
}

impl SignPDFTask {
    pub fn try_new(
        task_info: TaskInfo,
        group: Group,
        data: Vec<u8>,
        decisions: HashMap<Vec<u8>, i8>,
    ) -> Result<Self, String> {
        if data.len() > 8 * 1024 * 1024
            || task_info.name.len() > 256
            || task_info.name.chars().any(|x| x.is_control())
        {
            warn!("Invalid input name={} len={}", task_info.name, data.len());
            return Err("Invalid input".to_string());
        }

        let sign_task = SignTask::try_new(task_info, group, data, decisions)?;

        Ok(SignPDFTask { sign_task })
    }

    pub fn from_model(
        task_info: TaskInfo,
        model: TaskModel,
        communicator: Communicator,
        group: Group,
    ) -> Result<Self, Error> {
        let sign_task = SignTask::from_model(task_info, model, communicator, group)?;
        Ok(Self { sign_task })
    }

    fn start_task(&mut self) -> Result<RoundUpdate, Error> {
        let file = NamedTempFile::new();
        if file.is_err() {
            error!("Could not create temporary file");
            let reason = "Task failed (server error)".to_string();
            return Ok(RoundUpdate::Failed(FailedTask {
                task_info: self.task_info().clone(),
                reason,
            }));
        }
        let mut file = file.unwrap();
        if file.write_all(&self.sign_task.data).is_err() {
            error!("Could not write in temporary file");
            let reason = "Task failed (server error)".to_string();
            return Ok(RoundUpdate::Failed(FailedTask {
                task_info: self.task_info().clone(),
                reason,
            }));
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
            let reason = "Task failed (server error)".to_string();
            return Ok(RoundUpdate::Failed(FailedTask {
                task_info: self.task_info().clone(),
                reason,
            }));
        }
        let mut pdfhelper = pdfhelper.unwrap();

        let hash = request_hash(
            &mut pdfhelper,
            self.sign_task.get_group().certificate().unwrap(),
        );
        if hash.is_empty() {
            let reason = "Task failed (invalid PDF)".to_string();
            return Ok(RoundUpdate::Failed(FailedTask {
                task_info: self.task_info().clone(),
                reason,
            }));
        }
        PDF_HELPERS
            .lock()
            .unwrap()
            .insert(self.task_info().id.clone(), pdfhelper);
        self.sign_task.set_preprocessed(hash);
        self.sign_task.start_task()
    }

    fn advance_task(&mut self) -> Result<RoundUpdate, Error> {
        self.sign_task.advance_task()
    }

    fn finalize_task(&mut self) -> Result<RoundUpdate, Error> {
        let round_update = match self.sign_task.finalize_task()? {
            RoundUpdate::Finished(round, mut task) => {
                let mut pdfhelper = PDF_HELPERS
                    .lock()
                    .unwrap()
                    .remove(&self.task_info().id)
                    .unwrap();
                let TaskResult::Signed(signature) = task.result else {
                    unreachable!()
                };
                let signed = include_signature(&mut pdfhelper, &signature);

                info!(
                    "PDF signed by group_id={}",
                    hex::encode(self.sign_task.get_group().identifier())
                );
                task.result = TaskResult::SignedPdf(signed);
                RoundUpdate::Finished(round, task)
            }
            other => other,
        };
        Ok(round_update)
    }

    fn next_round(&mut self) -> Result<RoundUpdate, Error> {
        if self.sign_task.protocol.round() == 0 {
            self.start_task()
        } else if self.sign_task.protocol.round() < self.sign_task.protocol.last_round() {
            self.advance_task()
        } else {
            self.finalize_task()
        }
    }
}

impl RunningTask for SignPDFTask {
    fn task_info(&self) -> &TaskInfo {
        &self.sign_task.task_info
    }

    fn get_work(&self, device_id: &[u8]) -> Vec<Vec<u8>> {
        self.sign_task.get_work(device_id)
    }

    fn get_round(&self) -> u16 {
        self.sign_task.get_round()
    }

    fn initialize(&mut self) -> Result<RoundUpdate, Error> {
        self.start_task()
    }

    fn update(&mut self, device_id: &[u8], data: &Vec<Vec<u8>>) -> Result<RoundUpdate, Error> {
        let round_update = if self.sign_task.update_internal(device_id, data)? {
            self.next_round()?
        } else {
            RoundUpdate::Listen
        };
        Ok(round_update)
    }

    fn restart(&mut self) -> Result<RoundUpdate, Error> {
        if let Some(mut pdfhelper) = PDF_HELPERS.lock().unwrap().remove(&self.task_info().id) {
            pdfhelper.kill().unwrap();
        }
        self.sign_task.increment_attempt_count();
        self.start_task()
    }

    fn waiting_for(&self, device: &[u8]) -> bool {
        self.sign_task.waiting_for(device)
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
