use arc_application::{PortError, PortResult, SecretProvider, WorkerAuthenticator};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use subtle::ConstantTimeEq;
use uuid::Uuid;

pub const LOCAL_WORKER_CREDENTIALS_ENV: &str = "ARC_WORKER_CREDENTIALS_JSON";

pub struct LocalWorkerSecretProvider {
    credentials: HashMap<Uuid, String>,
}

impl LocalWorkerSecretProvider {
    pub fn from_json(json: &str) -> PortResult<Self> {
        let configured: HashMap<String, String> =
            serde_json::from_str(json).map_err(|_| invalid_credential_configuration())?;
        if configured.is_empty() {
            return Err(invalid_credential_configuration());
        }

        let mut credentials = HashMap::with_capacity(configured.len());
        for (worker_id, token) in configured {
            let parsed_worker_id =
                Uuid::parse_str(&worker_id).map_err(|_| invalid_credential_configuration())?;
            if parsed_worker_id.is_nil()
                || parsed_worker_id.to_string() != worker_id
                || !is_valid_local_token(&token)
            {
                return Err(invalid_credential_configuration());
            }
            credentials.insert(parsed_worker_id, token);
        }
        Ok(Self { credentials })
    }
}

#[async_trait]
impl SecretProvider for LocalWorkerSecretProvider {
    async fn resolve_reference(&self, reference: &str) -> PortResult<String> {
        let worker_id = reference
            .strip_prefix("workers/")
            .and_then(|value| value.strip_suffix("/control-token"))
            .and_then(|value| Uuid::parse_str(value).ok())
            .filter(|worker_id| !worker_id.is_nil())
            .ok_or_else(secret_not_found)?;
        self.credentials
            .get(&worker_id)
            .cloned()
            .ok_or_else(secret_not_found)
    }
}

pub struct FileWorkerSecretProvider {
    path: PathBuf,
}

impl FileWorkerSecretProvider {
    pub fn from_file(path: impl AsRef<Path>) -> PortResult<Self> {
        let path = path.as_ref().to_owned();
        let contents = std::fs::read_to_string(&path).map_err(|error| {
            tracing::error!(%error, "failed to read worker credential secret file");
            secret_provider_unavailable()
        })?;
        LocalWorkerSecretProvider::from_json(&contents)?;
        Ok(Self { path })
    }
}

#[async_trait]
impl SecretProvider for FileWorkerSecretProvider {
    async fn resolve_reference(&self, reference: &str) -> PortResult<String> {
        let contents = std::fs::read_to_string(&self.path).map_err(|error| {
            tracing::error!(%error, "failed to reload worker credential secret file");
            secret_provider_unavailable()
        })?;
        LocalWorkerSecretProvider::from_json(&contents)
            .map_err(|_| secret_provider_unavailable())?
            .resolve_reference(reference)
            .await
    }
}

pub struct SecretProviderWorkerAuthenticator<S> {
    secret_provider: S,
}

impl<S> SecretProviderWorkerAuthenticator<S> {
    pub fn new(secret_provider: S) -> Self {
        Self { secret_provider }
    }
}

#[async_trait]
impl<S> WorkerAuthenticator for SecretProviderWorkerAuthenticator<S>
where
    S: SecretProvider,
{
    async fn authenticate(&self, worker_id: Uuid, bearer_token: &str) -> PortResult<()> {
        if worker_id.is_nil() || bearer_token.is_empty() {
            return Err(authentication_failed());
        }

        let reference = format!("workers/{worker_id}/control-token");
        let expected = self
            .secret_provider
            .resolve_reference(&reference)
            .await
            .map_err(|error| {
                if error.code == "secret_not_found" {
                    return authentication_failed();
                }
                tracing::error!(
                    %worker_id,
                    error_code = error.code,
                    "failed to resolve worker control credential"
                );
                PortError {
                    code: "worker_authentication_unavailable",
                    message: "worker credentials could not be verified".to_owned(),
                }
            })?;

        if bool::from(expected.as_bytes().ct_eq(bearer_token.as_bytes())) {
            Ok(())
        } else {
            Err(authentication_failed())
        }
    }
}

fn is_valid_local_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn invalid_credential_configuration() -> PortError {
    PortError {
        code: "invalid_worker_credential_configuration",
        message:
            "worker credential JSON must map canonical worker UUIDs to 64-character hexadecimal tokens"
                .to_owned(),
    }
}

fn secret_not_found() -> PortError {
    PortError {
        code: "secret_not_found",
        message: "worker credential reference was not found".to_owned(),
    }
}

fn secret_provider_unavailable() -> PortError {
    PortError {
        code: "secret_provider_unavailable",
        message: "worker credentials could not be loaded".to_owned(),
    }
}

fn authentication_failed() -> PortError {
    PortError {
        code: "worker_authentication_failed",
        message: "worker identity or credential was rejected".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestSecretProvider {
        values: HashMap<String, String>,
        fail: bool,
    }

    #[async_trait]
    impl SecretProvider for TestSecretProvider {
        async fn resolve_reference(&self, reference: &str) -> PortResult<String> {
            if self.fail {
                return Err(PortError {
                    code: "secret_provider_error",
                    message: "private backend detail".to_owned(),
                });
            }

            self.values
                .get(reference)
                .cloned()
                .ok_or_else(|| PortError {
                    code: "secret_not_found",
                    message: "private lookup detail".to_owned(),
                })
        }
    }

    fn provider_for(worker_id: Uuid, token: &str) -> TestSecretProvider {
        TestSecretProvider {
            values: HashMap::from([(
                format!("workers/{worker_id}/control-token"),
                token.to_owned(),
            )]),
            fail: false,
        }
    }

    #[tokio::test]
    async fn local_provider_resolves_only_configured_canonical_worker_references() {
        let worker_id = Uuid::new_v4();
        let token = "ab".repeat(32);
        let provider =
            LocalWorkerSecretProvider::from_json(&format!("{{\"{worker_id}\":\"{token}\"}}"))
                .expect("local credentials parse");

        assert_eq!(
            provider
                .resolve_reference(&format!("workers/{worker_id}/control-token"))
                .await
                .expect("configured credential resolves"),
            token
        );
        assert_eq!(
            provider
                .resolve_reference(&format!("workers/{}/control-token", Uuid::new_v4()))
                .await
                .unwrap_err()
                .code,
            "secret_not_found"
        );
    }

    #[tokio::test]
    async fn file_provider_observes_rotated_credentials_without_restarting() {
        let worker_id = Uuid::new_v4();
        let path = std::env::temp_dir().join(format!("arc-worker-credentials-{worker_id}.json"));
        let first = "ab".repeat(32);
        let second = "cd".repeat(32);
        std::fs::write(
            &path,
            format!("{{\"{worker_id}\":\"{first}\"}}"),
        )
        .expect("initial credentials are written");
        let provider = FileWorkerSecretProvider::from_file(&path)
            .expect("initial credential file is valid");

        assert_eq!(
            provider
                .resolve_reference(&format!("workers/{worker_id}/control-token"))
                .await
                .expect("initial credential resolves"),
            first
        );
        std::fs::write(
            &path,
            format!("{{\"{worker_id}\":\"{second}\"}}"),
        )
        .expect("rotated credentials are written");
        assert_eq!(
            provider
                .resolve_reference(&format!("workers/{worker_id}/control-token"))
                .await
                .expect("rotated credential resolves"),
            second
        );
        std::fs::remove_file(path).expect("temporary credentials are removed");
    }

    #[test]
    fn local_provider_rejects_weak_or_malformed_credentials() {
        let worker_id = Uuid::new_v4();
        let short_token = serde_json::json!({ (worker_id.to_string()): "too-short" }).to_string();
        let invalid_id = serde_json::json!({ "not-a-uuid": "ab".repeat(32) }).to_string();
        let empty = "{}";

        for invalid in [short_token, invalid_id, empty.to_owned()] {
            assert!(matches!(
                LocalWorkerSecretProvider::from_json(&invalid),
                Err(error) if error.code == "invalid_worker_credential_configuration"
            ));
        }
    }

    #[tokio::test]
    async fn accepts_only_the_secret_for_the_requested_worker() {
        let worker_id = Uuid::new_v4();
        let authenticator =
            SecretProviderWorkerAuthenticator::new(provider_for(worker_id, "worker-secret"));

        assert!(authenticator
            .authenticate(worker_id, "worker-secret")
            .await
            .is_ok());
        assert_eq!(
            authenticator
                .authenticate(Uuid::new_v4(), "worker-secret")
                .await
                .unwrap_err()
                .code,
            "worker_authentication_failed"
        );
        assert_eq!(
            authenticator
                .authenticate(worker_id, "wrong-secret")
                .await
                .unwrap_err()
                .code,
            "worker_authentication_failed"
        );
    }

    #[tokio::test]
    async fn does_not_expose_secret_provider_errors_or_accept_empty_credentials() {
        let worker_id = Uuid::new_v4();
        let unavailable = SecretProviderWorkerAuthenticator::new(TestSecretProvider {
            values: HashMap::new(),
            fail: true,
        });
        let error = unavailable
            .authenticate(worker_id, "worker-secret")
            .await
            .unwrap_err();
        assert_eq!(error.code, "worker_authentication_unavailable");
        assert!(!error.message.contains("private"));

        let authenticator =
            SecretProviderWorkerAuthenticator::new(provider_for(worker_id, "worker-secret"));
        assert_eq!(
            authenticator
                .authenticate(worker_id, "")
                .await
                .unwrap_err()
                .code,
            "worker_authentication_failed"
        );
    }
}
