use crate::communicator::Communicator;
use crate::error::Error;
use crate::get_timestamp;
use crate::group::Group;
use crate::persistence::{Device, Repository};
use crate::proto::TaskType;
use crate::tasks::sign::SignTask;
use crate::tasks::{Task, TaskResult, TaskStatus};
use async_trait::async_trait;
use log::{error, info, warn};
use lazy_static::lazy_static;
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
    pub fn try_new(
        group: Group,
        name: String,
        data: Vec<u8>,
        repository: Arc<Repository>,
    ) -> Result<Self, String> {
        if data.len() > 8 * 1024 * 1024 || name.len() > 256 || name.chars().any(|x| x.is_control())
        {
            warn!("Invalid input name={} len={}", name, data.len());
            return Err("Invalid input".to_string());
        }

        let sign_task = SignTask::try_new(group, name, data, repository)?;

        Ok(SignPDFTask {
            sign_task,
            result: None,
        })
    }

    async fn start_task(&mut self, repository: Arc<Repository>) -> Result<(), Error> {
        let file = NamedTempFile::new();
        if file.is_err() {
            error!("Could not create temporary file");
            self.set_result(
                Err("Task failed (server error)".to_string()),
                repository,
            ).await?;
            return Ok(()); // TODO: double check that behavior won't change if we return as err
        }
        let mut file = file.unwrap();
        if file.write_all(&self.sign_task.data).is_err() {
            error!("Could not write in temporary file");
            self.set_result(
                Err("Task failed (server error)".to_string()),
                repository,
            ).await?;
            return Ok(()); // TODO: double check that behavior won't change if we return as err
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
            self.set_result(
                Err("Task failed (server error)".to_string()),
                repository,
            ).await?;
            return Ok(()); // TODO: double check that behavior won't change if we return as err
        }
        let mut pdfhelper = pdfhelper.unwrap();

        let hash = request_hash(
            &mut pdfhelper,
            self.sign_task.get_group().certificate().unwrap(),
        );
        if hash.is_empty() {
            self.set_result(
                Err("Task failed (invalid PDF)".to_string()),
                repository,
            ).await?;
            return Ok(()); // TODO: double check that behavior won't change if we return as err
        }
        PDF_HELPERS.lock().unwrap().insert(self.get_id().clone(), pdfhelper);
        self.sign_task.set_preprocessed(hash);
        self.sign_task.start_task(repository).await
    }

    async fn advance_task(&mut self) -> Result<(), Error> {
        self.sign_task.advance_task().await
    }

    async fn finalize_task(&mut self, repository: Arc<Repository>) -> Result<(), Error> {
        self.sign_task.finalize_task(repository.clone()).await?;
        if let Some(TaskResult::Signed(signature)) = self.sign_task.get_result() {
            let mut pdfhelper = PDF_HELPERS
                .lock()
                .unwrap()
                .remove(self.get_id())
                .unwrap();
            let signed = include_signature(
                &mut pdfhelper,
                &signature,
            );

            info!(
                "PDF signed by group_id={}",
                hex::encode(self.sign_task.get_group().identifier())
            );

            self.set_result(Ok(signed), repository).await?;
        } else {
            self.set_result(
                Err("Task failed (signature not output)".to_string()),
                repository,
            ).await?;
        }
        Ok(())
    }

    async fn next_round(&mut self, repository: Arc<Repository>) -> Result<(), Error> {
        if self.sign_task.protocol.round() == 0 {
            self.start_task(repository).await
        } else if self.sign_task.protocol.round() < self.sign_task.protocol.last_round() {
            self.advance_task().await
        } else {
            self.finalize_task(repository).await
        }
    }

    async fn set_result(&mut self, result: Result<Vec<u8>, String>, repository: Arc<Repository>) -> Result<(), Error> {
        repository
            .set_task_result(self.get_id(), &result)
            .await?;
        self.result = Some(result);
        Ok(())
    }
}

#[async_trait]
impl Task for SignPDFTask {
    fn get_status(&self) -> TaskStatus {
        self.sign_task.get_status()
    }

    fn get_type(&self) -> TaskType {
        TaskType::SignPdf
    }

    async fn get_work(&self, device_id: Option<&[u8]>) -> Vec<Vec<u8>> {
        self.sign_task.get_work(device_id).await
    }

    fn get_result(&self) -> Option<TaskResult> {
        if let Some(Ok(signature)) = &self.result {
            Some(TaskResult::SignedPdf(signature.clone()))
        } else {
            None
        }
    }

    async fn get_decisions(&self) -> (u32, u32) {
        self.sign_task.get_decisions().await
    }

    async fn update(
        &mut self,
        device_id: &[u8],
        data: &Vec<Vec<u8>>,
        repository: Arc<Repository>,
    ) -> Result<bool, Error> {
        let result = self
            .sign_task
            .update_internal(device_id, data, repository.clone())
            .await;
        if let Ok(true) = result {
            self.next_round(repository).await?;
        };
        result
    }

    async fn restart(&mut self, repository: Arc<Repository>) -> Result<bool, Error> {
        self.sign_task.last_update = get_timestamp();
        if self.result.is_some() {
            return Ok(false);
        }

        if self.is_approved().await {
            if let Some(mut pdfhelper) = PDF_HELPERS
                .lock()
                .unwrap()
                .remove(self.get_id())
            {
                pdfhelper.kill().unwrap();
            }
            self.sign_task.attempts += 1;
            self.start_task(repository).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn last_update(&self) -> u64 {
        self.sign_task.last_update()
    }

    async fn is_approved(&self) -> bool {
        self.sign_task.is_approved().await
    }

    fn get_devices(&self) -> &Vec<Device> {
        self.sign_task.get_devices()
    }

    async fn waiting_for(&self, device: &[u8]) -> bool {
        self.sign_task.waiting_for(device).await
    }

    async fn decide(
        &mut self,
        device_id: &[u8],
        decision: bool,
        repository: Arc<Repository>,
    ) -> Option<bool> {
        let result = self
            .sign_task
            .decide_internal(device_id, decision, repository.clone())
            .await;
        if let Some(true) = result {
            self.next_round(repository).await.unwrap();
        };
        result
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

    async fn from_model(
        model: crate::persistence::Task,
        devices: Vec<Device>,
        communicator: Arc<RwLock<Communicator>>,
        repository: Arc<Repository>,
    ) -> Result<Self, crate::error::Error>
    where
        Self: Sized,
    {
        let result = match model.result.clone() {
            Some(val) => val.try_into_option()?,
            None => None,
        };
        let sign_task = SignTask::from_model(model, devices, communicator, repository).await?;
        Ok(Self {
            sign_task,
            result,
        })
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
