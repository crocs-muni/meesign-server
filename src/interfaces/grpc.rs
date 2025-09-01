use futures::future;
use log::{debug, error, info, warn};
use openssl::asn1::{Asn1Integer, Asn1Time};
use openssl::bn::BigNum;
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::x509::extension::{
    AuthorityKeyIdentifier, BasicConstraints, ExtendedKeyUsage, KeyUsage, SubjectKeyIdentifier,
};
use openssl::x509::{X509Builder, X509NameBuilder, X509Req};
use rand::Rng;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
use tonic::codegen::Arc;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::persistence::DeviceKind;
use crate::proto::{Group, KeyType, MeeSign, MeeSignServer, ProtocolType};
use crate::state::State;
use crate::tasks::{Task, TaskStatus};
use crate::{proto as msg, utils, CA_CERT, CA_KEY};

use std::pin::Pin;

pub struct MeeSignService {
    state: Arc<Mutex<State>>,
}

impl MeeSignService {
    pub fn new(state: Arc<Mutex<State>>) -> Self {
        MeeSignService { state }
    }

    async fn check_client_auth(
        &self,
        certs: &Option<Arc<Vec<Certificate>>>,
        required: bool,
    ) -> Result<(), Status> {
        if let Some(certs) = certs {
            let device_id = certs.get(0).map(cert_to_id).unwrap_or(vec![]);
            if !self.state.lock().await.device_activated(&device_id).await? {
                return Err(Status::unauthenticated("Unknown device certificate"));
            }
        } else if required {
            return Err(Status::unauthenticated("Authentication required"));
        }
        Ok(())
    }
}

#[tonic::async_trait]
impl MeeSign for MeeSignService {
    type SubscribeUpdatesStream =
        Pin<Box<dyn Stream<Item = Result<msg::Task, Status>> + Send + 'static>>;

    async fn get_server_info(
        &self,
        request: Request<msg::ServerInfoRequest>,
    ) -> Result<Response<msg::ServerInfo>, Status> {
        self.check_client_auth(&request.peer_certs(), false).await?;

        debug!("ServerInfoRequest");
        Ok(Response::new(msg::ServerInfo {
            version: crate::VERSION.unwrap_or("unknown").to_string(),
        }))
    }

    async fn register(
        &self,
        request: Request<msg::RegistrationRequest>,
    ) -> Result<Response<msg::RegistrationResponse>, Status> {
        self.check_client_auth(&request.peer_certs(), false).await?;

        let request = request.into_inner();
        let name = request.name;
        let csr = request.csr;
        //let kind = DeviceKind::try_from(request.kind).unwrap();
        let kind = DeviceKind::User; // TODO
        info!("RegistrationRequest name={:?}", name);

        let state = self.state.lock().await;

        if let Ok(certificate) = issue_certificate(&name, &csr) {
            let identifier = cert_to_id(&certificate);
            match state
                .add_device(&identifier, &name, &kind, &certificate)
                .await
            {
                Ok(_) => Ok(Response::new(msg::RegistrationResponse {
                    device_id: identifier,
                    certificate,
                })),
                Err(_) => Err(Status::failed_precondition(
                    "Request failed: device was not added",
                )),
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
        self.check_client_auth(&request.peer_certs(), false).await?;

        let request = request.into_inner();
        let group_id = request.group_id;
        let name = request.name;
        let data = request.data;
        info!("SignRequest group_id={}", utils::hextrunc(&group_id));

        let mut state = self.state.lock().await;
        let task_id = state.add_sign_task(&group_id, &name, &data).await?;
        let task = state.get_task(&task_id).await?;
        Ok(Response::new(
            format_task(&task_id, &*task, None, None).await,
        ))
    }

    async fn decrypt(
        &self,
        request: Request<msg::DecryptRequest>,
    ) -> Result<Response<msg::Task>, Status> {
        self.check_client_auth(&request.peer_certs(), false).await?;

        let request = request.into_inner();
        let group_id = request.group_id;
        let name = request.name;
        let data = request.data;
        let data_type = request.data_type;
        info!("DecryptRequest group_id={}", utils::hextrunc(&group_id));

        let mut state = self.state.lock().await;
        let task_id = state
            .add_decrypt_task(&group_id, &name, &data, &data_type)
            .await?;
        let task = state.get_task(&task_id).await?;
        Ok(Response::new(
            format_task(&task_id, &*task, None, None).await,
        ))
    }

    async fn get_task(
        &self,
        request: Request<msg::TaskRequest>,
    ) -> Result<Response<msg::Task>, Status> {
        self.check_client_auth(&request.peer_certs(), false).await?;

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
            utils::hextrunc(task_id.as_bytes()),
            utils::hextrunc(device_id.unwrap_or(&[]))
        );

        let state = self.state.lock().await;
        if device_id.is_some() {
            state.activate_device(device_id.unwrap()).await?;
        }
        let task = state.get_task(&task_id).await?;
        let request = Some(task.get_request());

        let resp = format_task(&task_id, &*task, device_id, request).await;
        Ok(Response::new(resp))
    }

    async fn update_task(
        &self,
        request: Request<msg::TaskUpdate>,
    ) -> Result<Response<msg::Resp>, Status> {
        self.check_client_auth(&request.peer_certs(), true).await?;

        let device_id = request
            .peer_certs()
            .and_then(|certs| certs.get(0).map(cert_to_id))
            .unwrap();

        let request = request.into_inner();
        let task_id = Uuid::from_slice(&request.task).unwrap();
        let data = request.data;
        let attempt = request.attempt;
        if data.is_empty() {
            warn!(
                "TaskUpdate task_id={} device_id={} attempt={} data empty",
                utils::hextrunc(task_id.as_bytes()),
                utils::hextrunc(&device_id),
                attempt
            );
            return Err(Status::invalid_argument("Data must not be empty"));
        }
        debug!(
            "TaskUpdate task_id={} device_id={} attempt={}",
            utils::hextrunc(task_id.as_bytes()),
            utils::hextrunc(&device_id),
            attempt
        );

        let mut state = self.state.lock().await;
        state.activate_device(&device_id).await?;
        let result = state
            .update_task(&task_id, &device_id, &data, attempt)
            .await;

        match result {
            Ok(_) => Ok(Response::new(msg::Resp {
                message: "OK".into(),
            })),
            Err(err) => {
                error!(
                    "Couldn't update task with id {} for device {}",
                    task_id,
                    utils::hextrunc(&device_id)
                );
                return Err(err.into());
            }
        }
    }

    async fn get_tasks(
        &self,
        request: Request<msg::TasksRequest>,
    ) -> Result<Response<msg::Tasks>, Status> {
        self.check_client_auth(&request.peer_certs(), false).await?;

        let request = request.into_inner();
        let device_id = request.device_id;
        let device_str = device_id
            .as_ref()
            .map(utils::hextrunc)
            .unwrap_or_else(|| "unknown".to_string());
        debug!("TasksRequest device_id={}", device_str);

        let state = self.state.lock().await;
        let tasks = if let Some(device_id) = device_id {
            state.activate_device(&device_id).await?;
            future::join_all(
                state
                    .get_active_device_tasks(&device_id)
                    .await?
                    .iter()
                    .map(|task| format_task(task.get_id(), task.as_ref(), Some(&device_id), None)),
            )
            .await
        } else {
            future::join_all(
                state
                    .get_tasks()
                    .await?
                    .iter()
                    .map(|task| format_task(task.get_id(), task.as_ref(), None, None)),
            )
            .await
        };

        Ok(Response::new(msg::Tasks { tasks }))
    }

    async fn get_groups(
        &self,
        request: Request<msg::GroupsRequest>,
    ) -> Result<Response<msg::Groups>, Status> {
        self.check_client_auth(&request.peer_certs(), false).await?;

        let request = request.into_inner();
        let device_id = request.device_id;
        let device_str = device_id
            .as_ref()
            .map(utils::hextrunc)
            .unwrap_or_else(|| "unknown".to_string());
        debug!("GroupsRequest device_id={}", device_str);

        let state = self.state.lock().await;
        // TODO: refactor, consider storing device IDS in the group model directly
        let groups = if let Some(device_id) = device_id {
            state.activate_device(&device_id).await?;
            state
                .get_device_groups(&device_id)
                .await?
                .into_iter()
                .map(Group::from_model)
                .collect()
        } else {
            state
                .get_groups()
                .await?
                .into_iter()
                .map(Group::from_model)
                .collect()
        };

        Ok(Response::new(msg::Groups { groups }))
    }

    async fn group(
        &self,
        request: Request<msg::GroupRequest>,
    ) -> Result<Response<msg::Task>, Status> {
        self.check_client_auth(&request.peer_certs(), false).await?;

        let request = request.into_inner();
        let name = request.name;
        let device_ids = request.device_ids;
        let threshold = request.threshold;
        let protocol = ProtocolType::try_from(request.protocol).unwrap();
        let key_type = KeyType::try_from(request.key_type).unwrap();
        let note = None; // TODO

        info!(
            "GroupRequest name={:?} device_ids={:?} threshold={}",
            &name,
            device_ids
                .iter()
                .map(utils::hextrunc)
                .collect::<Vec<String>>(),
            threshold
        );

        let device_id_references: Vec<&[u8]> = device_ids
            .iter()
            .map(|device_id| device_id.as_ref())
            .collect();
        let mut state = self.state.lock().await;
        match state
            .add_group_task(
                &name,
                &device_id_references,
                threshold,
                protocol.into(),
                key_type.into(),
                note,
            )
            .await
        {
            Ok(task_id) => {
                state.send_updates(&task_id).await?;
                // TODO: use group task
                let task = state.get_task(&task_id).await?;
                Ok(Response::new(
                    format_task(&task_id, &*task, None, None).await,
                ))
            }
            Err(err) => {
                error!("{}", err);
                Err(Status::failed_precondition("Request failed"))
            }
        }
    }

    async fn get_devices(
        &self,
        request: Request<msg::DevicesRequest>,
    ) -> Result<Response<msg::Devices>, Status> {
        self.check_client_auth(&request.peer_certs(), false).await?;

        debug!("DevicesRequest");

        let resp = msg::Devices {
            devices: self
                .state
                .lock()
                .await
                .get_devices()
                .await?
                .into_iter()
                .map(|device| device.into())
                .collect(),
        };
        Ok(Response::new(resp))
    }

    async fn log(&self, request: Request<msg::LogRequest>) -> Result<Response<msg::Resp>, Status> {
        self.check_client_auth(&request.peer_certs(), false).await?;

        let device_id = request
            .peer_certs()
            .and_then(|certs| certs.get(0).map(cert_to_id));

        let device_str = device_id
            .as_ref()
            .map(utils::hextrunc)
            .unwrap_or_else(|| "unknown".to_string());
        let message = request.into_inner().message.replace('\n', "\\n");
        debug!("LogRequest device_id={} message={}", device_str, message);

        if device_id.is_some() {
            self.state
                .lock()
                .await
                .activate_device(device_id.as_ref().unwrap())
                .await?;
        }

        Ok(Response::new(msg::Resp {
            message: "OK".into(),
        }))
    }

    async fn decide_task(
        &self,
        request: Request<msg::TaskDecision>,
    ) -> Result<Response<msg::Resp>, Status> {
        self.check_client_auth(&request.peer_certs(), true).await?;

        let device_id = request
            .peer_certs()
            .and_then(|certs| certs.get(0).map(cert_to_id))
            .unwrap();

        let request = request.into_inner();
        let task_id = Uuid::from_slice(&request.task).unwrap();
        let accept = request.accept;

        info!(
            "TaskDecision task_id={} device_id={} accept={}",
            utils::hextrunc(task_id.as_bytes()),
            utils::hextrunc(&device_id),
            accept
        );

        let state = self.state.clone();
        tokio::task::spawn(async move {
            let mut state = state.lock().await;
            if let Err(err) = state.activate_device(&device_id).await {
                error!(
                    "Couldn't activate device with id {}: {}",
                    utils::hextrunc(&device_id),
                    err
                );
            }
            if let Err(err) = state.decide_task(&task_id, &device_id, accept).await {
                error!(
                    "Couldn't decide task {} for device {}: {}",
                    task_id,
                    utils::hextrunc(&device_id),
                    err
                );
            }
        });

        Ok(Response::new(msg::Resp {
            message: "OK".into(),
        }))
    }

    async fn acknowledge_task(
        &self,
        request: Request<msg::TaskAcknowledgement>,
    ) -> Result<Response<msg::Resp>, Status> {
        self.check_client_auth(&request.peer_certs(), true).await?;

        let device_id = request
            .peer_certs()
            .and_then(|certs| certs.get(0).map(cert_to_id))
            .unwrap();

        let task_id = request.into_inner().task_id;

        debug!(
            "TaskAcknowledgement task_id={} device_id={}",
            utils::hextrunc(&task_id),
            utils::hextrunc(&device_id)
        );

        let mut state = self.state.lock().await;
        state.activate_device(&device_id).await?;

        let task_id = Uuid::from_slice(&task_id).unwrap();
        if let Err(err) = state.acknowledge_task(&task_id, &device_id).await {
            error!(
                "Couldn't acknowledge task {} for device {}: {}",
                task_id,
                utils::hextrunc(&device_id),
                err
            );
        }

        Ok(Response::new(msg::Resp {
            message: "OK".into(),
        }))
    }

    async fn subscribe_updates(
        &self,
        request: Request<msg::SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeUpdatesStream>, Status> {
        self.check_client_auth(&request.peer_certs(), true).await?;

        let device_id = request
            .peer_certs()
            .and_then(|certs| certs.get(0).map(cert_to_id))
            .unwrap();

        let (tx, rx) = mpsc::channel(8);

        self.state.lock().await.add_subscriber(device_id, tx);

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }
}

pub async fn format_task(
    task_id: &Uuid,
    task: &dyn Task,
    device_id: Option<&[u8]>,
    request: Option<&[u8]>,
) -> msg::Task {
    let task_status = task.get_status();

    let (task_status, round, data) = match task_status {
        TaskStatus::Created => (
            msg::task::TaskState::Created,
            0,
            task.get_work(device_id).await,
        ),
        TaskStatus::Running(round) => (
            msg::task::TaskState::Running,
            round,
            task.get_work(device_id).await,
        ),
        TaskStatus::Finished => (
            msg::task::TaskState::Finished,
            u16::MAX,
            vec![task.get_result().unwrap().as_bytes().to_vec()],
        ),
        TaskStatus::Failed(data) => (
            msg::task::TaskState::Failed,
            u16::MAX,
            vec![data.as_bytes().to_vec()],
        ),
    };

    let (accept, reject) = task.get_decisions().await;

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

    let basic_constraints = BasicConstraints::new().critical().build().unwrap();

    let subject_key_identifier = SubjectKeyIdentifier::new().build(&context).unwrap();

    let authority_key_identifier = AuthorityKeyIdentifier::new()
        .keyid(false)
        .issuer(false)
        .build(&context)
        .unwrap();

    let key_usage = KeyUsage::new()
        .critical()
        .non_repudiation()
        .digital_signature()
        .key_encipherment()
        .key_agreement()
        .build()
        .unwrap();

    let extended_key_usage = ExtendedKeyUsage::new().client_auth().build().unwrap();

    cert_builder.append_extension(key_usage).unwrap();
    cert_builder.append_extension(extended_key_usage).unwrap();
    cert_builder.append_extension(basic_constraints).unwrap();
    cert_builder
        .append_extension(subject_key_identifier)
        .unwrap();
    cert_builder
        .append_extension(authority_key_identifier)
        .unwrap();

    let pub_key_ext = csr.extensions().unwrap().pop().unwrap();
    cert_builder.append_extension(pub_key_ext).unwrap();

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
