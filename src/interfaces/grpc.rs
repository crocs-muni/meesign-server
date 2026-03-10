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
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
use tonic::codegen::Arc;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::persistence::DeviceKind;
use crate::proto::{KeyType, MeeSign, MeeSignServer, ProtocolType};
use crate::state::State;
use crate::{proto as msg, utils, CA_CERT, CA_KEY};

use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use meesign_crypto::proto::{ClientMessage, Message as _};
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use tower_http::cors::CorsLayer;

// JWT secret key for token-based authentication (web clients).
// In production, load from a file or environment variable.
lazy_static::lazy_static! {
    static ref JWT_SECRET: Vec<u8> = {
        let path = "keys/jwt-secret.key";
        match std::fs::read(path) {
            Ok(key) => key,
            Err(_) => {
                // Auto-generate a secret if the file doesn't exist
                let secret: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();
                let _ = std::fs::write(path, &secret);
                warn!("JWT secret not found at {}, generated a new one", path);
                secret
            }
        }
    };
}

#[derive(Debug, Serialize, Deserialize)]
struct JwtClaims {
    /// device_id as hex string
    sub: String,
    /// Issued at (unix timestamp)
    iat: u64,
}

fn generate_jwt(device_id: &[u8]) -> Result<String, Status> {
    let claims = JwtClaims {
        sub: hex::encode(device_id),
        iat: crate::get_timestamp(),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(&JWT_SECRET),
    )
    .map_err(|e| Status::internal(format!("Failed to generate token: {}", e)))
}

fn validate_jwt(token: &str) -> Result<Vec<u8>, Status> {
    let mut validation = Validation::default();
    validation.required_spec_claims.clear();
    validation.validate_exp = false;

    let token_data = decode::<JwtClaims>(
        token,
        &DecodingKey::from_secret(&JWT_SECRET),
        &validation,
    )
    .map_err(|_| Status::unauthenticated("Invalid authentication token"))?;

    hex::decode(&token_data.claims.sub)
        .map_err(|_| Status::unauthenticated("Invalid device ID in token"))
}

/// Extract device_id from a request, trying mTLS peer certs first,
/// then falling back to JWT token in the "authorization" metadata header.
fn extract_device_id<T>(request: &Request<T>) -> Option<Vec<u8>> {
    // Path 1: mTLS peer certificate (native clients)
    if let Some(certs) = request.peer_certs() {
        if let Some(cert) = certs.first() {
            return Some(cert_to_id(cert));
        }
    }

    // Path 2: JWT token in metadata (web clients)
    if let Some(auth_header) = request.metadata().get("authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            let token = auth_str.strip_prefix("Bearer ").unwrap_or(auth_str);
            if let Ok(device_id) = validate_jwt(token) {
                return Some(device_id);
            }
        }
    }

    None
}

pub struct MeeSignService {
    state: Arc<State>,
}

impl MeeSignService {
    pub fn new(state: Arc<State>) -> Self {
        MeeSignService { state }
    }

    /// Check client authentication using either mTLS peer certs or JWT token.
    /// Returns Ok(()) if auth passes or is not required.
    fn check_client_auth<T>(
        &self,
        request: &Request<T>,
        required: bool,
    ) -> Result<(), Status> {
        if let Some(device_id) = extract_device_id(request) {
            if !self.state.device_exists(&device_id) {
                return Err(Status::unauthenticated("Unknown device"));
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
        self.check_client_auth(&request, false)?;

        debug!("ServerInfoRequest");
        Ok(Response::new(msg::ServerInfo {
            version: crate::VERSION.unwrap_or("unknown").to_string(),
        }))
    }

    async fn register(
        &self,
        request: Request<msg::RegistrationRequest>,
    ) -> Result<Response<msg::RegistrationResponse>, Status> {
        self.check_client_auth(&request, false)?;

        let request = request.into_inner();
        let name = request.name;
        let csr = request.csr;
        //let kind = DeviceKind::try_from(request.kind).unwrap();
        let kind = DeviceKind::User; // TODO
        info!("RegistrationRequest name={:?}", name);

        if let Ok(certificate) = issue_certificate(&name, &csr) {
            let identifier = cert_to_id(&certificate);
            match self
                .state
                .add_device(&identifier, &name, &kind, &certificate)
                .await
            {
                Ok(_) => {
                    let auth_token = generate_jwt(&identifier).ok();
                    Ok(Response::new(msg::RegistrationResponse {
                        device_id: identifier,
                        certificate,
                        auth_token,
                    }))
                }
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
        self.check_client_auth(&request, false)?;

        let request = request.into_inner();
        let group_id = request.group_id;
        let name = request.name;
        let data = request.data;
        info!("SignRequest group_id={}", utils::hextrunc(&group_id));

        let task = self.state.add_sign_task(&group_id, &name, &data).await?;
        Ok(Response::new(task))
    }

    async fn decrypt(
        &self,
        request: Request<msg::DecryptRequest>,
    ) -> Result<Response<msg::Task>, Status> {
        self.check_client_auth(&request, false)?;

        let request = request.into_inner();
        let group_id = request.group_id;
        let name = request.name;
        let data = request.data;
        let data_type = request.data_type;
        info!("DecryptRequest group_id={}", utils::hextrunc(&group_id));

        let task = self
            .state
            .add_decrypt_task(&group_id, &name, &data, &data_type)
            .await?;
        Ok(Response::new(task))
    }

    async fn get_task(
        &self,
        request: Request<msg::TaskRequest>,
    ) -> Result<Response<msg::Task>, Status> {
        self.check_client_auth(&request, false)?;

        let request = request.into_inner();
        let task_id = Uuid::from_slice(&request.task_id).unwrap();
        let device_id = request.device_id.as_deref();
        debug!(
            "TaskRequest task_id={} device_id={}",
            utils::hextrunc(task_id.as_bytes()),
            utils::hextrunc(device_id.unwrap_or(&[]))
        );

        if let Some(device_id) = device_id {
            self.state.activate_device(device_id);
        }
        let task = self.state.get_formatted_task(&task_id, device_id).await?;
        Ok(Response::new(task))
    }

    async fn update_task(
        &self,
        request: Request<msg::TaskUpdate>,
    ) -> Result<Response<msg::Resp>, Status> {
        self.check_client_auth(&request, true)?;

        let device_id = extract_device_id(&request)
            .expect("device_id must be present after auth check");

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

        let messages = data
            .into_iter()
            .map(|bytes| ClientMessage::decode(bytes.as_slice()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Status::invalid_argument("Invalid ClientMessage data."))?;

        self.state.activate_device(&device_id);
        let result = self
            .state
            .update_task(&task_id, &device_id, messages, attempt)
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
        self.check_client_auth(&request, false)?;

        let request = request.into_inner();
        let device_id = request.device_id;
        let device_str = device_id
            .as_ref()
            .map(utils::hextrunc)
            .unwrap_or_else(|| "unknown".to_string());
        debug!("TasksRequest device_id={}", device_str);

        let tasks = if let Some(device_id) = &device_id {
            self.state.activate_device(device_id);
            self.state
                .get_formatted_active_device_tasks(device_id)
                .await?
        } else {
            self.state.get_formatted_tasks().await?
        };

        Ok(Response::new(msg::Tasks { tasks }))
    }

    async fn get_groups(
        &self,
        request: Request<msg::GroupsRequest>,
    ) -> Result<Response<msg::Groups>, Status> {
        self.check_client_auth(&request, false)?;

        let request = request.into_inner();
        let device_id = request.device_id;
        let device_str = device_id
            .as_ref()
            .map(utils::hextrunc)
            .unwrap_or_else(|| "unknown".to_string());
        debug!("GroupsRequest device_id={}", device_str);

        let groups = if let Some(device_id) = device_id {
            self.state.activate_device(&device_id);
            self.state
                .get_device_groups(&device_id)
                .await?
                .into_iter()
                .map(msg::Group::from_model)
                .collect()
        } else {
            self.state
                .get_groups()
                .await?
                .into_iter()
                .map(msg::Group::from_model)
                .collect()
        };

        Ok(Response::new(msg::Groups { groups }))
    }

    async fn group(
        &self,
        request: Request<msg::GroupRequest>,
    ) -> Result<Response<msg::Task>, Status> {
        self.check_client_auth(&request, false)?;

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
        match self
            .state
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
            Ok(task) => Ok(Response::new(task)),
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
        self.check_client_auth(&request, false)?;

        debug!("DevicesRequest");

        let resp = msg::Devices {
            devices: self
                .state
                .get_devices()
                .into_iter()
                .map(|(device, last_active)| msg::Device {
                    identifier: device.id,
                    name: device.name,
                    kind: Into::<msg::DeviceKind>::into(device.kind).into(),
                    certificate: device.certificate,
                    last_active,
                })
                .collect(),
        };
        Ok(Response::new(resp))
    }

    async fn log(&self, request: Request<msg::LogRequest>) -> Result<Response<msg::Resp>, Status> {
        self.check_client_auth(&request, false)?;

        let device_id = extract_device_id(&request);

        let device_str = device_id
            .as_ref()
            .map(utils::hextrunc)
            .unwrap_or_else(|| "unknown".to_string());
        let message = request.into_inner().message.replace('\n', "\\n");
        debug!("LogRequest device_id={} message={}", device_str, message);

        if device_id.is_some() {
            self.state.activate_device(device_id.as_ref().unwrap());
        }

        Ok(Response::new(msg::Resp {
            message: "OK".into(),
        }))
    }

    async fn decide_task(
        &self,
        request: Request<msg::TaskDecision>,
    ) -> Result<Response<msg::Resp>, Status> {
        self.check_client_auth(&request, true)?;

        let device_id = extract_device_id(&request)
            .expect("device_id must be present after auth check");

        let request = request.into_inner();
        let task_id = Uuid::from_slice(&request.task).unwrap();
        let accept = request.accept;

        info!(
            "TaskDecision task_id={} device_id={} accept={}",
            utils::hextrunc(task_id.as_bytes()),
            utils::hextrunc(&device_id),
            accept
        );

        self.state.activate_device(&device_id);
        if let Err(err) = self.state.decide_task(&task_id, &device_id, accept).await {
            error!(
                "Couldn't decide task {} for device {}: {}",
                task_id,
                utils::hextrunc(&device_id),
                err
            );
        }

        Ok(Response::new(msg::Resp {
            message: "OK".into(),
        }))
    }

    async fn acknowledge_task(
        &self,
        request: Request<msg::TaskAcknowledgement>,
    ) -> Result<Response<msg::Resp>, Status> {
        self.check_client_auth(&request, true)?;

        let device_id = extract_device_id(&request)
            .expect("device_id must be present after auth check");

        let task_id = request.into_inner().task_id;

        debug!(
            "TaskAcknowledgement task_id={} device_id={}",
            utils::hextrunc(&task_id),
            utils::hextrunc(&device_id)
        );

        self.state.activate_device(&device_id);

        let task_id = Uuid::from_slice(&task_id).unwrap();
        if let Err(err) = self.state.acknowledge_task(&task_id, &device_id).await {
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
        self.check_client_auth(&request, true)?;

        let device_id = extract_device_id(&request)
            .expect("device_id must be present after auth check");

        let (tx, rx) = mpsc::channel(8);

        self.state.add_subscriber(device_id, tx);

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
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

pub async fn run_grpc(state: Arc<State>, addr: &str, port: u16) -> Result<(), String> {
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

    // Wrap service with gRPC-Web support (auto-detects protocol from content-type)
    let grpc_web_service = tonic_web::enable(MeeSignServer::new(node));

    // Permissive CORS layer to allow browser-based gRPC-Web clients.
    // tonic_web::enable() handles basic CORS but its default allowed headers
    // don't include "authorization" (needed for JWT auth). This outer CorsLayer
    // handles OPTIONS preflight with all necessary headers; for non-preflight
    // responses its insert() replaces tonic-web's headers, avoiding duplicates.
    let cors = CorsLayer::permissive();

    Server::builder()
        .accept_http1(true) // gRPC-Web uses HTTP/1.1
        .tls_config(
            ServerTlsConfig::new()
                .identity(Identity::from_pem(&cert, &key))
                .client_ca_root(Certificate::from_pem(ca_cert))
                .client_auth_optional(true),
        )
        .map_err(|_| "Unable to setup TLS for gRPC server")?
        .layer(cors)
        .add_service(grpc_web_service)
        .serve(addr)
        .await
        .map_err(|_| String::from("Unable to run gRPC server"))?;

    Ok(())
}
