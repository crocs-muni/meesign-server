use crate::communicator::Communicator;
use crate::error::Error;
use crate::group::Group;
use crate::persistence::{Participant, Task as TaskModel};
use crate::proto::TaskType;
use crate::tasks::sign::SignTask;
use crate::tasks::{DecisionUpdate, RestartUpdate, RoundUpdate, Task, TaskResult};
use async_trait::async_trait;
use lazy_static::lazy_static;
use log::{error, info, warn};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use tempfile::NamedTempFile;
use tokio::sync::RwLock;
use uuid::Uuid;

lazy_static! {
    static ref PDF_HELPERS: Mutex<HashMap<Uuid, Child>> = Mutex::new(HashMap::new());
}

pub struct SignPDFTask {
    sign_task: SignTask,
    result: Option<Result<Vec<u8>, String>>,
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
        })
    }

    pub fn from_model(
        model: TaskModel,
        communicator: Arc<RwLock<Communicator>>,
        group: Group,
    ) -> Result<Self, Error> {
        let result = model
            .result
            .clone()
            .map(|res| res.try_into_result())
            .transpose()?;
        let sign_task = SignTask::from_model(model, communicator, group)?;
        Ok(Self { sign_task, result })
    }

    async fn start_task(&mut self) -> Result<RoundUpdate, Error> {
        let file = NamedTempFile::new();
        if file.is_err() {
            error!("Could not create temporary file");
            let reason = "Task failed (server error)".to_string();
            self.set_result(Err(reason.clone()));
            return Ok(RoundUpdate::Failed(reason));
        }
        let mut file = file.unwrap();
        if file.write_all(&self.sign_task.data).is_err() {
            error!("Could not write in temporary file");
            let reason = "Task failed (server error)".to_string();
            self.set_result(Err(reason.clone()));
            return Ok(RoundUpdate::Failed(reason));
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
            self.set_result(Err(reason.clone()));
            return Ok(RoundUpdate::Failed(reason));
        }
        let mut pdfhelper = pdfhelper.unwrap();

        let hash = request_hash(
            &mut pdfhelper,
            self.sign_task.get_group().certificate().unwrap(),
        );
        if hash.is_empty() {
            let reason = "Task failed (invalid PDF)".to_string();
            self.set_result(Err(reason.clone()));
            return Ok(RoundUpdate::Failed(reason));
        }
        PDF_HELPERS
            .lock()
            .unwrap()
            .insert(self.get_id().clone(), pdfhelper);
        self.sign_task.set_preprocessed(hash);
        self.sign_task.start_task().await
    }

    async fn advance_task(&mut self) -> Result<RoundUpdate, Error> {
        self.sign_task.advance_task().await
    }

    async fn finalize_task(&mut self) -> Result<RoundUpdate, Error> {
        let round_update = match self.sign_task.finalize_task().await? {
            RoundUpdate::Finished(round, TaskResult::Signed(signature)) => {
                let mut pdfhelper = PDF_HELPERS.lock().unwrap().remove(self.get_id()).unwrap();
                let signed = include_signature(&mut pdfhelper, &signature);

                info!(
                    "PDF signed by group_id={}",
                    hex::encode(self.sign_task.get_group().identifier())
                );

                self.set_result(Ok(signed.clone()));
                RoundUpdate::Finished(round, TaskResult::SignedPdf(signed))
            }
            other => other,
        };
        Ok(round_update)
    }

    async fn next_round(&mut self) -> Result<RoundUpdate, Error> {
        if self.sign_task.protocol.round() == 0 {
            self.start_task().await
        } else if self.sign_task.protocol.round() < self.sign_task.protocol.last_round() {
            self.advance_task().await
        } else {
            self.finalize_task().await
        }
    }

    fn set_result(&mut self, result: Result<Vec<u8>, String>) {
        self.result = Some(result);
    }
}

#[async_trait]
impl Task for SignPDFTask {
    fn get_type(&self) -> TaskType {
        TaskType::SignPdf
    }

    async fn get_work(&self, device_id: &[u8]) -> Vec<Vec<u8>> {
        self.sign_task.get_work(device_id).await
    }

    async fn get_decisions(&self) -> (u32, u32) {
        self.sign_task.get_decisions().await
    }

    async fn update(
        &mut self,
        device_id: &[u8],
        data: &Vec<Vec<u8>>,
    ) -> Result<RoundUpdate, Error> {
        let round_update = if self.sign_task.update_internal(device_id, data).await? {
            self.next_round().await?
        } else {
            RoundUpdate::Listen
        };
        Ok(round_update)
    }

    async fn restart(&mut self) -> Result<RestartUpdate, Error> {
        if self.result.is_some() {
            return Ok(RestartUpdate::AlreadyFinished);
        }

        if self.is_approved().await {
            if let Some(mut pdfhelper) = PDF_HELPERS.lock().unwrap().remove(self.get_id()) {
                pdfhelper.kill().unwrap();
            }
            self.sign_task.increment_attempt_count();
            let round_update = self.start_task().await?;
            Ok(RestartUpdate::Started(round_update))
        } else {
            Ok(RestartUpdate::Voting)
        }
    }

    async fn is_approved(&self) -> bool {
        self.sign_task.is_approved().await
    }

    fn get_participants(&self) -> &Vec<Participant> {
        self.sign_task.get_participants()
    }

    async fn waiting_for(&self, device: &[u8]) -> bool {
        self.sign_task.waiting_for(device).await
    }

    async fn decide(&mut self, device_id: &[u8], decision: bool) -> Result<DecisionUpdate, Error> {
        let result = self.sign_task.decide_internal(device_id, decision).await;
        let decision_update = match result {
            Some(true) => {
                let round_update = self.next_round().await?;
                DecisionUpdate::Accepted(round_update)
            }
            Some(false) => {
                self.set_result(Err("Task declined".into()));
                DecisionUpdate::Declined
            }
            None => DecisionUpdate::Undecided,
        };
        Ok(decision_update)
    }

    async fn acknowledge(&mut self, device_id: &[u8]) {
        self.sign_task.acknowledge(device_id).await;
    }

    async fn device_acknowledged(&self, device_id: &[u8]) -> bool {
        self.sign_task.device_acknowledged(device_id).await
    }

    fn get_request(&self) -> &[u8] {
        self.sign_task.get_request()
    }

    fn get_attempts(&self) -> u32 {
        self.sign_task.get_attempts()
    }

    fn get_id(&self) -> &Uuid {
        self.sign_task.get_id()
    }

    fn get_communicator(&self) -> Arc<RwLock<Communicator>> {
        self.sign_task.get_communicator()
    }

    fn get_threshold(&self) -> u32 {
        self.sign_task.get_threshold()
    }
    fn get_data(&self) -> Option<&[u8]> {
        self.sign_task.get_data()
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
