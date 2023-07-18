use log::{debug, info};
use openssl::asn1::{Asn1Integer, Asn1Time};
use openssl::bn::BigNum;
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::x509::{X509Builder, X509Extension, X509NameBuilder, X509Req};
use rand::Rng;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
use tonic::codegen::Arc;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::proto::mee_sign_server::{MeeSign, MeeSignServer};
use crate::proto::{KeyType, ProtocolType};
use crate::state::State;
use crate::tasks::{Task, TaskStatus};
use crate::{proto as msg, CA_CERT, CA_KEY};

use std::pin::Pin;

pub struct MeeSignService {
    state: Arc<Mutex<State>>,
}

impl MeeSignService {
    pub fn new(state: Arc<Mutex<State>>) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl MeeSign for MeeSignService {
    type SubscribeUpdatesStream =
        Pin<Box<dyn Stream<Item = Result<msg::Task, Status>> + Send + 'static>>;

    async fn get_server_info(
        &self,
        _request: Request<msg::ServerInfoRequest>,
    ) -> Result<Response<msg::ServerInfo>, Status> {
        debug!("ServerInfoRequest");
        Ok(Response::new(msg::ServerInfo {
            version: crate::VERSION.unwrap_or("unknown").to_string(),
        }))
    }

    async fn register(
        &self,
        request: Request<msg::RegistrationRequest>,
    ) -> Result<Response<msg::RegistrationResponse>, Status> {
        let request = request.into_inner();
        let name = request.name;
        let csr = request.csr;
        info!("RegistrationRequest name={:?}", name);

        let mut state = self.state.lock().await;

        if let Ok(certificate) = issue_certificate(&name, &csr) {
            let device_id = cert_to_id(&certificate);
            if state.add_device(&device_id, &name, &certificate) {
                Ok(Response::new(msg::RegistrationResponse {
                    device_id,
                    certificate,
                }))
            } else {
                Err(Status::failed_precondition(
                    "Request failed: device was not added",
                ))
            }
        } else {
            Err(Status::failed_precondition(
                "Request failed: certificate was not created",
            ))
        }
    }

    async fn sign(
        &self,
        request: Request<msg::SignRequest>,
    ) -> Result<Response<msg::Task>, Status> {
        let request = request.into_inner();
        let group_id = request.group_id;
        let name = request.name;
        let data = request.data;
        info!("SignRequest group_id={}", hex::encode(&group_id));

        let mut state = self.state.lock().await;
        if let Some(task_id) = state.add_sign_task(&group_id, &name, &data) {
            let task = state.get_task(&task_id).unwrap();
            Ok(Response::new(format_task(&task_id, task, None, None)))
        } else {
            Err(Status::failed_precondition("Request failed"))
        }
    }

    async fn decrypt(
        &self,
        request: Request<msg::DecryptRequest>,
    ) -> Result<Response<msg::Task>, Status> {
        let request = request.into_inner();
        let group_id = request.group_id;
        let name = request.name;
        let data = request.data;
        info!("DecryptRequest group_id={}", hex::encode(&group_id));

        let mut state = self.state.lock().await;
        if let Some(task_id) = state.add_decrypt_task(&group_id, &name, &data) {
            let task = state.get_task(&task_id).unwrap();
            Ok(Response::new(format_task(&task_id, task, None, None)))
        } else {
            Err(Status::failed_precondition("Request failed"))
        }
    }

    async fn get_task(
        &self,
        request: Request<msg::TaskRequest>,
    ) -> Result<Response<msg::Task>, Status> {
        let request = request.into_inner();
        let task_id = Uuid::from_slice(&request.task_id).unwrap();
        let device_id = request.device_id;
        let device_id = if device_id.is_none() {
            None
        } else {
            Some(device_id.as_ref().unwrap().as_slice())
        };
        debug!(
            "TaskRequest task_id={} device_id={}",
            hex::encode(task_id),
            hex::encode(device_id.unwrap_or(&[]))
        );

        let state = self.state.lock().await;
        if device_id.is_some() {
            state.device_activated(device_id.as_ref().unwrap());
        }
        let task = state.get_task(&task_id).unwrap();
        let request = Some(task.get_request());

        let resp = format_task(&task_id, task, device_id, request);
        Ok(Response::new(resp))
    }

    async fn update_task(
        &self,
        request: Request<msg::TaskUpdate>,
    ) -> Result<Response<msg::Resp>, Status> {
        if request.peer_certs().is_none() {
            return Err(Status::unauthenticated("Authentication required"));
        }
        let device_id = request
            .peer_certs()
            .and_then(|certs| certs.get(0).map(cert_to_id))
            .unwrap();

        let request = request.into_inner();
        let task_id = Uuid::from_slice(&request.task).unwrap();
        let data = request.data;
        let attempt = request.attempt;
        info!(
            "TaskUpdate task_id={} device_id={} attempt={}",
            hex::encode(task_id),
            hex::encode(&device_id),
            attempt
        );

        let mut state = self.state.lock().await;
        state.device_activated(&device_id);
        let result = state.update_task(&task_id, &device_id, &data, attempt);

        match result {
            Ok(_) => Ok(Response::new(msg::Resp {
                message: "OK".into(),
            })),
            Err(e) => Err(Status::failed_precondition(e)),
        }
    }

    async fn get_tasks(
        &self,
        request: Request<msg::TasksRequest>,
    ) -> Result<Response<msg::Tasks>, Status> {
        let request = request.into_inner();
        let device_id = request.device_id;
        let device_str = device_id
            .as_ref()
            .map(hex::encode)
            .unwrap_or_else(|| "unknown".to_string());
        debug!("TasksRequest device_id={}", device_str);

        let state = self.state.lock().await;
        let tasks = if let Some(device_id) = device_id {
            state.device_activated(&device_id);
            state
                .get_device_tasks(&device_id)
                .iter()
                .map(|(task_id, task)| format_task(task_id, *task, Some(&device_id), None))
                .collect()
        } else {
            state
                .get_tasks()
                .iter()
                .map(|(task_id, task)| format_task(task_id, task.as_ref(), None, None))
                .collect()
        };

        Ok(Response::new(msg::Tasks { tasks }))
    }

    async fn get_groups(
        &self,
        request: Request<msg::GroupsRequest>,
    ) -> Result<Response<msg::Groups>, Status> {
        let request = request.into_inner();
        let device_id = request.device_id;
        let device_str = device_id
            .as_ref()
            .map(hex::encode)
            .unwrap_or_else(|| "unknown".to_string());
        debug!("GroupsRequest device_id={}", device_str);

        let state = self.state.lock().await;
        let groups = if let Some(device_id) = device_id {
            state.device_activated(&device_id);
            state
                .get_device_groups(&device_id)
                .iter()
                .map(|group| group.into())
                .collect()
        } else {
            state
                .get_groups()
                .values()
                .map(|group| group.into())
                .collect()
        };

        Ok(Response::new(msg::Groups { groups }))
    }

    async fn group(
        &self,
        request: Request<msg::GroupRequest>,
    ) -> Result<Response<msg::Task>, Status> {
        let request = request.into_inner();
        let name = request.name;
        let device_ids = request.device_ids;
        let threshold = request.threshold;
        let protocol = ProtocolType::from_i32(request.protocol).unwrap();
        let key_type = KeyType::from_i32(request.key_type).unwrap();

        info!(
            "GroupRequest name={:?} device_ids={:?} threshold={}",
            &name,
            device_ids.iter().map(hex::encode).collect::<Vec<String>>(),
            threshold
        );

        let mut state = self.state.lock().await;
        if let Some(task_id) =
            state.add_group_task(&name, &device_ids, threshold, protocol, key_type)
        {
            let task = state.get_task(&task_id).unwrap();
            Ok(Response::new(format_task(&task_id, task, None, None)))
        } else {
            Err(Status::failed_precondition("Request failed"))
        }
    }

    async fn get_devices(
        &self,
        _request: Request<msg::DevicesRequest>,
    ) -> Result<Response<msg::Devices>, Status> {
        debug!("DevicesRequest");

        let resp = msg::Devices {
            devices: self
                .state
                .lock()
                .await
                .get_devices()
                .values()
                .map(|device| device.as_ref().into())
                .collect(),
        };
        Ok(Response::new(resp))
    }

    async fn log(&self, request: Request<msg::LogRequest>) -> Result<Response<msg::Resp>, Status> {
        let device_id = request
            .peer_certs()
            .and_then(|certs| certs.get(0).map(cert_to_id));

        let device_str = device_id
            .as_ref()
            .map(hex::encode)
            .unwrap_or_else(|| "unknown".to_string());
        let message = request.into_inner().message;
        info!("LogRequest device_id={} message={}", device_str, message);

        if device_id.is_some() {
            self.state
                .lock()
                .await
                .device_activated(device_id.as_ref().unwrap());
        }

        Ok(Response::new(msg::Resp {
            message: "OK".into(),
        }))
    }

    async fn decide_task(
        &self,
        request: Request<msg::TaskDecision>,
    ) -> Result<Response<msg::Resp>, Status> {
        if request.peer_certs().is_none() {
            return Err(Status::unauthenticated("Authentication required"));
        }
        let device_id = request
            .peer_certs()
            .and_then(|certs| certs.get(0).map(cert_to_id))
            .unwrap();

        let request = request.into_inner();
        let task_id = Uuid::from_slice(&request.task).unwrap();
        let accept = request.accept;

        info!(
            "TaskDecision task_id={} device_id={} accept={}",
            hex::encode(task_id),
            hex::encode(&device_id),
            accept
        );

        let state = self.state.clone();
        tokio::task::spawn(async move {
            let mut state = state.lock().await;
            state.device_activated(&device_id);
            state.decide_task(&task_id, &device_id, accept);
        });

        Ok(Response::new(msg::Resp {
            message: "OK".into(),
        }))
    }

    async fn acknowledge_task(
        &self,
        request: Request<msg::TaskAcknowledgement>,
    ) -> Result<Response<msg::Resp>, Status> {
        if request.peer_certs().is_none() {
            return Err(Status::unauthenticated("Authentication required"));
        }
        let device_id = request
            .peer_certs()
            .and_then(|certs| certs.get(0).map(cert_to_id))
            .unwrap();

        let task_id = request.into_inner().task_id;

        info!(
            "TaskAcknowledgement task_id={} device_id={}",
            hex::encode(&task_id),
            hex::encode(&device_id)
        );

        let mut state = self.state.lock().await;
        state.device_activated(&device_id);
        state.acknowledge_task(&Uuid::from_slice(&task_id).unwrap(), &device_id);

        Ok(Response::new(msg::Resp {
            message: "OK".into(),
        }))
    }

    async fn subscribe_updates(
        &self,
        request: Request<msg::SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeUpdatesStream>, Status> {
        if request.peer_certs().is_none() {
            return Err(Status::unauthenticated("Authentication required"));
        }
        let device_id = request
            .peer_certs()
            .and_then(|certs| certs.get(0).map(cert_to_id))
            .unwrap();

        let (tx, rx) = mpsc::channel(8);

        self.state.lock().await.add_subscriber(device_id, tx);

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }
}

pub fn format_task(
    task_id: &Uuid,
    task: &dyn Task,
    device_id: Option<&[u8]>,
    request: Option<&[u8]>,
) -> msg::Task {
    let task_status = task.get_status();

    let (task_status, round, data) = match task_status {
        TaskStatus::Created => (msg::task::TaskState::Created, 0, task.get_work(device_id)),
        TaskStatus::Running(round) => (
            msg::task::TaskState::Running,
            round,
            task.get_work(device_id),
        ),
        TaskStatus::Finished => (
            msg::task::TaskState::Finished,
            u16::MAX,
            Some(task.get_result().unwrap().as_bytes().to_vec()),
        ),
        TaskStatus::Failed(data) => (
            msg::task::TaskState::Failed,
            u16::MAX,
            Some(data.as_bytes().to_vec()),
        ),
    };

    let (accept, reject) = task.get_decisions();

    msg::Task {
        id: task_id.as_bytes().to_vec(),
        r#type: task.get_type() as i32,
        state: task_status as i32,
        round: round.into(),
        accept,
        reject,
        data,
        request: request.map(Vec::from),
        attempt: task.get_attempts(),
    }
}

pub fn issue_certificate(device_name: &str, csr: &[u8]) -> Result<Vec<u8>, String> {
    let csr = X509Req::from_der(csr).unwrap();
    let public_key = csr.public_key().unwrap();
    if !csr.verify(&public_key).unwrap() {
        return Err(String::from("CSR does not contain a valid signature."));
    }

    let public_key = public_key.public_key_to_der().unwrap();

    let mut cert_builder = X509Builder::new().unwrap();

    cert_builder.set_version(2).unwrap();

    let sn: [u8; 16] = rand::thread_rng().gen(); // TODO consider stateful approach
    let sn = BigNum::from_slice(&sn).unwrap();
    cert_builder
        .set_serial_number(&Asn1Integer::from_bn(&sn).unwrap())
        .unwrap();

    cert_builder
        .set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();

    cert_builder
        .set_not_after(&Asn1Time::days_from_now(365 * 4 + 1).unwrap())
        .unwrap();

    cert_builder.set_issuer_name(CA_CERT.issuer_name()).unwrap();

    cert_builder
        .set_pubkey(&PKey::public_key_from_der(&public_key).unwrap())
        .unwrap();

    let mut subject = X509NameBuilder::new().unwrap();
    subject.append_entry_by_text("CN", device_name).unwrap();
    cert_builder.set_subject_name(&subject.build()).unwrap();

    let context = cert_builder.x509v3_context(Some(&CA_CERT), None);

    let basic_constraints = X509Extension::new(
        None,
        Some(&context),
        "basicConstraints",
        "critical, CA:FALSE",
    )
    .unwrap();

    let subject_key_identifier =
        X509Extension::new(None, Some(&context), "subjectKeyIdentifier", "hash").unwrap();

    let authority_key_identifier = X509Extension::new(
        None,
        Some(&context),
        "authorityKeyIdentifier",
        "keyid,issuer",
    )
    .unwrap();
    let key_usage = X509Extension::new(
        None,
        Some(&context),
        "keyUsage",
        "critical, nonRepudiation, digitalSignature, keyEncipherment, keyAgreement",
    )
    .unwrap();
    let extended_key_usage =
        X509Extension::new(None, Some(&context), "extendedKeyUsage", "clientAuth").unwrap();

    cert_builder.append_extension(key_usage).unwrap();
    cert_builder.append_extension(extended_key_usage).unwrap();
    cert_builder.append_extension(basic_constraints).unwrap();
    cert_builder
        .append_extension(subject_key_identifier)
        .unwrap();
    cert_builder
        .append_extension(authority_key_identifier)
        .unwrap();

    cert_builder.sign(&CA_KEY, MessageDigest::sha256()).unwrap();

    Ok(cert_builder.build().to_der().unwrap())
}

pub fn cert_to_id(cert: impl AsRef<[u8]>) -> Vec<u8> {
    use sha2::Digest;
    sha2::Sha256::digest(cert).to_vec()
}

pub async fn run_grpc(state: Arc<Mutex<State>>, addr: &str, port: u16) -> Result<(), String> {
    let addr = format!("{}:{}", addr, port)
        .parse()
        .map_err(|_| String::from("Unable to parse server address"))?;
    let node = MeeSignService::new(state);

    let ca_cert = CA_CERT
        .to_pem()
        .map_err(|_| "Unable to load CA certificate".to_string())?;
    let cert = tokio::fs::read("keys/meesign-server-cert.pem")
        .await
        .map_err(|_| "Unable to load server certificate".to_string())?;
    let key = tokio::fs::read("keys/meesign-server-key.pem")
        .await
        .map_err(|_| "Unable to load server key".to_string())?;

    Server::builder()
        .tls_config(
            ServerTlsConfig::new()
                .identity(Identity::from_pem(&cert, &key))
                .client_ca_root(Certificate::from_pem(ca_cert))
                .client_auth_optional(true),
        )
        .map_err(|_| "Unable to setup TLS for gRPC server")?
        .add_service(MeeSignServer::new(node))
        .serve(addr)
        .await
        .map_err(|_| String::from("Unable to run gRPC server"))?;

    Ok(())
}
